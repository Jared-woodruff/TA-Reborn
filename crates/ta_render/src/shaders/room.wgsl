// The room the intro plays in (room.rs): a bedroom at night, lit by the CRT on the desk, the
// moon through the blinds and a lava lamp.
// Prepended: fullscreen.wgsl.
//
// Light comes from:
// - the bakes (vertex colours: the moonlight's bounce and the occlusion, each quarter of the
//   tube's light bounced round the room at unit white, the lava lamp's light);
// - the moon, directly: an orthographic shadow map along its light, filtered as wide as the
//   moon's disc makes the shadow soft (PCSS), and a tree swaying outside;
// - the tube, directly: 3×3 patches each the colour of its part of the picture, shadowed by a
//   map from the glass's middle, softened as wide as the glass makes them; its reflection in
//   glossy things is the picture itself, blurred as they are rough;
// - the lava lamp: its glass shows its wax rising and falling (traced), and it shines in what
//   is glossy;
// - the glowing parts: LEDs, the turbo display.
// Over that: dust motes and the coffee's steam, and the light in the air (fs_fog).

struct Room {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    moon_view_proj: mat4x4<f32>,
    tube_view_proj: mat4x4<f32>,
    // xyz = the eye, w = time (s)
    eye: vec4<f32>,
    // xyz = the glass's middle, w = power (0 off .. 1 on: the picture opens and closes with it)
    screen_center: vec4<f32>,
    // Half the glass's width and height as vectors; right.w = how far it bulges, up.w = 1 while
    // a picture is on
    screen_right: vec4<f32>,
    screen_up: vec4<f32>,
    // The way it faces; w = how near white its white shows
    screen_normal: vec4<f32>,
    // The picture's 3×3 regional colours (linear), row by row from the top left
    glow: array<vec4<f32>, 9>,
    // Each quarter's light (radiance), top left, top right, bottom left, bottom right
    quarters: array<vec4<f32>, 4>,
    // x = the tube's radiance for white as it lights the room, y = the fill from the seat
    screen_light: vec4<f32>,
    // The room look's exposure, and its tone curve's toe and shoulder (post.wgsl room_look)
    look: vec4<f32>,
    // Where the person at the desk sits
    seat: vec4<f32>,
    // Towards the moon; w = its irradiance now
    moon_dir: vec4<f32>,
    // rgb = the moonlight's colour; w = the tube's degaussing (1 .. 0)
    moon_color: vec4<f32>,
    // The moon's map: x = its depth (m), y = tan(the moon's radius), z = a texel (m), w = texels
    moon_map: vec4<f32>,
    // The tube's map: x = near, y = far, z = tan(half its field of view), w = texels
    tube_map: vec4<f32>,
    // xyz = the lava lamp's light, w = how bright (0..1)
    lava: vec4<f32>,
    lava_color: vec4<f32>,
    // The window's pane: its middle and half its extent
    window: vec4<f32>,
    window_half: vec4<f32>,
    // The bakes' scales (bounce, tube, lava); w = the dark's own faint light
    scales: vec4<f32>,
    // How bright each glowing part is (room.rs GLOW_NAMES), four to a vector
    glows: array<vec4<f32>, 6>,
    // xyz = where the coffee steams, w = the first steam puff's instance
    steam: vec4<f32>,
    // xy = 1 / the picture's size in pixels, z = near, w = far
    pixel: vec4<f32>,
    // x = the season (0 none, 1 Christmas, 2 New Year)
    season: vec4<f32>,
    // The season's lights: xyz = place, w = reach (0: not there); rgb = radiance at a metre
    festive: array<vec4<f32>, 3>,
    festive_color: array<vec4<f32>, 3>,
    // Fireworks: x = along the sky (z), y = up it, z = age (0..1; below 0: none), w = hue
    bursts: array<vec4<f32>, 4>,
    // The fireworks' light through the window (rgb)
    flash: vec4<f32>,
}

struct Material {
    // Linear colour, alpha
    base: vec4<f32>,
    // Roughness, metallic, role, index (a picture's layer, a glow's slot)
    params: vec4<f32>,
    // rgb = its own glow; for a picture that glows (a display): x = how much of the room's light
    // it shows, y = how bright it glows, w = its glow's slot plus one
    emissive: vec4<f32>,
}

@group(0) @binding(0) var<uniform> room: Room;
@group(0) @binding(1) var<uniform> materials: array<Material, 192>;
@group(0) @binding(2) var layers: texture_2d_array<f32>;
@group(0) @binding(3) var layer_sampler: sampler;
@group(0) @binding(4) var picture: texture_2d<f32>;
@group(0) @binding(5) var picture_sampler: sampler;
@group(0) @binding(6) var moon_map: texture_depth_2d;
@group(0) @binding(7) var tube_map: texture_depth_2d;
@group(0) @binding(8) var shadow_sampler: sampler_comparison;
@group(0) @binding(9) var depth_sampler: sampler;

// The fog's passes: the scene's depth, and the light marched in the air.
@group(1) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(1) @binding(1) var fog_light: texture_2d<f32>;
@group(1) @binding(2) var fog_sampler: sampler;

const PI: f32 = 3.14159265;

// Materials' roles (room.rs Role).
const PLAIN: u32 = 0u;
const WOOD: u32 = 1u;
const LAMINATE: u32 = 2u;
const PAINT: u32 = 3u;
const CARPET: u32 = 4u;
const PLASTIC: u32 = 5u;
const PICTURE: u32 = 6u;
const GLASS: u32 = 7u;
const LAVA: u32 = 8u;
const GLOW: u32 = 9u;
const SKY: u32 = 10u;
const MOON: u32 = 11u;
const FABRIC: u32 = 12u;
const PAPER: u32 = 13u;
const BACKDROP: u32 = 14u;
const COFFEE: u32 = 15u;

// ---- Noise -----------------------------------------------------------------------------------

fn hash3(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

fn hash_u(n: u32) -> u32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    return (x >> 22u) ^ x;
}

fn rand_u(n: u32) -> f32 {
    return f32(hash_u(n) & 0xffffffu) / 16777216.0;
}

// Value noise, 0..1.
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += amp * noise3(q);
        q = q * 2.03 + vec3<f32>(1.7, 9.2, 5.1);
        amp *= 0.5;
    }
    return sum;
}

// Interleaved gradient noise: a different fraction for each pixel, evenly spread.
fn ign(px: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(px, vec2<f32>(0.06711056, 0.00583715))));
}

// A turn for a soft shadow's samples at a point of a surface: fixed to the surface (keyed on the
// screen's pixels, the noise would crawl over it as the view drifts).
fn surface_turn(p: vec3<f32>) -> f32 {
    return hash3(floor(p * 1400.0)) * 6.2831853;
}

// The `i`th of `n` points spread evenly over a unit disc (Vogel's), turned by `turn`.
fn disc(i: u32, n: u32, turn: f32) -> vec2<f32> {
    let r = sqrt((f32(i) + 0.5) / f32(n));
    let a = f32(i) * 2.39996323 + turn;
    return vec2<f32>(cos(a), sin(a)) * r;
}

// A bumpy normal: `n` tipped by the slope of noise at `p` (`scale` per metre, `amount`).
fn bumped(n: vec3<f32>, p: vec3<f32>, scale: f32, amount: f32) -> vec3<f32> {
    let e = 0.37;
    let q = p * scale;
    let c = noise3(q);
    let g = vec3<f32>(noise3(q + vec3<f32>(e, 0.0, 0.0)) - c, noise3(q + vec3<f32>(0.0, e, 0.0)) - c, noise3(q + vec3<f32>(0.0, 0.0, e)) - c) / e;
    return normalize(n - (g - n * dot(g, n)) * amount);
}

// ---- Light -----------------------------------------------------------------------------------

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

fn f_schlick(f0: vec3<f32>, vh: f32) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
}

