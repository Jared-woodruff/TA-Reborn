// TA Reborn's own trees, bushes and rocks (foliage.rs): drawn in place of TA's painted feature
// sprites when the camera comes close, each stretched and tinted to stand in for its painting.

@group(1) @binding(0) var layers: texture_2d_array<f32>;
@group(1) @binding(1) var layer_sampler: sampler;
// Each layer's mean colour (linear): the instances give the colours to take them to.
@group(1) @binding(2) var<uniform> layer_means: array<vec4<f32>, 19>;
// A unit followed from behind (Perspective): xyz where it is, w how far round the line from the
// eye to it the leaves and branches give way (0: none).
@group(1) @binding(3) var<uniform> sight: vec4<f32>;

// The leaves' layers come before this one; then bark and rock.
const FIRST_OPAQUE: u32 = 7u;
// TA's paintings carry the light they were painted in. The leaves' colour is taken from a
// painting's lit side and the bark's and rock's from all of it (shade included): this much more
// gives them the brightness they were painted at, lit as this renderer lights them.
const LEAF_LIGHT: f32 = 0.95;
const BARK_LIGHT: f32 = 1.2;

// The rock's layer, laid on along the world's three axes (the rocks have no texture
// coordinates), and how many world units its texture spans; and the crystal's, so too.
const ROCK_LAYER: u32 = 9u;
const ROCK_TILE: f32 = 44.0;
const CRYSTAL_LAYER: u32 = 11u;
const CRYSTAL_TILE: f32 = 24.0;
// How much of an energy node's light shows through its fingers (as their `emit`, see `crystal`).
const GLOW_THROUGH: f32 = 0.3;
// A vehicle's painted metal, so too, and pipework's steel.
const HULL_LAYER: u32 = 13u;
const HULL_TILE: f32 = 40.0;
const METAL_LAYER: u32 = 14u;
const METAL_TILE: f32 = 36.0;
// A second paint (a relay's roof), so too: its colour the instance's leaves'.
const TRIM_LAYER: u32 = 16u;
const TRIM_TILE: f32 = 24.0;
// TA Reborn's own festive features (tools/features/festive.py), their colours their own: the
// snow lying on them, laid on along the world's axes, and the festive atlas, by its texture
// coordinates, whose lights glow (COLOR_0's alpha) and twinkle (its blue, their phase; 0 steady).
const SNOW_LAYER: u32 = 17u;
const SNOW_TILE: f32 = 40.0;
const FESTIVE_LAYER: u32 = 18u;
// How bright a festive light glows by day, and how much more by night; a twinkling one's cycle
// (seconds: its painting's 16 frames at 10 a second, see tools/features/sprites.py).
const FESTIVE_GLOW: f32 = 1.3;
const FESTIVE_NIGHT: f32 = 2.6;
const TWINKLE_PERIOD: f32 = 1.6;
// Snow lying on leaves (linear): a little blue, as snow in the shade of a crown is.
const SNOW_ON_LEAVES: vec3<f32> = vec3<f32>(0.78, 0.82, 0.89);
// The twigs' layer (burnt trees' crowns), its texture's size, and how wide its finest stroke
// is in its texels: from further than about a pixel a stroke, its coverage is its mean, not cut
// at half (only the twigs' crossings passed that, as loose pixels crawling as the eye moved).
const TWIGS_LAYER: u32 = 5u;
const LEAF_TEXELS: f32 = 512.0;
const TWIG_STROKE: f32 = 6.0;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    // r: how open to the sky, g: how far the wind bends it, b: how much it flutters, a: its phase.
    @location(3) color: vec4<f32>,
    @location(4) layer: u32,
    @location(5) i_pos: vec3<f32>,
    @location(6) i_yaw: f32,
    @location(7) i_scale: vec3<f32>,
    @location(8) i_fade: f32,
    @location(9) i_leaf: vec4<f32>,
    @location(10) i_bark: vec4<f32>,
    @location(11) i_extra: vec4<f32>,
    @location(12) i_light: vec4<f32>,
    @location(13) i_lod_range: vec2<f32>,
    @location(14) i_snow: f32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) layer: u32,
    @location(4) ao: f32,
    // rgb: the colour its texture is multiplied by; a: how charred.
    @location(5) tint: vec4<f32>,
    @location(6) @interpolate(flat) glow: f32,
    @location(7) @interpolate(flat) fade: f32,
    // Height of its foot (for the occlusion near the ground).
    @location(8) @interpolate(flat) foot: f32,
    // Bark and rock: how much it shines (crystal, ice) and glows by itself (an energy node's
    // crystal); leaves: unused.
    @location(9) gloss: f32,
    @location(10) emit: f32,
    // A crystal's light within (linear): what shines through it and from an energy node's tips.
    @location(11) @interpolate(flat) light: vec3<f32>,
    @location(12) @interpolate(flat) lod_range: vec2<f32>,
    // Leaves: how much snow lies on them (a snowy fir's); a festive light: the phase it
    // twinkles at (0 steady).
    @location(13) @interpolate(flat) aux: f32,
};