// What a light from `l` reflects towards `v` off a surface (not yet times its light): its
// glossy part, with the light's own size widening the highlight (`a_light`, as Karis does).
fn glossy(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, a: f32, a_light: f32, f0: vec3<f32>) -> vec3<f32> {
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let nl = max(dot(n, l), 0.0);
    let nv = max(dot(n, v), 1e-3);
    let wide = clamp(a + a_light, 0.0, 1.0);
    let norm = (a / wide) * (a / wide);
    return d_ggx(nh, a) * norm * v_smith(nv, nl, a) * f_schlick(f0, max(dot(v, h), 0.0));
}

// The tube's shadow map's depth as distance along its axis.
fn tube_linear(d: f32) -> f32 {
    let n = room.tube_map.x;
    let f = room.tube_map.y;
    return n * f / (f + d * (n - f));
}

// How much of the moon `p` sees through the blinds (0..1), soft as far as the blinds are.
fn moon_shadow(p: vec3<f32>, n: vec3<f32>, turn: f32) -> f32 {
    let texel = room.moon_map.z;
    let lp = room.moon_view_proj * vec4<f32>(p + n * texel * 2.0, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) || lp.z >= 1.0 || lp.z <= 0.0) {
        // Beyond the window's light: the wall's shade.
        return 0.0;
    }
    let size = room.moon_map.w;
    // The blockers' depth, over as much of the map as the moon's disc can be hidden by.
    let search = 26.0 / size;
    var blockers = 0.0;
    var count = 0.0;
    for (var i = 0u; i < 12u; i++) {
        let d = textureSampleLevel(moon_map, depth_sampler, uv + disc(i, 12u, turn) * search, 0);
        if (d < lp.z - 0.0002) {
            blockers += d;
            count += 1.0;
        }
    }
    if (count < 0.5) {
        return 1.0;
    }
    // The penumbra: the distance behind the blocker times the moon's size.
    let penumbra = (lp.z - blockers / count) * room.moon_map.x * room.moon_map.y;
    let radius = clamp(penumbra / texel, 1.0, 40.0) / size;
    var lit = 0.0;
    for (var i = 0u; i < 16u; i++) {
        lit += textureSampleCompareLevel(moon_map, shadow_sampler, uv + disc(i, 16u, turn + 1.1) * radius, lp.z - 0.0001);
    }
    return lit / 16.0;
}

// One look at the moon's map, for the air and the dust.
fn moon_tap(p: vec3<f32>) -> f32 {
    let lp = room.moon_view_proj * vec4<f32>(p, 1.0);
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0)) || lp.z >= 1.0 || lp.z <= 0.0) {
        return 0.0;
    }
    return textureSampleCompareLevel(moon_map, shadow_sampler, uv, lp.z - 0.0002);
}

// How much of the tube's glass `p` sees past what stands before it (0..1).
fn tube_shadow(p: vec3<f32>, n: vec3<f32>, turn: f32) -> f32 {
    let clip = room.tube_view_proj * vec4<f32>(p + n * 0.004, 1.0);
    if (clip.w <= room.tube_map.x) {
        return 1.0;
    }
    let lp = clip.xyz / clip.w;
    let uv = vec2<f32>(lp.x * 0.5 + 0.5, 0.5 - lp.y * 0.5);
    if (any(uv <= vec2<f32>(0.0)) || any(uv >= vec2<f32>(1.0))) {
        return 1.0;
    }
    let size = room.tube_map.w;
    let receiver = clip.w;
    // The map's span at the receiver's distance, and the glass's width.
    let to_uv = 1.0 / (2.0 * room.tube_map.z * receiver);
    let width = length(room.screen_right.xyz) * 2.0;
    let search = clamp(width * 0.35 * to_uv, 2.0 / size, 48.0 / size);
    var blockers = 0.0;
    var count = 0.0;
    for (var i = 0u; i < 12u; i++) {
        let d = textureSampleLevel(tube_map, depth_sampler, uv + disc(i, 12u, turn) * search, 0);
        let z = tube_linear(d);
        if (z < receiver - 0.006) {
            blockers += z;
            count += 1.0;
        }
    }
    if (count < 0.5) {
        return 1.0;
    }
    let blocker = blockers / count;
    let penumbra = width * (receiver - blocker) / max(blocker, 0.02);
    let radius = clamp(penumbra * 0.5 * to_uv, 1.0 / size, 48.0 / size);
    var lit = 0.0;
    for (var i = 0u; i < 16u; i++) {
        lit += textureSampleCompareLevel(tube_map, shadow_sampler, uv + disc(i, 16u, turn + 0.7) * radius, lp.z);
    }
    return lit / 16.0;
}

// A tree outside the window, swaying: how much of the moon its branches and leaves leave `p`
// (0..1). Where the moonlight reaching `p` passed, a metre and a half out from the window, the
// tree stands to the front, its branches reaching in over the window's top.
fn branches(p: vec3<f32>) -> f32 {
    let l = room.moon_dir.xyz;
    if (l.x > -0.05) {
        return 1.0;
    }
    let plane = room.window.x - 1.5;
    let q = p + l * ((plane - p.x) / l.x);
    let t = room.eye.w;
    // The plane's coordinates: along the wall (z, from the window's front edge), and up (y,
    // from the window's top).
    var w = vec2<f32>(q.z - (room.window.z + room.window_half.z), q.y - (room.window.y + room.window_half.y));
    // The wind: a slow sway and a quicker shiver, more at the branches' ends.
    let reach = clamp(-w.x, 0.0, 2.0);
    w += vec2<f32>(sin(t * 0.41) * 0.03 + sin(t * 1.37) * 0.008, sin(t * 0.53 + 1.0) * 0.015 + sin(t * 1.9) * 0.005) * reach;
    var shade = 0.0;
    // Three branches from the trunk (beyond the window's front edge), reaching back over it.
    for (var i = 0; i < 3; i++) {
        let fi = f32(i);
        let a = vec2<f32>(0.35, -0.55 + fi * 0.32);
        let b = vec2<f32>(-0.55 - fi * 0.18, -0.05 + fi * 0.18 - fi * fi * 0.07);
        let ab = b - a;
        let h = clamp(dot(w - a, ab) / dot(ab, ab), 0.0, 1.0);
        let d = length(w - a - ab * h);
        let thick = mix(0.035, 0.008, h);
        shade = max(shade, 1.0 - smoothstep(thick, thick + 0.012, d));
        // Leaves clustered round the outer half.
        let c = a + ab * (0.55 + 0.4 * fi / 2.0);
        let leaf = noise3(vec3<f32>(w * 11.0, fi * 3.1 + t * 0.05)) * 0.55 + noise3(vec3<f32>(w * 29.0, fi * 1.7)) * 0.2 - length(w - c) * 1.9;
        shade = max(shade, smoothstep(0.1, 0.22, leaf));
    }
    return 1.0 - shade * 0.85;
}

// ---- The seasons -----------------------------------------------------------------------------

// A colour of the rainbow (hue 0..1, full saturation and value).
fn hue(h: f32) -> vec3<f32> {
    return clamp(abs(fract(h + vec3<f32>(0.0, 0.6666667, 0.3333333)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

// The season's lights (a tree's, a string's, a party's) on a surface at `p` facing `n`: spread
// lights, not points, the nearest of them softened and the reach faded out (irradiance over pi).
fn festive_light(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    for (var i = 0u; i < 3u; i++) {
        let reach = room.festive[i].w;
        if (reach <= 0.0) {
            continue;
        }
        let d = room.festive[i].xyz - p;
        let dist2 = max(dot(d, d), 0.0004);
        let dist = sqrt(dist2);
        if (dist >= reach) {
            continue;
        }
        let take = max(dot(n, d / dist), 0.0) * 0.75 + 0.25;
        let fade = 1.0 - smoothstep(reach * 0.4, reach, dist);
        c += room.festive_color[i].rgb * (take * fade / (PI * (dist2 + 0.02)));
    }
    return c;
}

// Snow falling outside the window, seen through the pane along the eye's ray: three layers of
// flakes beyond it, the near ones large and quick, the far ones small and slow (0..1).
fn snowfall(p: vec3<f32>) -> f32 {
    let dir = normalize(p - room.eye.xyz);
    if (dir.x > -0.03) {
        return 0.0;
    }
    let t = room.eye.w;
    var seen = 0.0;
    for (var k = 0u; k < 3u; k++) {
        var depth = 0.45;
        var cell = 0.22;
        var size = 0.012;
        var speed = 0.55;
        var bright = 0.9;
        if (k == 1u) {
            depth = 1.6;
            cell = 0.13;
            size = 0.0068;
            speed = 0.38;
            bright = 0.65;
        }
        if (k == 2u) {
            depth = 4.0;
            cell = 0.075;
            size = 0.0036;
            speed = 0.22;
            bright = 0.4;
        }
        let x = room.window.x - depth;
        let hit = room.eye.xyz + dir * ((x - room.eye.x) / dir.x);
        let fk = f32(k);
        let q = vec2<f32>(hit.z / cell, (hit.y + t * speed) / cell);
        let id = floor(q);
        let f = q - id;
        // (A flake sways as it falls; half the cells have one.)
        if (hash3(vec3<f32>(id, fk * 17.0 + 3.0)) < 0.48) {
            let sway = sin(t * 0.7 + hash3(vec3<f32>(id, fk + 11.0)) * 6.28) * 0.18;
            let centre = vec2<f32>(0.2 + 0.6 * hash3(vec3<f32>(id, fk + 91.0)) + sway, 0.2 + 0.6 * hash3(vec3<f32>(id, fk + 53.0)));
            let r = size / cell;
            seen = max(seen, bright * (1.0 - smoothstep(r * 0.35, r, distance(f, centre))));
        }
    }
    return seen;
}

// Frost on the window's pane: ice creeping in from its edges and corners, in ridges (0..1).
fn frost(p: vec3<f32>) -> f32 {
    let local = vec2<f32>((p.z - room.window.z) / room.window_half.z, (p.y - room.window.y) / room.window_half.y);
    let edge = max(abs(local.x), abs(local.y));
    let corner = abs(local.x) * abs(local.y);
    let reach = smoothstep(0.6, 1.05, edge) * 0.45 + smoothstep(0.3, 1.0, corner) * 0.75;
    let n = noise3(vec3<f32>(local * 7.0, 1.7)) * 0.6 + noise3(vec3<f32>(local * 19.0, 5.3)) * 0.4;
    let ridge = 1.0 - abs(2.0 * n - 1.0);
    return clamp(reach * 1.6 - 0.55 + ridge * 0.45 * reach, 0.0, 1.0);
}

// The fireworks over the street, on the sky outside: each burst's sparks fly out from its
// middle, slow as they go, fall a little, and fade; (the shell's own flash first).
fn fireworks(p: vec3<f32>) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    for (var b = 0u; b < 4u; b++) {
        let burst = room.bursts[b];
        let age = burst.z;
        if (age < 0.0) {
            continue;
        }
        let d = vec2<f32>(p.z - burst.x, p.y - burst.y);
        let radius = 1.25 * (1.0 - pow(1.0 - age, 3.0));
        if (length(d) > radius + 0.3) {
            continue;
        }
        let colour = hue(burst.w) * 0.8 + 0.2;
        let fade = pow(1.0 - age, 1.7) * smoothstep(0.0, 0.03, age);
        for (var k = 0u; k < 40u; k++) {
            // (Sparks on a ball seen from outside: the golden angle round, the height spread
            // evenly, so more of them lie near the rim.)
            let fk = f32(k);
            let a = fk * 2.3999632 + 6.2831853 * hash3(vec3<f32>(f32(b), 1.0, 5.0));
            let u = 2.0 * hash3(vec3<f32>(fk, f32(b), 7.0)) - 1.0;
            let reach = radius * (0.06 + 0.94 * sqrt(1.0 - u * u));
            let spark = vec2<f32>(cos(a), sin(a)) * reach + vec2<f32>(0.0, -0.45 * age * age);
            let dd = d - spark;
            c += colour * fade * (0.0005 / (dot(dd, dd) + 0.0005));
        }
        c += colour * 2.0 * (1.0 - smoothstep(0.0, 0.1, age)) * (1.0 - smoothstep(0.0, 0.25, length(d)));
    }
    return c;
}

// The moon's light on a surface: (diffuse, glossy), with the blinds' shadows and the tree's.
struct Lit {
    diffuse: vec3<f32>,
    glossy: vec3<f32>,
}

fn moon_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, a: f32, f0: vec3<f32>, turn: f32) -> Lit {
    var out: Lit;
    let l = room.moon_dir.xyz;
    let nl = dot(n, l);
    let flash = max(room.flash.x, max(room.flash.y, room.flash.z));
    if (nl <= 0.0 || (room.moon_dir.w <= 0.0 && flash <= 0.0)) {
        return out;
    }
    let seen = moon_shadow(p, n, turn);
    if (seen <= 0.0) {
        return out;
    }
    let e = (room.moon_color.rgb * room.moon_dir.w + room.flash.rgb) * nl * seen * branches(p);
    out.diffuse = e / PI;
    out.glossy = e * glossy(n, v, l, a, 0.01, f0);
    return out;
}

// The picture's colour at `uv` on the glass (0..1 from the top left) as the room's light has it:
// its 3×3 regional colours, eased, blended between.
fn glow_at(uv: vec2<f32>) -> vec3<f32> {
    let g = clamp(uv * 3.0 - 0.5, vec2<f32>(0.0), vec2<f32>(2.0));
    let i = min(vec2<u32>(g), vec2<u32>(1u));
    let f = g - vec2<f32>(i);
    let a = mix(room.glow[i.y * 3u + i.x].rgb, room.glow[i.y * 3u + i.x + 1u].rgb, f.x);
    let b = mix(room.glow[(i.y + 1u) * 3u + i.x].rgb, room.glow[(i.y + 1u) * 3u + i.x + 1u].rgb, f.x);
    return mix(a, b, f.y);
}

// A point of the glass, `c` (-1..1 across and up).
fn screen_point(c: vec2<f32>) -> vec3<f32> {
    return room.screen_center.xyz + room.screen_right.xyz * c.x + room.screen_up.xyz * c.y;
}

// The tube's light on a surface: each of its 3×3 patches shines from its place in the colour of
// its part of the picture; its reflection in a glossy surface is the picture there, blurred.
fn tube_light(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, a: f32, f0: vec3<f32>, turn: f32) -> Lit {
    var out: Lit;
    let power = room.screen_center.w * room.screen_up.w;
    if (power <= 0.0) {
        return out;
    }
    let sn = room.screen_normal.xyz;
    let to = room.screen_center.xyz - p;
    // Behind the glass, or facing away from it: no light.
    if (dot(-to, sn) <= 0.0 || dot(n, to) <= 0.0) {
        return out;
    }
    let seen = tube_shadow(p, n, turn);
    if (seen <= 0.0) {
        return out;
    }
    let hw = length(room.screen_right.xyz);
    let hh = length(room.screen_up.xyz);
    let area = 4.0 * hw * hh / 9.0;
    var e = vec3<f32>(0.0);
    for (var i = 0u; i < 9u; i++) {
        let c = vec2<f32>(f32(i % 3u) - 1.0, 1.0 - f32(i / 3u)) * 0.6667;
        let d = screen_point(c) - p;
        // (Nearer than a patch is wide, a patch is no point: its light no stronger.)
        let dist2 = max(dot(d, d), area);
        let l = d * inverseSqrt(dist2);
        let emit = max(dot(sn, -l), 0.0);
        let take = max(dot(n, l), 0.0);
        e += room.glow[i].rgb * (area * emit * take / dist2);
    }
    out.diffuse = e * room.screen_light.x * power * seen / PI;
    // The reflection: the point of the glass nearest the reflected ray (Karis's representative
    // point), the picture there blurred by the surface's roughness and the glass's size.
    let r = reflect(-v, n);
    let along = dot(r, sn);
    var hit = p + r * length(to);
    if (along < -0.001) {
        hit = p + r * max(dot(-to, sn) / -along, 0.0);
    }
    let rn = room.screen_right.xyz / hw;
    let un = room.screen_up.xyz / hh;
    let local = vec2<f32>(clamp(dot(hit - room.screen_center.xyz, rn), -hw, hw), clamp(dot(hit - room.screen_center.xyz, un), -hh, hh));
    let q = room.screen_center.xyz + rn * local.x + un * local.y;
    let dq = q - p;
    let dist = max(length(dq), 0.02);
    let l = dq / dist;
    let emit = max(dot(sn, -l), 0.0);
    let nl = max(dot(n, l), 0.0);
    if (emit > 0.0 && nl > 0.0) {
        let source = sqrt(4.0 * hw * hh / PI);
        let a_light = source / (2.0 * dist);
        let uv = vec2<f32>(0.5 + 0.5 * local.x / hw, 0.5 - 0.5 * local.y / hh);
        let size = f32(textureDimensions(picture).x);
        let lod = log2(max((a + a_light * 0.5) * size, 1.0));
        // A mirror shows the picture itself; something rough only its light, eased (the
        // picture's own frames would flicker its sheen).
        let sharp = textureSampleLevel(picture, picture_sampler, uv, lod).rgb;
        let colour = mix(glow_at(uv), sharp, 1.0 - smoothstep(0.12, 0.3, sqrt(a))) * room.screen_light.x * power;
        let solid = 4.0 * hw * hh * emit / (dist * dist);
        out.glossy = colour * glossy(n, v, l, a, a_light, f0) * nl * min(solid, 2.0 * PI) * seen;
    }
    return out;
}

// The light the tube throws back off whoever sits at it (and the room behind them): a soft fill
// from the seat in the picture's colours, reaching the wall behind the desk.
fn seat_fill(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let d = room.seat.xyz - p;
    let d2 = dot(d, d);
    let take = max(dot(n, d * inverseSqrt(max(d2, 1e-4))), 0.0) * 0.75 + 0.25;
    let screen = (room.quarters[0].rgb + room.quarters[1].rgb + room.quarters[2].rgb + room.quarters[3].rgb) * 0.25;
    return screen * room.screen_light.y * take / (d2 + 1.0);
}

// The lava lamp's glow in what is glossy (its light on the room is baked).
fn lava_glossy(p: vec3<f32>, n: vec3<f32>, v: vec3<f32>, a: f32, f0: vec3<f32>) -> vec3<f32> {
    let d = room.lava.xyz - p;
    let dist2 = max(dot(d, d), 0.0025);
    let l = d * inverseSqrt(dist2);
    let nl = max(dot(n, l), 0.0);
    let dist = sqrt(dist2);
    let intensity = room.lava_color.rgb * room.lava.w * 0.6;
    return intensity * glossy(n, v, l, a, 0.05 / dist, f0) * nl / dist2;
}

// ---- Materials -------------------------------------------------------------------------------

struct Surface {
    albedo: vec3<f32>,
    rough: f32,
    metallic: f32,
    n: vec3<f32>,
    emission: vec3<f32>,
}

fn glow_of(slot: u32) -> f32 {
    return room.glows[slot / 4u][slot % 4u];
}

// The desk: varnished wood, its grain running along it, worn pale where the arms rest, an old
// ring from a mug.
fn desk_wood(p: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
    let warp = fbm(vec3<f32>(p.x * 0.7, p.y * 3.0, p.z * 3.0));
    let rings = p.z * 24.0 + p.y * 9.0 + warp * 3.2 + noise3(vec3<f32>(p.x * 0.25, 0.0, p.z * 2.0)) * 1.5;
    let band = abs(fract(rings) - 0.5) * 2.0;
    let late = smoothstep(0.55, 0.95, band);
    let fibre = noise3(vec3<f32>(p.x * 7.0, p.y * 140.0, p.z * 140.0));
    let pores = smoothstep(0.72, 0.9, noise3(vec3<f32>(p.x * 22.0, p.y * 600.0, p.z * 600.0)));
    var c = base * mix(1.12, 0.62, late) * mix(0.9, 1.08, fibre) * (1.0 - pores * 0.25);
    // Wear along the front edge, where wrists and elbows rest.
    let front = smoothstep(-0.02, 0.1, p.z) * step(0.735, p.y);
    c = mix(c, c * 1.18 + vec3<f32>(0.015, 0.01, 0.004), front * 0.5 * smoothstep(0.3, 0.7, noise3(p * 18.0)));
    // A ring left by a mug long ago.
    let ring = abs(length(p.xz - vec2<f32>(-0.24, 0.07)) - 0.039);
    c *= 1.0 - 0.28 * (1.0 - smoothstep(0.0012, 0.004, ring)) * step(0.735, p.y) * smoothstep(0.2, 0.6, noise3(p * 60.0));
    return c;
}

// A surface's material at `p` (`texel`: its picture there, for a picture).
fn surface(m: Material, role: u32, p: vec3<f32>, n: vec3<f32>, texel: vec4<f32>) -> Surface {
    var s: Surface;
    s.albedo = m.base.rgb;
    s.rough = m.params.x;
    s.metallic = m.params.y;
    s.n = n;
    s.emission = vec3<f32>(0.0);
    switch role {
        case WOOD: {
            s.albedo = desk_wood(p, m.base.rgb);
            s.rough = clamp(m.params.x + (noise3(p * 40.0) - 0.5) * 0.12, 0.2, 0.9);
            s.n = bumped(n, p, 220.0, 0.025);
        }
        case LAMINATE: {
            let grain = noise3(vec3<f32>(p.x * 3.0, p.y * 90.0, p.z * 3.0));
            s.albedo = m.base.rgb * mix(0.85, 1.1, grain);
        }
        case PAINT: {
            s.albedo = m.base.rgb * (0.96 + 0.06 * noise3(p * 23.0));
            s.n = bumped(n, p, 700.0, 0.06);
            s.rough = m.params.x + (noise3(p * 300.0) - 0.5) * 0.08;
        }
        case CARPET: {
            s.albedo = m.base.rgb * (0.65 + 0.7 * noise3(p * 260.0)) * (0.85 + 0.3 * noise3(p * 9.0));
            s.n = bumped(n, p, 400.0, 0.5);
        }
        case PLASTIC: {
            // The fine texture moulded into the cases, and the beige yellowed unevenly.
            s.n = bumped(n, p, 1500.0, 0.05);
            s.albedo = m.base.rgb * (0.97 + 0.05 * noise3(p * 7.0)) * vec3<f32>(1.0, 0.995, 0.97 + 0.03 * noise3(p * 3.0));
            s.rough = m.params.x + (noise3(p * 900.0) - 0.5) * 0.1;
        }
        case PICTURE: {
            s.albedo = texel.rgb;
            if (m.emissive.w > 0.5) {
                // A display: lit segments behind a filter, a backlit screen.
                s.emission = texel.rgb * m.emissive.y * glow_of(u32(m.emissive.w) - 1u);
                s.albedo = texel.rgb * m.emissive.x;
            } else {
                // Paper rolled and flattened: a gentle wave.
                s.n = bumped(n, p, 14.0, 0.02);
            }
        }
        case GLOW: {
            s.emission = m.emissive.rgb * glow_of(u32(m.params.w));
            s.albedo = m.base.rgb * 0.15;
        }
        case FABRIC: {
            let weave = noise3(p * vec3<f32>(900.0, 900.0, 60.0)) * noise3(p * vec3<f32>(60.0, 900.0, 900.0));
            s.albedo = m.base.rgb * (0.75 + 0.5 * weave);
        }
        case PAPER: {
            s.albedo = m.base.rgb * (0.95 + 0.08 * noise3(p * 450.0));
        }
        case COFFEE: {
            // The coffee: dark and glassy, trembling a little.
            let t = room.eye.w;
            s.n = normalize(n + vec3<f32>(sin(p.x * 160.0 + t * 1.3), 0.0, cos(p.z * 150.0 - t * 1.1)) * 0.015);
            s.rough = 0.05;
        }
        default: {}
    }
    // Dust settles on what faces up: thick on the monitor's and the computer's tops, thin on
    // the desk where hands wipe it, none on what glows or is painted on.
    let up = smoothstep(0.8, 0.97, n.y);
    var settled = 0.0;
    switch role {
        case PLASTIC: {
            settled = 0.55;
        }
        case WOOD, LAMINATE, PAPER, PLAIN: {
            settled = 0.3;
        }
        default: {}
    }
    if (up * settled > 0.0) {
        // (Less over the desk's front, where the keyboard and the mouse are used.)
        let used = 1.0 - 0.7 * smoothstep(-0.3, 0.0, p.z) * step(p.y, 0.8);
        let dust = (smoothstep(0.35, 0.85, noise3(p * 70.0)) * 0.6 + noise3(p * 700.0) * 0.4) * up * settled * used;
        s.albedo = mix(s.albedo, vec3<f32>(0.4, 0.39, 0.36), dust * 0.4);
        s.rough = mix(s.rough, 0.95, dust);
    }
    s.rough = clamp(s.rough, 0.045, 1.0);
    return s;
}

// ---- Surfaces --------------------------------------------------------------------------------

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) light: vec4<f32>,
    @location(4) tube: vec4<f32>,
    @location(5) lava: vec4<f32>,
    @location(6) material: u32,
}