struct Placed {
    world: vec3<f32>,
    normal: vec3<f32>,
};

// A vertex in the world: stretched, turned about the vertical, set on its foot, bent by the wind
// and, for leaves, fluttering.
fn place(in: VIn) -> Placed {
    let c = cos(in.i_yaw);
    let s = sin(in.i_yaw);
    let local = in.pos * in.i_scale;
    var world = in.i_pos + vec3<f32>(local.x * c + local.z * s, local.y, -local.x * s + local.z * c);
    // Lying on the ground (`extra.y`, a Mars plant's tendrils): following its slope.
    if (in.i_extra.y > 0.0) {
        world.y += (ground_height(world.xz) - ground_height(in.i_pos.xz)) * in.i_extra.y;
    }
    // (Normals under the stretch: divided by it.)
    let m = in.normal.xyz / in.i_scale;
    var n = normalize(vec3<f32>(m.x * c + m.z * s, m.y, -m.x * s + m.z * c));
    // A rock on a slope (`extra.zw`, the slope): sheared so its foot lies along it, upright
    // still. (Normals by the shear's inverse transpose.)
    let slope = in.i_extra.zw;
    if (any(slope != vec2<f32>(0.0))) {
        world.y += dot(world.xz - in.i_pos.xz, slope);
        n = normalize(vec3<f32>(n.x - slope.x * n.y, n.y, n.z - slope.y * n.y));
    }
    let sway = frame.wind.z;
    if (sway > 0.0 && in.color.g > 0.0) {
        // The vertex's bend is its share of the top's (the crown moves, the foot does not).
        let top = max(in.i_extra.x, 1.0);
        world += wind_bend(in.i_pos, top, top * sqrt(in.color.g)) * sway;
        let strength = length(frame.wind.xy);
        let t = frame.world.x;
        let phase = in.color.a * 6.2831853 + dot(in.i_pos.xz, vec2<f32>(0.071, 0.053));
        // (Not a festive decoration's: its blue is its shine, or a light's phase.)
        let flutter = select(in.color.b, 0.0, in.layer == FESTIVE_LAYER);
        world += n * sin(t * (3.5 + in.color.a * 2.5) + phase) * flutter * (0.25 + 0.7 * strength) * sway;
    }
    return Placed(world, n);
}