// (At the triangle's centroid: multisampled, a pixel whose centre lies off a thin triangle (a
// bevel's lip seen edge-on, a button's) would carry its normal, place and baked light on past
// the triangle's corners, the light squared after: specks thousands of times too bright, split
// into red, green and blue by the lens's fringe.)
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(perspective, centroid) world: vec3<f32>,
    @location(1) @interpolate(perspective, centroid) normal: vec3<f32>,
    @location(2) @interpolate(perspective, centroid) uv: vec2<f32>,
    @location(3) @interpolate(perspective, centroid) light: vec4<f32>,
    @location(4) @interpolate(perspective, centroid) tube: vec4<f32>,
    @location(5) @interpolate(perspective, centroid) lava: vec4<f32>,
    @location(6) @interpolate(flat) material: u32,
}

@vertex
fn vs_surface(in: VIn) -> VOut {
    var out: VOut;
    out.clip = room.view_proj * vec4<f32>(in.pos, 1.0);
    out.world = in.pos;
    out.normal = in.normal;
    out.uv = in.uv;
    out.light = in.light;
    out.tube = in.tube;
    out.lava = in.lava;
    out.material = in.material;
    return out;
}

// The light the bakes left at a point (the moon's bounce as bright as the moon is now, each of
// the tube's quarters in its colour, the lava lamp's), and its occlusion.
fn baked(in: VOut) -> vec3<f32> {
    let bounce = in.light.rgb * in.light.rgb * room.scales.x * (room.moon_dir.w / 3.0);
    let t = in.tube * in.tube * room.scales.y;
    let tube = room.quarters[0].rgb * t.x + room.quarters[1].rgb * t.y + room.quarters[2].rgb * t.z + room.quarters[3].rgb * t.w;
    let lava = in.lava.rgb * in.lava.rgb * room.scales.z * room.lava.w;
    return bounce + tube + lava + vec3<f32>(room.scales.w * in.light.a);
}

// The baked light, and the fill from the seat (where nothing hides it: its occlusion).
fn ambient(in: VOut, n: vec3<f32>) -> vec3<f32> {
    return baked(in) + seat_fill(in.world, n) * in.light.a;
}

// A surface lit: everything but what glows of itself.
fn shade(in: VOut, s: Surface, v: vec3<f32>) -> vec3<f32> {
    let turn = surface_turn(in.world);
    let a = s.rough * s.rough;
    let f0 = mix(vec3<f32>(0.04), s.albedo, s.metallic);
    let diffuse = s.albedo * (1.0 - s.metallic);
    let indirect = ambient(in, s.n);
    let moon = moon_light(in.world, s.n, v, a, f0, turn);
    let tube = tube_light(in.world, s.n, v, a, f0, turn);
    var c = diffuse * (indirect + moon.diffuse + tube.diffuse + festive_light(in.world, s.n) * (0.35 + 0.65 * in.light.a));
    c += moon.glossy + tube.glossy + lava_glossy(in.world, s.n, v, a, f0);
    // What the room's own dim light reflects in what is glossy.
    let nv = max(dot(s.n, v), 0.0);
    let fresnel = f0 + (max(vec3<f32>(1.0 - s.rough), f0) - f0) * pow(1.0 - nv, 5.0);
    c += fresnel * indirect * in.light.a * (1.0 - s.rough) * 0.6;
    return c;
}

@fragment
fn fs_surface(in: VOut) -> @location(0) vec4<f32> {
    let m = materials[in.material];
    let role = u32(m.params.z);
    // (Sampled here, where every pixel does, for its mipmaps.)
    let texel = textureSample(layers, layer_sampler, in.uv, u32(m.params.w));
    // How fast the surface turns across a pixel: a glint narrower than a pixel (on a bevel's
    // curve, a key's edge) would only shimmer, so the gloss is widened to what a pixel shows.
    let turn_rate = fwidth(in.normal);
    let widen = min(2.0 * dot(turn_rate, turn_rate), 0.2);
    let v = normalize(room.eye.xyz - in.world);
    var n = normalize(in.normal);
    // (Thin things are seen from both sides.)
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    switch role {
        case SKY: {
            // The night outside: deep blue, paler near the horizon, a few stars.
            let up = clamp((in.world.y - 0.5) / 4.0, 0.0, 1.0);
            var sky = mix(vec3<f32>(0.03, 0.045, 0.09), vec3<f32>(0.006, 0.01, 0.028), up);
            let star = step(0.996, hash3(floor(in.world * 90.0)));
            sky += vec3<f32>(0.6, 0.65, 0.8) * star * up;
            if (room.season.x > 1.5) {
                sky += fireworks(in.world);
            } else if (room.season.x > 0.5) {
                sky += vec3<f32>(0.022, 0.02, 0.03) * (1.0 - up * 0.5);
            }
            return vec4<f32>(sky, 1.0);
        }
        case MOON: {
            return vec4<f32>(m.emissive.rgb * 0.5, 1.0);
        }
        case BACKDROP: {
            return vec4<f32>(m.base.rgb, 1.0);
        }
        default: {}
    }
    var s = surface(m, role, in.world, n, texel);
    let a = s.rough * s.rough;
    s.rough = sqrt(sqrt(a * a + widen));
    return vec4<f32>(shade(in, s, v) + s.emission, 1.0);
}

// ---- The window's pane -------------------------------------------------------------------------