@vertex
fn vs_main(in: VIn) -> VOut {
    let p = place(in);
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(p.world, 1.0);
    out.world = p.world;
    out.normal = p.normal;
    out.uv = in.uv;
    out.layer = in.layer;
    out.ao = in.color.r;
    let leaf = in.layer < FIRST_OPAQUE;
    var colour = select(in.i_bark.rgb * BARK_LIGHT, in.i_leaf.rgb * LEAF_LIGHT, leaf);
    // (A crystal's own colour comes as its leaves' would: its bark's is the rock it stands on.
    // A second paint's is the leaves' too, lit as the bark is.)
    if (in.layer == CRYSTAL_LAYER) {
        colour = in.i_leaf.rgb;
    } else if (in.layer == TRIM_LAYER) {
        colour = in.i_leaf.rgb * BARK_LIGHT;
    } else if (in.layer == SNOW_LAYER || in.layer == FESTIVE_LAYER) {
        // (Their own colours: their textures' as they are.)
        colour = layer_means[in.layer].rgb;
    }
    // In the world's colour grade, as the ground round it and the painting it stands in for
    // (`world_graded` in common.wgsl): Krocark's brightened, a lava world's darkened. (Not its
    // light within, which is light, not paint.)
    colour = world_graded(colour);
    // Rock TA painted near black, its light all shade (a lava world's basalt spires), is at
    // least half as light as the map's ground on average, graded as the ground is (perspective
    // camera): its shadow side shows its form instead of a black cut-out. (Charred, it stays
    // charcoal; rock TA painted dark itself, `light.a`, an energy node's mound, stays as dark as
    // painted.)
    if (in.layer == ROCK_LAYER && frame.ground_bounce.a > 0.0) {
        let average = world_graded(frame.ground_bounce.rgb / max(frame.sun_color.rgb * frame.sun_dir.w * max(frame.sun_dir.y, 0.0) + frame.ambient.rgb, vec3<f32>(1e-3)));
        let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
        let want = 0.5 * dot(average, lum) * frame.ground_bounce.a * (1.0 - in.i_leaf.a) * (1.0 - in.i_light.a);
        colour *= clamp(want / max(dot(colour, lum), 1e-4), 1.0, 3.0);
    }
    out.tint = vec4<f32>(colour / max(layer_means[in.layer].rgb, vec3<f32>(0.002)), in.i_leaf.a);
    out.light = in.i_light.rgb;
    out.glow = in.i_bark.a;
    out.fade = in.i_fade;
    out.lod_range = in.i_lod_range;
    out.foot = in.i_pos.y;
    let festive_light = in.layer == FESTIVE_LAYER && in.color.a > 0.0;
    out.gloss = select(in.color.b, 0.0, leaf || festive_light);
    out.emit = select(in.color.a, 0.0, leaf);
    out.aux = select(select(0.0, in.color.b, festive_light), in.i_snow, leaf);
    return out;
}

@vertex
fn vs_shadow(in: VIn) -> VOut {
    var out: VOut;
    // Not faded in at all: the painting's own shadow shows alone (see the game's features.rs).
    if (in.i_fade <= 0.0) {
        out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return out;
    }
    let p = place(in);
    out.clip = frame.shadow_view_proj * vec4<f32>(p.world, 1.0);
    out.uv = in.uv;
    out.layer = in.layer;
    out.fade = in.i_fade;
    out.lod_range = in.i_lod_range;
    return out;
}

// A tree fading in casts as much of its shadow as it has taken over from its painting (whose
// own shadow fades out with it): its share of the shadow map's texels, dithered 4 x 4, which
// the shadows' filter then averages. (The whole of it at once, when half faded in, popped.)
fn shadow_faded_out(in: VOut) -> bool {
    if (in.fade >= 1.0 && in.lod_range.x == 0.0 && in.lod_range.y >= 1.0) {
        return false;
    }
    let sample = handover_dither(in.clip.xy);
    return sample < in.fade * in.lod_range.x || sample >= in.fade * in.lod_range.y;
}

// Adjacent detail levels divide the samples already owned by the model. Their union is
// exactly the model's old coverage, including while its painted sprite hands over to it.
fn detail_mask(in: VOut, coverage: f32) -> u32 {
    let start = in.fade * in.lod_range.x;
    // With one sample there is no alpha-to-coverage: reduce coverage within this level's
    // own interval. Scaling both endpoints would move the coarser mesh into the finer one's
    // samples whenever their leaf alpha/facing differs, leaving gaps and double coverage.
    let end = mix(start, in.fade * in.lod_range.y, select(1.0, coverage, SCENE_SAMPLES == 1u));
    // A fully selected level, and the finer half of a transition, own a prefix already.
    // Avoid computing and rotating a second empty mask over most of a forest's fragments.
    if (start == 0.0) {
        if (end >= 1.0) {
            return (1u << SCENE_SAMPLES) - 1u;
        }
        return handover_mask(in.clip.xy, end, SCENE_SAMPLES);
    }
    return handover_mask(in.clip.xy, end, SCENE_SAMPLES)
        & ~handover_mask(in.clip.xy, start, SCENE_SAMPLES);
}

@fragment
fn fs_shadow_opaque(in: VOut) {
    if (shadow_faded_out(in)) {
        discard;
    }
}

@fragment
fn fs_shadow_cutout(in: VOut) {
    if (shadow_faded_out(in) || textureSample(layers, layer_sampler, in.uv, in.layer).a < 0.5) {
        discard;
    }
}