@fragment
fn fs_glass(in: VOut) -> @location(0) vec4<f32> {
    let v = normalize(room.eye.xyz - in.world);
    var n = normalize(in.normal);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    let f0 = vec3<f32>(0.04);
    let a = 0.003;
    let turn = surface_turn(in.world);
    let tube = tube_light(in.world, n, v, a, f0, turn);
    let fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
    var rgb = tube.glossy + lava_glossy(in.world, n, v, a, f0) + baked(in) * fresnel;
    var alpha = fresnel * 0.5;
    if (room.season.x > 0.5 && room.season.x < 1.5) {
        // Christmas: the snow outside and the frost on the pane.
        let cover = clamp(max(snowfall(in.world) * 0.85, frost(in.world) * 0.55), 0.0, 0.9);
        rgb += (vec3<f32>(0.5, 0.58, 0.72) + baked(in) * 0.5) * cover;
        alpha += cover * (1.0 - alpha);
    }
    return vec4<f32>(rgb, alpha);
}

// ---- The lava lamp ---------------------------------------------------------------------------

// The glass's radius at a height above the desk (its shape, as tools/room/build.py turns it).
fn lamp_radius(h: f32) -> f32 {
    var hs = array<f32, 9>(0.135, 0.15, 0.17, 0.2, 0.24, 0.28, 0.32, 0.355, 0.375);
    var rs = array<f32, 9>(0.042, 0.0465, 0.0485, 0.0488, 0.0472, 0.0442, 0.0402, 0.0355, 0.0315);
    if (h <= hs[0]) {
        return rs[0];
    }
    for (var i = 1; i < 9; i++) {
        if (h <= hs[i]) {
            return mix(rs[i - 1], rs[i], (h - hs[i - 1]) / (hs[i] - hs[i - 1]));
        }
    }
    return rs[8];
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

fn ellipsoid(q: vec3<f32>, r: vec3<f32>) -> f32 {
    let k0 = length(q / r);
    let k1 = length(q / (r * r));
    return k0 * (k0 - 1.0) / max(k1, 1e-6);
}

// The wax (signed distance, metres) at `q`: the lamp's axis through x = z = 0, y the height
// above the desk. A pool over the bulb, one under the cap, and blobs rising, stretching and
// sinking between them.
fn wax(q: vec3<f32>, t: f32) -> f32 {
    var d = ellipsoid(q - vec3<f32>(0.0, 0.148, 0.0), vec3<f32>(0.043, 0.018, 0.043));
    d = smin(d, ellipsoid(q - vec3<f32>(0.0, 0.362, 0.0), vec3<f32>(0.03, 0.009, 0.03)), 0.012);
    for (var i = 0u; i < 5u; i++) {
        let fi = f32(i);
        let speed = 0.05 + 0.025 * rand_u(i * 7u + 1u);
        let phase = t * speed * 6.2831853 + fi * 2.1;
        // Up and down, lingering at the ends as the wax warms and cools.
        let s = 0.5 - 0.5 * cos(phase);
        let h = mix(0.17, 0.335, s * s * (3.0 - 2.0 * s));
        let r = 0.011 + 0.008 * rand_u(i * 7u + 3u);
        let room_left = max(lamp_radius(h) - r - 0.006, 0.0);
        let o = vec2<f32>(sin(phase * 0.7 + fi), cos(phase * 0.9 + fi * 2.0)) * min(0.012, room_left);
        // Stretched while it moves fastest.
        let stretch = 1.0 + 0.6 * abs(sin(phase));
        d = smin(d, ellipsoid(q - vec3<f32>(o.x, h, o.y), vec3<f32>(r / sqrt(stretch), r * stretch, r / sqrt(stretch))), 0.02);
    }
    return d;
}

fn wax_normal(q: vec3<f32>, t: f32) -> vec3<f32> {
    let e = vec2<f32>(0.0008, -0.0008);
    return normalize(e.xyy * wax(q + e.xyy, t) + e.yyx * wax(q + e.yyx, t) + e.yxy * wax(q + e.yxy, t) + e.xxx * wax(q + e.xxx, t));
}

@fragment
fn fs_lava(in: VOut) -> @location(0) vec4<f32> {
    let t = room.eye.w;
    let warm = room.lava.w;
    let v = normalize(room.eye.xyz - in.world);
    var n = normalize(in.normal);
    if (dot(n, v) < 0.0) {
        n = -n;
    }
    // The lamp's space: its axis, the desk's height (the anchor is 24 cm above it).
    let desk = room.lava.y - 0.24;
    let axis = room.lava.xz;
    let dir = -v;
    let start = vec3<f32>(in.world.x - axis.x, in.world.y - desk, in.world.z - axis.y);
    // Through the glass to its far side (a cylinder as wide as the glass where the ray enters).
    let radius = lamp_radius(start.y) - 0.003;
    let o = start.xz;
    let d2 = dir.xz;
    let qa = max(dot(d2, d2), 1e-5);
    let qb = dot(o, d2);
    let qc = dot(o, o) - radius * radius;
    let far = max((-qb + sqrt(max(qb * qb - qa * qc, 0.0))) / qa, 0.0);
    let steps = 40;
    let dt = far / f32(steps);
    var march = hash3(floor(in.world * 1400.0)) * dt;
    // (What a pixel spans there: the wax's edge is blended over it, below.)
    let pixel = max(length(dpdx(in.world)), length(dpdy(in.world)));
    var q = start;
    // How near the ray came to the wax, where, and the liquid's glow up to there.
    var nearest = 1e9;
    var near_q = start;
    var near_liquid = vec3<f32>(0.0);
    // The liquid glows yellow with the bulb's light, brightest low down; the wax, red-orange,
    // brightest where it is thin and near the bulb.
    var liquid = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        q = start + dir * march;
        let d = wax(q, t);
        if (d < nearest) {
            nearest = d;
            near_q = q;
            near_liquid = liquid;
        }
        if (d < 0.0006) {
            break;
        }
        let heat = 1.0 - smoothstep(0.14, 0.36, q.y);
        let step = min(max(d * 0.8, dt * 0.5), far - march + 1e-4);
        liquid += vec3<f32>(1.0, 0.5, 0.1) * (0.8 + 4.5 * heat * heat) * step;
        march += step;
        if (march >= far) {
            break;
        }
    }
    // The wax covers the pixel as far as the ray came within a pixel of it: one ray a pixel
    // and a hit or a miss stepped its outline (multisampling smooths triangles' edges, and the
    // wax is no triangle), crawling as the blobs rose.
    let cover = 1.0 - smoothstep(0.0006, 0.0006 + pixel, nearest);
    // Past the wax, the liquid all the way through and the far side of the glass, dimmer.
    var c = (liquid + vec3<f32>(0.9, 0.42, 0.1) * 0.12) * warm;
    if (cover > 0.0) {
        let wn = wax_normal(near_q, t);
        let rim = 1.0 - abs(dot(wn, -dir));
        let heat = 1.0 - smoothstep(0.15, 0.36, near_q.y);
        let wax_glow = vec3<f32>(1.0, 0.2, 0.035) * (0.45 + 2.2 * heat) * (0.6 + 1.3 * rim * rim);
        c = mix(c, (near_liquid + wax_glow) * warm, cover);
    }
    // The bulb's glare under the wax pool.
    c += vec3<f32>(1.0, 0.62, 0.25) * 2.0 * warm * (1.0 - smoothstep(0.135, 0.145, start.y));
    // The glass over it: the tube's reflection and the room's.
    let f0 = vec3<f32>(0.04);
    let tube = tube_light(in.world, n, v, 0.01, f0, surface_turn(in.world));
    let fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
    c += tube.glossy + baked(in) * fresnel * 0.2;
    return vec4<f32>(c, 1.0);
}

// ---- The tube --------------------------------------------------------------------------------

struct ScreenOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // 0..1 across the glass, from the top left
    @location(2) uv: vec2<f32>,
}