// Right in front of the eye (a unit's own view among the trees) it fades out, as the painted
// sprites do, instead of filling the picture; and following a unit from behind, what stands
// between the eye and it gives way round the line of sight.
fn near_fade(world: vec3<f32>) -> f32 {
    if (frame.camera_pos.w < 0.5) {
        return 1.0;
    }
    let eye = frame.camera_pos.xyz;
    var keep = smoothstep(14.0, 22.0, distance(world, eye));
    if (sight.w > 0.0) {
        let line = sight.xyz - eye;
        let t = clamp(dot(world - eye, line) / max(dot(line, line), 1e-4), 0.0, 1.0);
        let d = distance(world, eye + line * t);
        keep *= 1.0 - (1.0 - smoothstep(sight.w * 0.55, sight.w, d)) * (1.0 - smoothstep(0.8, 0.97, t));
    }
    return keep;
}

// The lit colour of a leaf (`leaf`) or of bark and rock with normal `n`.
fn shade(in: VOut, albedo_in: vec3<f32>, n: vec3<f32>, leaf: bool) -> vec3<f32> {
    return shade_glowing(in, albedo_in, n, leaf, vec3<f32>(0.0));
}

// So, and glowing by itself by `glow` (linear: a festive light) under the haze.
fn shade_glowing(in: VOut, albedo_in: vec3<f32>, n: vec3<f32>, leaf: bool, glow: vec3<f32>) -> vec3<f32> {
    let reborn = saturate(frame.ao.w);
    let l = frame.sun_dir.xyz;
    let v = view_dir(in.world);
    var shadow = shadow_factor(in.world, n);
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }
    let ndl = dot(n, l);
    var direct = max(ndl, 0.0);
    if (leaf) {
        // Light wraps round a crown, and shines through its leaves towards the sun; leaves in the
        // shade of others still get some of it, through them.
        let wrap = mix(0.7, 0.45, reborn);
        direct = saturate((ndl + wrap) / (1.0 + wrap)) + pow(saturate(dot(-v, l)), 4.0) * mix(0.08, 0.3, reborn);
        shadow = mix(0.3, 1.0, shadow);
    }
    // Wet from the rain: darker. By moonlight colours fade, as on the ground.
    var albedo = albedo_in * (1.0 - 0.15 * frame.rain.x);
    // Every feature is already tinted to its installed painting. Enhanced Classic keeps
    // that palette exactly; Reborn slightly calms foliage pigments under physical light.
    let value = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
    albedo = mix(albedo, mix(vec3<f32>(value), albedo, 0.94), reborn * select(0.0, 1.0, leaf));
    if (frame.night.a > 0.0) {
        albedo = mix(albedo, vec3<f32>(dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722))), 0.5 * frame.night.a);
    }
    // The sky's light from above and the ground's from below, as on the units.
    let sky = frame.ambient.rgb * mix(vec3<f32>(1.0), frame.daylight.rgb, 0.6);
    let ground = frame.ambient.rgb * vec3<f32>(0.75, 0.62, 0.5);
    var ambient = mix(ground, sky, n.y * 0.5 + 0.5);
    if (frame.ground_bounce.a > 0.0) {
        ambient = mix(ambient, sky_and_ground_light(n, ground), frame.ground_bounce.a);
    }
    var ao = in.ao;
    if (frame.ao.z > 0.0) {
        ao *= near_ground_occlusion(in.world, in.world.y - in.foot, 10.0);
    }
    let sun = frame.sun_color.rgb * frame.sun_dir.w;
    // (On lava worlds the melt glows on the rock and the trees beside it, from below; on rock
    // and wood from afar too, `melt_glow`.)
    var color = albedo * (ambient * ao + sun * direct * shadow * mix(0.5, 1.0, ao) * 1.1 + point_lighting(in.world, n) + lava_on(lava_light(in.world), n) + frame.night.rgb * ao);
    if (!leaf) {
        color += albedo * melt_glow(in.world, n, in.world.y - in.foot) * ao;
    }
    // Crystal and ice shine: a sharp highlight from the sun, the sky mirrored at grazing angles.
    if (in.gloss > 0.0) {
        let h = normalize(l + v);
        let fresnel = 0.04 + 0.96 * pow(1.0 - saturate(dot(n, v)), 5.0);
        var sky_col = frame.ambient.rgb * 1.4;
        if (frame.sky.w > 0.5) {
            sky_col = mix(frame.fog.rgb, frame.sky_zenith.rgb, saturate(reflect(-v, n).y * 1.5));
        }
        // (Capped: a flat facet catching the sun lit whole, a white triangle.)
        color += (sun * min(pow(saturate(dot(n, h)), 90.0) * 2.5, 0.6) * shadow + sky_col * fresnel * 0.8 * ao) * in.gloss * mix(0.3, 1.0, reborn);
    }
    // A burning tree (see `burn`): its fire front flames and the embers in what is charred
    // glow, flickering.
    if (in.glow > 0.0) {
        let t = frame.world.x;
        let flicker = 0.55 + 0.45 * sin(t * 9.0 + in.world.y * 0.4 + in.world.x * 0.23) * sin(t * 5.3 + in.world.z * 0.31);
        let b = burn(in.world, in.glow);
        color += (vec3<f32>(2.6, 0.95, 0.22) * b.flame + vec3<f32>(2.2, 0.6, 0.1) * b.embers) * flicker;
    }
    return apply_fog(color + glow, in.world);
}

// How bright a festive light twinkling at `phase` is now (1 for a steady one, `phase` 0): once
// or twice a cycle it fades down and up again (as its painting's frames do,
// tools/features/sprites.py's `twinkle_level`).
fn twinkle(phase: f32) -> f32 {
    if (phase <= 0.0) {
        return 1.0;
    }
    let rate = 1.0 + f32(u32(phase * 1000.0) % 2u);
    let s = 0.5 + 0.5 * cos(6.2831853 * (frame.world.x / TWINKLE_PERIOD * rate + phase));
    return 0.22 + 0.78 * smoothstep(0.25, 0.75, s);
}

// Snow lying on leaves (`in.aux`, how much: a snowy fir's): where a card's face, `across` (its
// plane's normal), is turned up to the sky on the side seen, in clumps (noise over the world).
fn snowed(c: vec3<f32>, in: VOut, across: vec3<f32>) -> vec3<f32> {
    if (in.aux <= 0.0) {
        return c;
    }
    var face = across / max(length(across), 1e-12);
    if (dot(face, view_dir(in.world)) < 0.0) {
        face = -face;
    }
    let clump = fire_noise(in.world * 0.42) * 0.7 + fire_noise(in.world * 1.3) * 0.3;
    let k = in.aux * smoothstep(0.3, 0.75, face.y) * smoothstep(0.62 - 0.45 * in.aux, 0.78 - 0.45 * in.aux, clump);
    return mix(c, SNOW_ON_LEAVES, saturate(k));
}

// The colour of charred wood and leaves (`char` 0..1 of the way).
fn charred(c: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(c, vec3<f32>(0.03, 0.027, 0.025), amount);
}

// A random 0..1 for a lattice point.
fn fire_hash(p: vec3<i32>) -> f32 {
    var h = (bitcast<u32>(p.x) * 73856093u) ^ (bitcast<u32>(p.y) * 19349663u) ^ (bitcast<u32>(p.z) * 83492791u);
    h = (h ^ (h >> 13u)) * 0x5bd1e995u;
    h ^= h >> 15u;
    return f32(h & 0xFFFFu) / 65535.0;
}