// The glass as a grid (room.rs), bulging out in the middle.
@vertex
fn vs_screen(@location(0) grid: vec2<f32>) -> ScreenOut {
    let c = grid * 2.0 - 1.0;
    let bulge = room.screen_right.w * (1.0 - 0.5 * (c.x * c.x + c.y * c.y));
    let right = room.screen_right.xyz;
    let up = room.screen_up.xyz;
    let sn = room.screen_normal.xyz;
    let p = room.screen_center.xyz + right * c.x + up * c.y + sn * bulge;
    // The bulge's slope tips the normal outwards towards the edges.
    let tip = room.screen_right.w;
    let n = normalize(sn + normalize(right) * c.x * tip / length(right) + normalize(up) * c.y * tip / length(up));
    var out: ScreenOut;
    out.clip = room.view_proj * vec4<f32>(p, 1.0);
    out.world = p;
    out.normal = n;
    out.uv = vec2<f32>(grid.x, 1.0 - grid.y);
    return out;
}

// The light that the room's exposure and tone curve show as `display` (0..1): the glass shows
// the movies in their own colours.
fn shown_as(display: vec3<f32>) -> vec3<f32> {
    let y = clamp(display, vec3<f32>(0.0), vec3<f32>(0.97));
    return pow(room.look.z * y / (1.0 - y), vec3<f32>(1.0 / room.look.y)) / room.look.x;
}

// Turns a colour's hue by `a` radians (the degaussing's swirl).
fn hue_turn(c: vec3<f32>, a: f32) -> vec3<f32> {
    let k = vec3<f32>(0.57735);
    let ca = cos(a);
    return c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca);
}

@fragment
fn fs_screen(in: ScreenOut) -> @location(0) vec4<f32> {
    let power = room.screen_center.w;
    let t = room.eye.w;
    let v = normalize(room.eye.xyz - in.world);
    let n = normalize(in.normal);
    let degauss = room.moon_color.w;
    // The picture sits a little inside the glass's edge, bowed with it; while the tube
    // degausses the beam shudders.
    let c = in.uv - 0.5;
    let r2 = dot(c, c);
    var warped = 0.5 + c * (1.0 + 0.1 * r2) * 1.04;
    warped += degauss * 0.012 * vec2<f32>(sin(t * 47.0 + warped.y * 13.0), sin(t * 31.0 + warped.x * 9.0));
    // How many of the picture's pixels and rows a pixel of the view covers.
    let size = vec2<f32>(textureDimensions(picture));
    let footprint = max(fwidth(warped.x) * size.x, fwidth(warped.y) * size.y);
    let rows_per_pixel = fwidth(warped.y * size.y);
    let lod = log2(max(footprint, 1.0));
    // Power: the picture opens from a bright line across the middle as the tube warms, and
    // closes to it when it goes off.
    let open = smoothstep(0.35, 1.0, power);
    let line = smoothstep(0.0, 0.35, power);
    let half_height = mix(0.004, 0.5, open);
    let in_y = 1.0 - smoothstep(half_height - 0.006, half_height + 0.006, abs(warped.y - 0.5));
    let in_x = 1.0 - smoothstep(0.5 * line - 0.01, 0.5 * line + 0.01, abs(warped.x - 0.5));
    var shown = vec3<f32>(0.0);
    let inside = warped.x > 0.0 && warped.x < 1.0 && warped.y > 0.0 && warped.y < 1.0;
    if (inside && room.screen_up.w > 0.5) {
        // Scanlines: the picture's rows lit, with dark gaps the brightest rows fill. Seen from
        // too far for the rows to resolve they would only beat into moiré: they fade to their
        // average instead.
        let s = sin(fract(warped.y * size.y) * PI);
        let sample = textureSampleLevel(picture, picture_sampler, warped, lod).rgb;
        let lum = dot(sample, vec3<f32>(0.2126, 0.7152, 0.0722));
        let resolved = 1.0 - smoothstep(0.35, 0.7, rows_per_pixel);
        let scan = mix(1.0, mix(0.35 + 0.65 * s * s, 1.0, lum * 0.5), resolved) * mix(0.85, 1.0, resolved);
        // The aperture grille: stripes of red, green and blue phosphor.
        let stripe = u32(in.clip.x) % 3u;
        var mask = vec3<f32>(0.9);
        mask[stripe] = 1.06;
        shown = sample * scan * mix(vec3<f32>(1.0), mask, 0.5 + 0.5 * resolved);
        // The phosphor's glow in the glass: a soft halo round what is bright.
        shown += textureSampleLevel(picture, picture_sampler, warped, lod + 4.0).rgb * 0.1;
        // Darker into the corners, as a tube's are.
        let edge = min(min(warped.x, 1.0 - warped.x), min(warped.y, 1.0 - warped.y));
        shown *= smoothstep(0.0, 0.04, edge) * (1.0 - r2 * 0.6);
        // The degaussing: colours swirl in patches across the glass and settle.
        if (degauss > 0.0) {
            let swirl = sin(warped.x * 7.0 + t * 3.0) + sin(warped.y * 5.0 - t * 2.3) + sin((warped.x + warped.y) * 4.0 + t * 1.7);
            shown = max(hue_turn(shown, swirl * degauss * 1.4), vec3<f32>(0.0)) * (1.0 + degauss * 0.3);
        }
    }
    // While it warms up (or cools off), the line and the opening picture flare.
    let flare = (1.0 - open) * line * 4.0;
    shown = shown_as((shown * in_y * in_x + vec3<f32>(0.85, 0.9, 1.0) * flare * in_y * in_x) * room.screen_normal.w);
    // A faint flicker; the phosphor's own dim glow when there is no picture.
    shown *= 0.985 + 0.015 * sin(t * 377.0);
    let idle = vec3<f32>(0.0016, 0.0024, 0.0021) * (0.3 + 0.7 * power);
    // The glass: dark grey-green, smudged with fingerprints; the lava lamp, the moonlit room
    // and the screen's own light thrown back from the room in it.
    let f0 = vec3<f32>(0.045);
    let smudge = smoothstep(0.45, 0.75, fbm(vec3<f32>(in.uv * vec2<f32>(9.0, 7.0), 3.7)));
    let a = mix(0.003, 0.08, smudge);
    let fresnel = 0.045 + 0.955 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
    let lava = lava_glossy(in.world, n, v, a, f0);
    let room_glow = (room.quarters[0].rgb + room.quarters[1].rgb + room.quarters[2].rgb + room.quarters[3].rgb) * 0.25;
    let behind = (room_glow * 0.02 + room.moon_color.rgb * room.moon_dir.w * 0.002) * fresnel * (1.0 + smudge);
    return vec4<f32>(shown + idle + lava + behind, 1.0);
}

// ---- Dust and steam --------------------------------------------------------------------------

struct MoteOut {
    @builtin(position) clip: vec4<f32>,
    // -1..1 across the mote
    @location(0) local: vec2<f32>,
    @location(1) colour: vec3<f32>,
    // 1 a speck, 0 a puff of steam
    @location(2) @interpolate(flat) speck: f32,
}

// The light a point in the air scatters towards the eye (per steradian, before its density):
// the moon's through the blinds (`moon_phase`: how much of it turns towards the eye), the
// tube's and the lava lamp's, scattered evenly.
fn air_light(p: vec3<f32>, moon_phase: f32) -> vec3<f32> {
    var c = vec3<f32>(0.0);
    let moon = moon_tap(p);
    if (moon > 0.0) {
        c = (room.moon_color.rgb * room.moon_dir.w + room.flash.rgb) * (moon * branches(p) * moon_phase);
    }
    for (var i = 0u; i < 3u; i++) {
        if (room.festive[i].w > 0.0) {
            let l = room.festive[i].xyz - p;
            c += room.festive_color[i].rgb * (0.35 / max(dot(l, l), 0.01) / (4.0 * PI));
        }
    }
    let to = p - room.screen_center.xyz;
    // (No point source near the glass: the glow is as wide as the glass is.)
    let d2 = dot(to, to) + 0.04;
    let emit = max(dot(room.screen_normal.xyz, to * inverseSqrt(d2)), 0.0);
    let area = 4.0 * length(room.screen_right.xyz) * length(room.screen_up.xyz);
    let screen = (room.quarters[0].rgb + room.quarters[1].rgb + room.quarters[2].rgb + room.quarters[3].rgb) * 0.25;
    c += screen * (area * emit / d2 / (8.0 * PI));
    let l = room.lava.xyz - p;
    c += room.lava_color.rgb * (room.lava.w * 0.05 / max(dot(l, l), 0.01) / (4.0 * PI));
    return c;
}