// Smooth value noise in space (0..1).
fn fire_noise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = p - floor(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(fire_hash(i), fire_hash(i + vec3<i32>(1, 0, 0)), u.x);
    let x10 = mix(fire_hash(i + vec3<i32>(0, 1, 0)), fire_hash(i + vec3<i32>(1, 1, 0)), u.x);
    let x01 = mix(fire_hash(i + vec3<i32>(0, 0, 1)), fire_hash(i + vec3<i32>(1, 0, 1)), u.x);
    let x11 = mix(fire_hash(i + vec3<i32>(0, 1, 1)), fire_hash(i + vec3<i32>(1, 1, 1)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

struct Burn {
    // How charred (0..1), how much the fire front flames, how much the embers glow.
    char: f32,
    flame: f32,
    embers: f32,
};

// A tree on fire at `world` (`glow`: 0.5 as it catches, 1 as it burns out, see features.rs):
// the fire runs through it as a ragged front (a level of noise in space, drifting up as the
// flames lick upwards), behind it the leaves and bark charred black with embers glowing in
// patches, ahead of it still as they were.
fn burn(world: vec3<f32>, glow: f32) -> Burn {
    var b: Burn;
    let gone = saturate(glow * 2.0 - 1.0);
    let t = frame.world.x;
    let n = fire_noise(world * 0.07 + vec3<f32>(0.0, -t * 0.35, 0.0)) * 0.7 + fire_noise(world * 0.19 - vec3<f32>(0.0, t * 0.8, 0.0)) * 0.3;
    // (It catches at once, a third of it soon alight; burnt through as it ends.)
    let front = mix(0.3, 1.1, gone);
    b.char = smoothstep(front + 0.06, front - 0.1, n);
    b.flame = 1.0 - smoothstep(0.0, 0.07, abs(n - front));
    let speck = fire_noise(world * 0.45 + vec3<f32>(0.0, -t * 1.4, 0.0));
    b.embers = b.char * smoothstep(0.62, 0.9, speck) * (1.0 - 0.7 * gone);
    return b;
}

// How charred a burning or burnt surface is: the burn's pattern on a tree on fire, else its
// instance's (`tint.a`).
fn char_of(in: VOut) -> f32 {
    if (in.glow > 0.0) {
        return burn(in.world, in.glow).char * 0.95;
    }
    return in.tint.a;
}

// Taking over from its painting (`fade`), a model takes that share of each pixel's samples and
// the painting the rest (see `handover_mask`).
@fragment
fn fs_cutout(in: VOut) -> Covered {
    let t = textureSample(layers, layer_sampler, in.uv, in.layer);
    // (Every derivative before any discard.)
    let texels = max(length(dpdx(in.uv)), length(dpdy(in.uv))) * LEAF_TEXELS;
    let across = cross(dpdx(in.world), dpdy(in.world));
    // The leaves' outline as coverage (alpha to coverage anti-aliases it), less near the eye.
    let sharp = clamp((t.a - 0.5) / max(fwidth(t.a), 0.0001) + 0.5, 0.0, 1.0);
    // Twigs thinner than a pixel show as their mean coverage, a faint lattice.
    let soft = select(0.0, smoothstep(0.6, 1.5, texels / TWIG_STROKE), in.layer == TWIGS_LAYER && SCENE_SAMPLES > 1u);
    // A card turning edge-on fades out (its own plane's: the normals are bent to the crown's)
    // rather than breaking into dashes along its sliver.
    let facing = abs(dot(across, view_dir(in.world))) / max(length(across), 1e-12);
    let coverage = mix(sharp, saturate(t.a), soft) * smoothstep(0.02, 0.1, facing) * near_fade(in.world);
    // With MSAA off, neither alpha-to-coverage nor blending consumes the output alpha.
    // Fold coverage into the ordered sample mask so leaves and the near-eye fade still
    // cover their share of pixels instead of becoming opaque cards.
    let mask = detail_mask(in, coverage);
    if (coverage < 0.04 || mask == 0u) {
        discard;
    }
    let albedo = snowed(charred(t.rgb * in.tint.rgb, char_of(in)), in, across);
    return Covered(vec4<f32>(shade(in, albedo, normalize(in.normal), true), coverage), mask);
}

// ---- Mirrored in the water (the reflection pass, see reflection.rs) ----------------------------
// What stands above the water, as far as the model has taken over from its sprite (the two split
// the pixels between them, see `handover_dither`). One sample a pixel: leaves are cut at half
// coverage.

@fragment
fn fs_cutout_reflection(in: VOut) -> @location(0) vec4<f32> {
    // Keep derivatives defined for the whole quad, then skip texture and lighting work for
    // fragments the water cannot show (most of the forest away from a shore).
    let duv = vec4<f32>(dpdx(in.uv), dpdy(in.uv));
    let across = cross(dpdx(in.world), dpdy(in.world));
    if (!seen_in_water(in.world) || shadow_faded_out(in)) {
        discard;
    }
    let t = textureSampleGrad(layers, layer_sampler, in.uv, in.layer, duv.xy, duv.zw);
    if (t.a < 0.5) {
        discard;
    }
    return vec4<f32>(shade(in, snowed(charred(t.rgb * in.tint.rgb, char_of(in)), in, across), normalize(in.normal), true), 1.0);
}

@fragment
fn fs_opaque_reflection(in: VOut) -> @location(0) vec4<f32> {
    let duv = vec4<f32>(dpdx(in.uv), dpdy(in.uv));
    let dw = array<vec3<f32>, 2>(dpdx(in.world), dpdy(in.world));
    if (!seen_in_water(in.world) || shadow_faded_out(in)) {
        discard;
    }
    return vec4<f32>(opaque_colour_with_gradients(in, duv, dw), 1.0);
}

@fragment
fn fs_opaque(in: VOut) -> Covered {
    let coverage = near_fade(in.world);
    let mask = detail_mask(in, coverage);
    if (mask == 0u) {
        discard;
    }
    return Covered(vec4<f32>(opaque_colour(in), coverage), mask);
}

// Bark and rock lit: bark by its texture coordinates, rock laid on along the world's axes.
fn opaque_colour(in: VOut) -> vec3<f32> {
    let duv = vec4<f32>(dpdx(in.uv), dpdy(in.uv));
    let dw = array<vec3<f32>, 2>(dpdx(in.world), dpdy(in.world));
    return opaque_colour_with_gradients(in, duv, dw);
}

// Explicit gradients let reflections reject hidden fragments before any texture or shading
// work, without changing their mip selection along the water's edge.
fn opaque_colour_with_gradients(in: VOut, duv: vec4<f32>, dw: array<vec3<f32>, 2>) -> vec3<f32> {
    let n = normalize(in.normal);
    // Bark by its texture coordinates; rock along the world's axes, blended by the normal.
    var t: vec3<f32>;
    if (in.layer == ROCK_LAYER || in.layer == CRYSTAL_LAYER || in.layer == HULL_LAYER || in.layer == METAL_LAYER || in.layer == TRIM_LAYER || in.layer == SNOW_LAYER) {
        let tile = select(select(select(select(select(ROCK_TILE, CRYSTAL_TILE, in.layer == CRYSTAL_LAYER), HULL_TILE, in.layer == HULL_LAYER), METAL_TILE, in.layer == METAL_LAYER), TRIM_TILE, in.layer == TRIM_LAYER), SNOW_TILE, in.layer == SNOW_LAYER);
        let w = pow(abs(n), vec3<f32>(4.0));
        let wsum = w / (w.x + w.y + w.z);
        let p = in.world / tile;
        let gx = array<vec2<f32>, 2>(dw[0].zy / tile, dw[1].zy / tile);
        let gy = array<vec2<f32>, 2>(dw[0].xz / tile, dw[1].xz / tile);
        let gz = array<vec2<f32>, 2>(dw[0].xy / tile, dw[1].xy / tile);
        t = textureSampleGrad(layers, layer_sampler, p.zy, in.layer, gx[0], gx[1]).rgb * wsum.x
            + textureSampleGrad(layers, layer_sampler, p.xz, in.layer, gy[0], gy[1]).rgb * wsum.y
            + textureSampleGrad(layers, layer_sampler, p.xy, in.layer, gz[0], gz[1]).rgb * wsum.z;
    } else {
        t = textureSampleGrad(layers, layer_sampler, in.uv, in.layer, duv.xy, duv.zw).rgb;
    }
    if (in.layer == CRYSTAL_LAYER) {
        let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
        let stria = saturate(0.5 + (dot(t, lum) / max(dot(layer_means[CRYSTAL_LAYER].rgb, lum), 1e-3) - 1.0) * 2.5);
        return crystal(in, t * in.tint.rgb, stria, n);
    }
    let albedo = charred(t * in.tint.rgb, char_of(in));
    if (in.layer == FESTIVE_LAYER && in.emit > 0.0) {
        // A festive light: its own colour glowing, the brighter by night, twinkling at its phase.
        let glow = t * in.tint.rgb * in.emit * twinkle(in.aux) * (FESTIVE_GLOW + FESTIVE_NIGHT * frame.night.a);
        return shade_glowing(in, albedo, n, false, glow);
    }
    return shade(in, albedo, n, false);
}

// Crystal (a crystal spire's, an energy node's fingers): glassy and dark in its body as painted,
// the sky mirrored at its rims (`gloss`: how clear it is), light through it in its body's hue
// as bright as its light within (`light`), the more towards its thin tips (its occlusion rises
// to them), a sun glint broken into streaks along its striations (`stria`, 0..1) rather than
// whole facets flashing white; an energy node's gems glowing in its light's hue in place of
// their surface (`emit` 1), that light showing through its fingers (`emit` under 1: light added
// to their surface, all there is to see of them by night), pulsing slowly.
fn crystal(in: VOut, body_in: vec3<f32>, stria: f32, n_face: vec3<f32>) -> vec3<f32> {
    let reborn = saturate(frame.ao.w);
    let l = frame.sun_dir.xyz;
    let v = view_dir(in.world);
    var shadow = shadow_factor(in.world, n_face);
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }
    let across = normalize(cross(n_face, vec3<f32>(0.0, 1.0, 0.0)) + vec3<f32>(1e-4, 0.0, 0.0));
    let n = normalize(n_face + across * (stria - 0.5) * 0.18);
    let ndl = dot(n, l);
    let nv = max(dot(n, v), 1e-3);
    let fresnel = 0.05 + 0.95 * pow(1.0 - nv, 5.0);
    let body = body_in * (1.0 - 0.15 * frame.rain.x);
    let sky = frame.ambient.rgb * mix(vec3<f32>(1.0), frame.daylight.rgb, 0.6);
    let ground = frame.ambient.rgb * vec3<f32>(0.75, 0.62, 0.5);
    let ambient = mix(ground, sky, n.y * 0.5 + 0.5);
    var ao = in.ao;
    if (frame.ao.z > 0.0) {
        ao *= near_ground_occlusion(in.world, in.world.y - in.foot, 10.0);
    }
    let sun = frame.sun_color.rgb * frame.sun_dir.w;
    var color = body * (ambient * ao + sun * max(ndl, 0.0) * shadow * 0.8 + point_lighting(in.world, n) + lava_on(lava_light(in.world), n) + frame.night.rgb * ao);
    // Light through it: the sun's round its sides and from behind it, the sky's, in its hue.
    let inner = in.light;
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let hue = body / max(dot(body, lum), 1e-4) * dot(inner, lum);
    let through = sun * shadow * (0.25 * saturate((ndl + 0.6) / 1.6) + 0.6 * pow(saturate(dot(-v, l)), 3.0)) + ambient * 0.35;
    color += hue * through * mix(0.35, 1.0, smoothstep(0.6, 1.0, in.ao)) * (0.75 + 0.25 * stria) * in.gloss * (1.0 - fresnel) * mix(0.5, 1.0, reborn);
    // The sky (or the haze) mirrored at its rims.
    var env = frame.ambient.rgb * 1.4;
    if (frame.sky.w > 0.5) {
        env = mix(frame.fog.rgb, frame.sky_zenith.rgb, saturate(reflect(-v, n).y * 1.5));
    }
    color += env * fresnel * ao * mix(0.35, 0.9, in.gloss);
    // The sun glinting off it: a narrow lobe, kept under white.
    let h = normalize(l + v);
    let fh = 0.05 + 0.95 * pow(1.0 - saturate(dot(h, v)), 5.0);
    color += sun * shadow * min(pow(saturate(dot(n, h)), 160.0) * 3.0 * fh, 0.7) * mix(vec3<f32>(1.0), inner / max(max(inner.r, max(inner.g, inner.b)), 1e-3), 0.2);
    // An energy node's gems: its light's hue, a little over the bloom's threshold, the facets
    // facing the eye the brightest; and through its fingers, deeper in hue (filtered by the
    // crystal), as it grows dark (by day, beside their sunlit olive as painted, it would only
    // grey them; by night it is what shows of them).
    if (in.emit > 0.0) {
        let pulse = 0.85 + 0.15 * sin(frame.world.x * 1.7 + in.world.x * 0.05 + in.world.z * 0.03);
        let hue = inner / max(max(inner.r, max(inner.g, inner.b)), 1e-3);
        let glow = hue * 1.6 * pulse;
        let gem = smoothstep(0.8, 1.0, in.emit);
        let facet = 0.7 + 0.6 * saturate(dot(n_face, v));
        color = mix(color + glow * hue * GLOW_THROUGH * frame.night.a * in.emit, glow * facet, gem);
    }
    return apply_fog(color, in.world);
}