@vertex
fn vs_mote(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> MoteOut {
    var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let corner = corners[vi];
    let t = room.eye.w;
    var out: MoteOut;
    var p: vec3<f32>;
    var size: f32;
    let first_steam = u32(room.steam.w);
    // The camera's right and up, from its picture's corners.
    let centre4 = room.inv_view_proj * vec4<f32>(0.0, 0.0, 0.5, 1.0);
    let right4 = room.inv_view_proj * vec4<f32>(1.0, 0.0, 0.5, 1.0);
    let up4 = room.inv_view_proj * vec4<f32>(0.0, 1.0, 0.5, 1.0);
    let centre = centre4.xyz / centre4.w;
    let right = normalize(right4.xyz / right4.w - centre);
    let up = normalize(up4.xyz / up4.w - centre);
    let forward = normalize(centre - room.eye.xyz);
    if (ii < first_steam) {
        // Dust, drifting in the air over the desk and through the moonbeams, wrapping round.
        let h = vec3<f32>(rand_u(ii * 3u), rand_u(ii * 3u + 1u), rand_u(ii * 3u + 2u));
        let lo = vec3<f32>(-1.3, 0.78, -0.56);
        let span = vec3<f32>(2.3, 1.1, 1.06);
        var q = h * span + vec3<f32>(sin(t * 0.07 + h.y * 31.0) * 0.12 + t * 0.006, -t * (0.002 + 0.004 * h.x) + sin(t * 0.11 + h.z * 17.0) * 0.05, cos(t * 0.05 + h.x * 23.0) * 0.1);
        q = q - floor(q / span) * span;
        p = lo + q;
        size = 0.0005 + 0.0007 * h.z;
        // Specks tumble: they flash as a face catches the light.
        let flash = 0.35 + 0.65 * pow(0.5 + 0.5 * sin(t * (1.5 + 3.0 * h.x) + h.y * 40.0), 4.0);
        out.colour = air_light(p, 2.5) * flash;
        out.speck = 1.0;
    } else {
        // The coffee's steam: puffs rising from the mug, widening and fading as they go.
        let k = ii - first_steam;
        let life = 5.0;
        let age = fract(t / life + rand_u(k * 5u + 11u)) * life;
        let swirl = vec2<f32>(sin(age * 1.7 + f32(k) * 1.3), cos(age * 1.3 + f32(k) * 2.1)) * (0.004 + 0.006 * age);
        p = room.steam.xyz + vec3<f32>(swirl.x, age * 0.032, swirl.y);
        size = 0.006 + age * 0.008;
        let fade = smoothstep(0.0, 0.6, age) * (1.0 - smoothstep(1.2, 4.5, age));
        out.colour = air_light(p, 0.5) * fade * 0.5;
        out.speck = 0.0;
    }
    // Specks too small for a pixel are drawn as one, as dim as they are small.
    let clip = room.view_proj * vec4<f32>(p, 1.0);
    let edge = room.view_proj * vec4<f32>(p + up * size, 1.0);
    let pixels = abs(edge.y / edge.w - clip.y / clip.w) * 0.5 / room.pixel.y;
    let least = 1.1;
    if (pixels < least && pixels > 0.0) {
        out.colour *= (pixels / least) * (pixels / least);
        size *= least / pixels;
    }
    let world = p + (right * corner.x + up * corner.y) * size;
    out.clip = room.view_proj * vec4<f32>(world, 1.0);
    out.local = corner;
    // (Behind the eye: nothing.)
    if (dot(p - room.eye.xyz, forward) < 0.05) {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    return out;
}

@fragment
fn fs_mote(in: MoteOut) -> @location(0) vec4<f32> {
    let r2 = dot(in.local, in.local);
    if (r2 > 1.0) {
        discard;
    }
    let speck = exp(-r2 * 4.0);
    let puff = (1.0 - r2) * (1.0 - r2);
    return vec4<f32>(in.colour * mix(puff, speck, in.speck), 0.0);
}

// ---- The light in the air --------------------------------------------------------------------

// The air's density (per metre): a faint haze and slowly drifting dust, thicker low down.
fn density(p: vec3<f32>, t: f32) -> f32 {
    // (Dustier than a clean room: the moon's beams through the blinds and the tube's glow show
    // in the air.)
    let dust = noise3(p * 1.7 + vec3<f32>(t * 0.02, -t * 0.012, t * 0.015));
    return 0.045 + 0.05 * dust;
}

// The scene's distance at a pixel (metres from the eye, at most `cap`).
fn scene_distance(px: vec2<i32>, cap: f32) -> f32 {
    let depth = textureLoad(scene_depth, px, 0);
    let ndc = vec2<f32>((f32(px.x) + 0.5) * room.pixel.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) * room.pixel.y * 2.0);
    let w = room.inv_view_proj * vec4<f32>(ndc, depth, 1.0);
    return min(distance(w.xyz / w.w, room.eye.xyz), cap);
}

const FOG_REACH: f32 = 5.0;

// Half resolution: marches each pixel's ray to the scene, gathering the light the air throws
// towards the eye. rgb = that light, a = how far it went.
@fragment
fn fs_fog(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = min(vec2<i32>(in.pos.xy * 2.0), dims - 1);
    let reach = scene_distance(px, FOG_REACH);
    let ndc = vec2<f32>((f32(px.x) + 0.5) * room.pixel.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) * room.pixel.y * 2.0);
    let far4 = room.inv_view_proj * vec4<f32>(ndc, 1.0, 1.0);
    let dir = normalize(far4.xyz / far4.w - room.eye.xyz);
    let t = room.eye.w;
    let steps = 28;
    let dt = reach / f32(steps);
    // (The same for a pixel every frame: re-rolled, the march's noise flickers.)
    let jitter = ign(in.pos.xy);
    // The moonlight scatters a little more forwards than back.
    let g = 0.1;
    let cos_theta = dot(dir, -room.moon_dir.xyz);
    let phase = (1.0 - g * g) / (4.0 * PI * pow(1.0 + g * g - 2.0 * g * cos_theta, 1.5));
    var light = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let p = room.eye.xyz + dir * ((f32(i) + jitter) * dt);
        light += air_light(p, phase) * density(p, t);
    }
    return vec4<f32>(light * dt, reach);
}

// Full resolution: the air's light, its half-resolution samples weighed by how near their
// distance is to this pixel's (so it stays on its side of edges), laid over the picture.
@fragment
fn fs_fog_composite(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(scene_depth));
    let px = min(vec2<i32>(in.pos.xy), dims - 1);
    let here = scene_distance(px, FOG_REACH);
    let half_dims = vec2<i32>(textureDimensions(fog_light));
    let centre = in.pos.xy * 0.5 - 0.5;
    let base = vec2<i32>(floor(centre));
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    for (var y = -1; y <= 2; y++) {
        for (var x = -1; x <= 2; x++) {
            let q = clamp(base + vec2<i32>(x, y), vec2<i32>(0), half_dims - 1);
            let s = textureLoad(fog_light, q, 0);
            let d = vec2<f32>(q) - centre;
            let w = exp(-dot(d, d) * 0.6) / (1.0 + 60.0 * abs(s.a - here));
            sum += s.rgb * w;
            weight += w;
        }
    }
    return vec4<f32>(sum / max(weight, 1e-4), 0.0);
}
