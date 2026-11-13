// 3DO models (units, 3D features, debris).

struct Affine {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
};

@group(1) @binding(0) var<storage, read> pieces: array<Affine>;
@group(1) @binding(1) var textures: texture_2d_array<f32>;
@group(1) @binding(2) var tex_sampler: sampler;
// Reborn's paint and finish (normal XY, roughness, metallic) for every layer (remaster.rs).
@group(1) @binding(3) var reborn_textures: texture_2d_array<f32>;
@group(1) @binding(4) var reborn_physical: texture_2d_array<f32>;
@group(1) @binding(5) var skin_textures: texture_2d_array<f32>;
@group(1) @binding(6) var enemy_skin_textures: texture_2d_array<f32>;
// The skins' liveries (livery.wgsl): the own bank's and the enemy bank's.
@group(1) @binding(7) var<uniform> liveries: Liveries;
// The paint's second bank (model.rs `TEXTURE_BANKS`: a mod's models may name more textures than
// one array holds): its layers are numbered on from SECOND_BANK, the first bank's count (0: one
// bank, and these are stand-ins nothing samples).
@group(1) @binding(8) var textures_2: texture_2d_array<f32>;
@group(1) @binding(9) var reborn_textures_2: texture_2d_array<f32>;
@group(1) @binding(10) var reborn_physical_2: texture_2d_array<f32>;
override SECOND_BANK: u32 = 0u;

fn in_second_bank(layer: u32) -> bool {
    return SECOND_BANK != 0u && layer >= SECOND_BANK;
}

// A layer of the paint, the redrawn paint or its finish, from whichever bank holds it.
fn classic_paint(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    if (in_second_bank(layer)) {
        return textureSampleGrad(textures_2, tex_sampler, uv, layer - SECOND_BANK, dx, dy);
    }
    return textureSampleGrad(textures, tex_sampler, uv, layer, dx, dy);
}

fn reborn_colour(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    if (in_second_bank(layer)) {
        return textureSampleGrad(reborn_textures_2, tex_sampler, uv, layer - SECOND_BANK, dx, dy);
    }
    return textureSampleGrad(reborn_textures, tex_sampler, uv, layer, dx, dy);
}

fn reborn_finish(uv: vec2<f32>, layer: u32, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    if (in_second_bank(layer)) {
        return textureSampleGrad(reborn_physical_2, tex_sampler, uv, layer - SECOND_BANK, dx, dy);
    }
    return textureSampleGrad(reborn_physical, tex_sampler, uv, layer, dx, dy);
}

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) layer: u32,
    @location(4) color: vec4<f32>,
    @location(5) piece: u32,
    @location(6) matrix_base: u32,
    @location(7) team: u32,
    @location(8) flags: u32,
    @location(9) build: f32,
    // Edge wear (see model.rs `worn_edges`): distances to up to four of the face's edges,
    // and how sharp each is (+ convex, - a crease).
    @location(10) edge_dist: vec4<f32>,
    @location(11) edge_weight: vec4<f32>,
    // Battle damage, 0 (whole) to 1 (about to go).
    @location(12) damage: f32,
    // Treads (model.rs `ModelVertex::tread`): how fast u runs as the face goes forward, and
    // how far to the side it is; and the unit's travel: how far it has driven and turned.
    @location(13) tread: vec2<f32>,
    @location(14) travel: vec2<f32>,
    // Which pieces' animated textures play (model.rs `DrawInstance::playing`): bit n of x for
    // piece n, of y for piece 32 + n (the pieces past 63 share y's last bit).
    @location(15) playing: vec2<u32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    // The texture's layer (low 16 bits; 0xFFFF: none) and, for an animated one, the frame after
    // it (high 16 bits, faded into over the frame's eighth of a second).
    @location(3) @interpolate(flat) layer: u32,
    @location(4) color: vec3<f32>,
    @location(5) @interpolate(flat) flags: u32,
    @location(6) build: f32,
    // The position in the piece (model units; x and z moved along by the piece's number, so
    // each piece has its own pattern): what battle damage and the build effect are fixed to.
    @location(7) local: vec3<f32>,
    // 1 when the face's texture is a light (see model.rs).
    @location(8) @interpolate(flat) emissive: u32,
    // Colour-blind colours: the player's colour slot + 1, to tint the white team frame with
    // (0: the texture as it is).
    @location(9) @interpolate(flat) tint: u32,
    @location(10) edge_dist: vec4<f32>,
    @location(11) @interpolate(flat) edge_weight: vec4<f32>,
    @location(12) @interpolate(flat) damage: f32,
    // Authored material factors (roughness, metallic), packed into existing vertex bytes.
    @location(13) material: vec2<f32>,
    // Reborn's tracks running: how much of their fine marks and of their coarse ones shows (they
    // blur away as they pass too fast to follow).
    @location(14) @interpolate(flat) run_marks: vec2<f32>,
};

// Matches the ordinary player's colours in the game's palette.rs.
fn team_paint(slot: u32) -> vec3<f32> {
    let table = array<vec3<f32>, 10>(
        vec3<f32>(48.0, 92.0, 220.0), vec3<f32>(210.0, 40.0, 30.0),
        vec3<f32>(230.0), vec3<f32>(40.0, 170.0, 60.0),
        vec3<f32>(20.0, 40.0, 140.0), vec3<f32>(130.0, 70.0, 180.0),
        vec3<f32>(240.0, 220.0, 40.0), vec3<f32>(40.0),
        vec3<f32>(120.0, 190.0, 240.0), vec3<f32>(180.0, 170.0, 120.0)
    );
    return srgb_to_linear(table[min(slot, 9u)] / 255.0);
}

// Colour-blind-safe player colours (sRGB), in the order of TA's colour slots; the same table
// as the game's `palette::TEAM_COLORS_CB`.
fn team_cb(slot: u32) -> vec3<f32> {
    var table = array<vec3<f32>, 10>(
        vec3<f32>(0.0, 114.0, 178.0),
        vec3<f32>(230.0, 159.0, 0.0),
        vec3<f32>(240.0, 240.0, 240.0),
        vec3<f32>(0.0, 158.0, 115.0),
        vec3<f32>(86.0, 180.0, 233.0),
        vec3<f32>(204.0, 121.0, 167.0),
        vec3<f32>(240.0, 228.0, 66.0),
        vec3<f32>(35.0, 35.0, 35.0),
        vec3<f32>(213.0, 94.0, 0.0),
        vec3<f32>(150.0, 150.0, 150.0),
    );
    return srgb_to_linear(table[min(slot, 9u)] / 255.0);
}

fn xform(m: Affine, p: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m.r0, p), dot(m.r1, p), dot(m.r2, p));
}

// Whether `piece`'s animated textures play by an instance's `playing` (model.rs `piece_plays`).
fn piece_plays(playing: vec2<u32>, piece: u32) -> bool {
    let p = min(piece, 63u);
    return ((select(playing.x, playing.y, p >= 32u) >> (p & 31u)) & 1u) != 0u;
}

// A vertex's piece's matrix (its `piece`'s low 16 bits). A connector's vertex (model/connectors.rs)
// carries the piece at its other end, plus one, in the high 16 bits: hidden with that piece (its
// matrix zero), it is hidden too, rather than stretched to the world's corner.
fn piece_matrix(base: u32, piece: u32) -> Affine {
    let m = pieces[base + (piece & 0xFFFFu)];
    let other = piece >> 16u;
    if (other != 0u) {
        let o = pieces[base + other - 1u];
        if (all(o.r0 == vec4<f32>(0.0)) && all(o.r1 == vec4<f32>(0.0))) {
            return Affine(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
        }
    }
    return m;
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// `TA_SHOW_BACKS=1` (model.rs): the models drawn again, their back faces alone, in magenta,
// where they lie in front of what was drawn: every hole a 3D camera looks through into a shell
// shows. (Pushed this far from the eye, so that a face's back lying on it or a hair in front of
// it, a track's or a sheet's, stays behind it.)
override BACKS_PASS: bool = false;
const BACKS_PUSH: f32 = 0.6;

@vertex
fn vs_main(in: VIn) -> VOut {
    let m = piece_matrix(in.matrix_base, in.piece);
    let world = xform(m, vec4<f32>(in.pos, 1.0));
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(select(world, world + normalize(world - frame.camera_pos.xyz) * BACKS_PUSH, BACKS_PASS), 1.0);
    out.world = world;
    out.normal = xform(m, vec4<f32>(in.normal.xyz, 0.0));
    out.uv = in.uv;
    var layer = in.layer & 0xFFFFu;
    var next = layer;
    let frames = (in.layer >> 16u) & 0xFFu;
    out.tint = 0u;
    if (layer != 0xFFFFu && frames > 1u) {
        if (frames == 10u && frame.detail.w > 0.5) {
            // Colour-blind colours: the white team's frame (its shades of grey), tinted.
            layer += 2u;
            next = layer;
            out.tint = min(in.team, 9u) + 1u;
        } else if (frames == 10u) {
            // Team colours: the player's frame.
            layer += min(in.team, frames - 1u);
            next = layer;
        } else if (piece_plays(in.playing, in.piece & 0xFFFFu)) {
            // Animations (flashing lamps, glows): cycle at 8 frames per second, each frame fading
            // into the next (stepped, a lamp jumped between its shades: it flickered). Only where
            // the unit's script lets them play (TA's `dont-cache`); elsewhere they hold their first
            // frame (a metal maker switched off).
            let step = u32(frame.world.x * 8.0);
            next = layer + (step + 1u) % frames;
            layer += step % frames;
        }
    }
    out.layer = layer | (next << 16u);
    // Bit 0: light, bit 1: a road wheel's hub (Reborn's kit), bit 2: team paint, bit 3: moving
    // tread, bit 4: a physical finish of its own (the roughness and metallic in the vertex:
    // Reborn's plates, bolts and wheels), bit 5: one of TA's faces painted in a palette colour.
    out.emissive = ((in.layer >> 24u) & 23u) | (((in.layer >> 27u) & 1u) << 5u);
    // Bits 6/7/8: a skin's texture, its livery, and the enemy bank (model/skin.rs), changing
    // neither opacity nor geometry.
    out.emissive |= ((in.layer >> 29u) & 7u) << 6u;
    // Treads run as the unit drives, the two sides against each other as it turns (a face at
    // x goes forward by the distance less x times the turn); bit 3 wraps the texture. A road
    // wheel's hub turns instead, a radian for each radius of track its rim rolls over.
    out.run_marks = vec2<f32>(1.0);
    if (in.tread.x != 0.0) {
        let run = in.tread.x * (in.travel.x - in.tread.y * in.travel.y);
        // How many of its marks pass a second (the fine ones; the coarse ones are a fifth or a
        // sixth of them), from how fast it drives (bits 16-23 of the flags).
        let speed = f32((in.flags >> 16u) & 255u) * abs(in.tread.x);
        var passing = speed * round(in.uv.y * 255.0);
        if ((out.emissive & 2u) != 0u) {
            let c = in.uv - 0.5;
            let s = sin(run);
            let k = cos(run);
            out.uv = vec2<f32>(c.x * k - c.y * s, c.x * s + c.y * k) + 0.5;
            passing = speed * 5.0 / 6.2831853;
        } else {
            out.uv.x += run;
        }
        // (Followed up to about 12 a second, gone past 30.)
        out.run_marks = saturate(vec2<f32>(1.65, 1.65) - vec2<f32>(passing, passing * 0.2) / 18.0);
        out.emissive |= 8u;
    }
    out.color = srgb_to_linear(in.color.rgb);
    if ((out.emissive & 4u) != 0u) {
        out.color *= select(team_paint(in.team), team_cb(in.team), frame.detail.w > 0.5);
    }
    out.material = vec2<f32>(in.normal.w, in.color.a);
    out.flags = in.flags;
    out.build = in.build;
    out.local = in.pos + vec3<f32>(31.4 + f32(in.piece & 0xFFFFu) * 13.7, 0.0, 17.2 + f32(in.piece & 0xFFFFu) * 7.3);
    out.edge_dist = in.edge_dist;
    out.edge_weight = in.edge_weight;
    out.damage = in.damage;
    return out;
}

@vertex
fn vs_shadow(in: VIn) -> @builtin(position) vec4<f32> {
    // Cloaked units cast no shadow (outside the clip volume, so nothing is drawn), nor do the
    // holograms of buildings placed, planned or not yet half built (model.rs
    // `HOLOGRAM_SHADOWLESS`), nor structures remembered where they were last seen.
    if ((in.flags & 68u) != 0u || ((in.flags & 2u) != 0u && in.build < 0.5)) {
        return vec4<f32>(0.0, 0.0, -2.0, 1.0);
    }
    let m = piece_matrix(in.matrix_base, in.piece);
    let world = xform(m, vec4<f32>(in.pos, 1.0));
    return frame.shadow_view_proj * vec4<f32>(world, 1.0);
}

// A model fragment's surface: the face's texture or its palette colour, and its normal;
// with HD surfaces how worn its edge is there and the glow of battle damage.
struct Surface {
    albedo: vec3<f32>,
    normal: vec3<f32>,
    // > 0 on a convex edge (worn to bare metal), < 0 in a crease.
    wear: f32,
    // Light of its own (embers in battle damage).
    glow: vec3<f32>,
    // The paint's own occlusion (1: none); the scene's contact and ray-traced AO come on top.
    occlusion: f32,
    material: vec2<f32>,
};

// Width (model units) of the band along an edge that edge wear lightens or darkens.
const EDGE_WIDTH: f32 = 1.1;
const PI: f32 = 3.14159265;
// How deep under the water a submarine looks all over, at the least.
const SUBMERGED: f32 = 14.0;

// What a model surface mirrors in direction `r` (HD surfaces): the sky (its haze and the sun's
// glow near the horizon, the zenith above) and the ground's bounce below; blurred towards the
// plain light around as the surface gets rough.
fn model_env(r: vec3<f32>, rough: f32, ground: vec3<f32>) -> vec3<f32> {
    var sky = frame.ambient.rgb * 1.4;
    if (frame.sky.w > 0.5) {
        sky = mix(haze_color(r), frame.sky_zenith.rgb, smoothstep(0.0, 0.6, r.y));
    }
    // (The horizon blurs as the surface roughens: a painted plate shows no line of it.)
    let blur = rough * 0.6;
    let sharp = mix(ground * 0.8, sky, smoothstep(-0.12 - blur, 0.08 + blur, r.y));
    let dull = mix(ground * 0.8, frame.ambient.rgb * 1.3, 0.5 + 0.5 * r.y);
    return mix(sharp, dull, smoothstep(0.25, 0.85, rough));
}

// Edge wear: a model's convex edges catch the light like the worn paint on a painted
// miniature's edges, and its creases darken, from the distances to the face's edges (see
// model.rs). Where the band is thinner than a pixel (far from the camera) it fades away rather
// than shimmer.
fn edge_wear(dist: vec4<f32>, weight: vec4<f32>) -> f32 {
    let aa = fwidth(dist);
    let visible = saturate(vec4<f32>(EDGE_WIDTH) / max(aa * 1.5, vec4<f32>(1e-4)) - 0.5);
    let band = (1.0 - smoothstep(vec4<f32>(0.0), vec4<f32>(EDGE_WIDTH) + aa, dist)) * visible;
    return clamp(dot(band, weight), -1.0, 1.0);
}

// Rounded edges: within `width` of a convex edge the surface turns towards the face across it,
// as a machined or cast edge does, so every edge catches a thin line of light. The edges'
// distances run straight across a face, so their gradient (from the screen derivatives) points
// in from each edge. Bands thinner than a pixel fade out rather than shimmer.
fn bevelled(n: vec3<f32>, world: vec3<f32>, dist: vec4<f32>, weight: vec4<f32>, width: f32, strength: f32) -> vec3<f32> {
    let dpx = dpdx(world);
    let dpy = dpdy(world);
    let ddx = dpdx(dist);
    let ddy = dpdy(dist);
    let a = dot(dpx, dpx);
    let b = dot(dpx, dpy);
    let c = dot(dpy, dpy);
    let det = a * c - b * b;
    if (abs(det) < 1e-14) {
        return n;
    }
    let visible = saturate(vec4<f32>(width) / max(fwidth(dist) * 2.0, vec4<f32>(1e-4)) - 0.5);
    var out = n;
    for (var k = 0u; k < 4u; k++) {
        let near = 1.0 - saturate(dist[k] / width);
        if (weight[k] <= 0.0 || near <= 0.0) {
            continue;
        }
        let g = ((ddx[k] * c - ddy[k] * b) * dpx + (ddy[k] * a - ddx[k] * b) * dpy) / det;
        let len = length(g);
        if (len < 1e-5) {
            continue;
        }
        out = normalize(out - (g / len) * (near * near * strength * weight[k] * visible[k]));
    }
    return out;
}

fn hash3(p: vec3<f32>) -> f32 {
    return fract(sin(dot(p, vec3<f32>(127.1, 311.7, 74.7))) * 43758.5453);
}

// Smooth value noise in 3D (0..1).
fn value_noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash3(i), hash3(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash3(i + vec3<f32>(0.0, 1.0, 0.0)), hash3(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash3(i + vec3<f32>(0.0, 0.0, 1.0)), hash3(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash3(i + vec3<f32>(0.0, 1.0, 1.0)), hash3(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

// Battle damage: soot spreads over the hull as it loses health, in soft patches that grow from
// a noise field fixed to its pieces (so they move with them); badly hurt, it glows through thin
// cracks in the scorch. Returns how scorched the point is (0..1), and adds the embers' light
// to `glow`.
fn battle_damage(local: vec3<f32>, damage: f32, glow: ptr<function, vec3<f32>>) -> f32 {
    if (damage <= 0.02) {
        return 0.0;
    }
    // Two octaves, spread out to cover 0..1 about evenly.
    let raw = value_noise3(local * 0.05) * 0.7 + value_noise3(local * 0.17 + vec3<f32>(17.0)) * 0.3;
    let n = saturate((raw - 0.22) / 0.56);
    let reach = damage * 1.2;
    let scorch = smoothstep(1.0 - reach, 1.0 - reach + 0.28, n);
    if (damage > 0.6) {
        let t = frame.world.x;
        let crack = 1.0 - smoothstep(0.0, 0.026, abs(value_noise3(local * 0.3 + vec3<f32>(5.0, 1.0, 9.0)) - 0.5));
        let flicker = 0.75 + 0.25 * sin(t * 7.0 + local.x * 0.4 + local.z * 0.3);
        *glow += vec3<f32>(1.0, 0.3, 0.05) * 1.1 * crack * smoothstep(0.55, 1.0, scorch) * flicker * saturate((damage - 0.6) * 3.0);
    }
    return scorch;
}

// The surface at a fragment (the texture lookup takes screen-space derivatives, so before
// any discard).
fn mapped_normal(n: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, ux: vec2<f32>, uy: vec2<f32>, texel: vec3<f32>) -> vec3<f32> {
    let det = ux.x * uy.y - ux.y * uy.x;
    if (abs(det) < 1e-10) { return n; }
    let raw_t = (dx * uy.y - dy * ux.y) / det;
    let raw_b = (dy * ux.x - dx * uy.x) / det;
    let projected_t = raw_t - n * dot(n, raw_t);
    if (dot(projected_t, projected_t) < 1e-12) { return n; }
    let t = normalize(projected_t);
    let b = cross(n, t) * select(-1.0, 1.0, dot(cross(n, t), raw_b) >= 0.0);
    let mapped = texel * 2.0 - 1.0;
    return normalize(t * mapped.x + b * mapped.y + n * mapped.z);
}

// Enhanced Classic's paint magnified as TA drew it, each texel crisp, with a pixel's soft edge
// between them rather than the blur of plain filtering (a "sharp bilinear" lookup: the
// coordinate is held at the texel's middle but for its last pixel). Minified, unchanged.
fn crisp_texels(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(textureDimensions(textures).xy);
    let t = uv * size;
    // Texels per screen pixel, each way.
    let span = max(abs(dx), abs(dy)) * size;
    if (max(span.x, span.y) >= 1.0) {
        return uv;
    }
    let cell = floor(t);
    let edge = clamp((t - cell - 0.5) / max(span, vec2<f32>(1e-4)), vec2<f32>(-0.5), vec2<f32>(0.5));
    return (cell + 0.5 + edge) / size;
}

// Reborn's packed normal XY (remaster.rs). Byte 128 is exactly flat, on coarse mips and the
// fallback layer too: no lasting half-byte tilt.
fn packed_normal_texel(packed: vec2<f32>) -> vec3<f32> {
    let xy = clamp((packed * 255.0 - 128.0) / 127.0, vec2<f32>(-1.0), vec2<f32>(1.0));
    let z = sqrt(max(1.0 - dot(xy, xy), 0.001));
    return normalize(vec3<f32>(xy, z)) * 0.5 + 0.5;
}

// The marks that show Reborn's tracks running (kit.rs `track`), as a factor on the colour: round
// a track's band its links (u along the band, v the count of them round it), dark gaps between
// them and a deeper one every fifth; on a road wheel's hub (uv across it, turned as the wheel
// rolls) a cap and five lug nuts, one of them bigger. `footprint`: a pixel's size in uv (marks
// finer than a pixel fade into the plain colour); `marks`: how much of the fine marks and of the
// coarse ones shows (both blur away as they pass too fast to follow, the fine ones first).
fn kit_tread(uv: vec2<f32>, wheel: bool, footprint: f32, marks: vec2<f32>) -> f32 {
    if (wheel) {
        let p = (uv - 0.5) * 2.0;
        let w = footprint * 2.0 + 1e-4;
        let sector = 6.2831853 / 5.0;
        let k = round(atan2(p.y, p.x) / sector);
        let nut = vec2<f32>(cos(k * sector), sin(k * sector)) * 0.6;
        let size = select(0.15, 0.24, k == 0.0);
        let nuts = 1.0 - smoothstep(size - w, size + w, length(p - nut));
        let cap = 1.0 - smoothstep(0.26 - w, 0.26 + w, length(p));
        let shows = select(marks.x, marks.y, k == 0.0) * saturate(1.5 - w * 5.0);
        return 1.0 + 0.4 * cap + 0.9 * nuts * shows;
    }
    let links = round(uv.y * 255.0);
    let w = footprint * links + 1e-4;
    // (How far, in links, to the nearest gap, and to the nearest fifth.)
    let gap = 1.0 - smoothstep(0.12 - w, 0.12 + w, abs(fract(uv.x * links + 0.5) - 0.5));
    let fifth = 1.0 - smoothstep(0.1 - w * 0.2, 0.1 + w * 0.2, abs(fract(uv.x * links * 0.2 + 0.5) - 0.5));
    let shows = saturate(1.5 - w * 3.0);
    return 1.0 - 0.45 * gap * marks.x * shows - 0.35 * fifth * marks.y * saturate(1.5 - w * 0.6);
}

// A texture layer's average colour: its smallest mip.
fn texture_mean(layer: u32, reborn: bool) -> vec3<f32> {
    if (in_second_bank(layer)) {
        let l = layer - SECOND_BANK;
        if (reborn) {
            return textureSampleLevel(reborn_textures_2, tex_sampler, vec2<f32>(0.5), l, f32(textureNumLevels(reborn_textures_2) - 1u)).rgb;
        }
        return textureSampleLevel(textures_2, tex_sampler, vec2<f32>(0.5), l, f32(textureNumLevels(textures_2) - 1u)).rgb;
    }
    if (reborn) {
        return textureSampleLevel(reborn_textures, tex_sampler, vec2<f32>(0.5), layer, f32(textureNumLevels(reborn_textures) - 1u)).rgb;
    }
    return textureSampleLevel(textures, tex_sampler, vec2<f32>(0.5), layer, f32(textureNumLevels(textures) - 1u)).rgb;
}

fn model_surface(in: VOut, front: bool) -> Surface {
    var s: Surface;
    var n = normalize(in.normal);
    if (!front) {
        n = -n;
    }
    s.albedo = in.color;
    s.normal = n;
    s.wear = 0.0;
    s.glow = vec3<f32>(0.0);
    s.occlusion = 1.0;
    s.material = in.material;
    // (Derivatives of the unwrapped coordinates, so a running tread's wrap leaves no seam.)
    let duv_x = dpdx(in.uv);
    let duv_y = dpdy(in.uv);
    let position_x = dpdx(in.world);
    let position_y = dpdy(in.world);
    // (And of the place in the piece, and the edges' distances, for a livery.)
    let local_x = dpdx(in.local);
    let local_y = dpdy(in.local);
    let edge_aa = fwidth(in.edge_dist);
    let edges = edge_wear(in.edge_dist, in.edge_weight);
    let layer = in.layer & 0xFFFFu;
    let next = in.layer >> 16u;
    // (How far an animated texture has faded into its next frame.)
    let fading = select(0.0, fract(frame.world.x * 8.0), next != layer);
    // Reborn's redrawn paint in its style; but a livery laid over TA's own paint (Heritage's)
    // shows the paint as TA drew it there too.
    let painted = (in.emissive & 128u) != 0u;
    let bank = liveries.banks[select(0u, 1u, (in.emissive & 256u) != 0u)];
    let reborn_paint = frame.ao.w > 0.5 && !(painted && livery_original(bank));
    if (layer != 0xFFFFu) {
        let uv = select(in.uv, vec2<f32>(fract(in.uv.x), in.uv.y), (in.emissive & 8u) != 0u);
        if ((in.emissive & 64u) != 0u) {
            if ((in.emissive & 256u) != 0u) {
                s.albedo = textureSampleGrad(enemy_skin_textures, tex_sampler, uv, layer, duv_x, duv_y).rgb;
            } else {
                s.albedo = textureSampleGrad(skin_textures, tex_sampler, uv, layer, duv_x, duv_y).rgb;
            }
        } else if (reborn_paint) {
            var colour = reborn_colour(uv, layer, duv_x, duv_y);
            var finish = reborn_finish(uv, layer, duv_x, duv_y);
            if (fading > 0.0) {
                colour = mix(colour, reborn_colour(uv, next, duv_x, duv_y), fading);
                finish = mix(finish, reborn_finish(uv, next, duv_x, duv_y), fading);
            }
            s.albedo = colour.rgb;
            s.occlusion *= colour.a;
            s.material = finish.ba;
            s.normal = mapped_normal(n, position_x, position_y, duv_x, duv_y, packed_normal_texel(finish.rg));
        } else {
            let texel = crisp_texels(uv, duv_x, duv_y);
            s.albedo = classic_paint(texel, layer, duv_x, duv_y).rgb;
            if (fading > 0.0) {
                s.albedo = mix(s.albedo, classic_paint(texel, next, duv_x, duv_y).rgb, fading);
            }
        }
    } else if ((in.emissive & 8u) != 0u) {
        // Reborn's tracks running: the band's links, a road wheel's lug nuts.
        s.albedo *= kit_tread(in.uv, (in.emissive & 2u) != 0u, max(length(duv_x), length(duv_y)), in.run_marks);
    }
    // Rounded edges: Reborn's broad and bright, Enhanced Classic's a fine line (and so on TA's own
    // paint under a livery: Heritage's units are TA's as Enhanced Classic draws them).
    let reborn = reborn_paint;
    // TA's palette colours were chosen for its own light, often near white for the lit side of
    // a shaded part (a dish's back, a dome): under Reborn's sun they would blow out to blank
    // white. Their brighter shades come down, the darker ones hardly.
    // (A face's paint before its mottle: what a livery reads its light and dark against.)
    var mean = s.albedo;
    if ((in.emissive & 32u) != 0u) {
        if (reborn) {
            s.albedo /= 1.0 + 0.6 * dot(s.albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        }
        mean = s.albedo;
        // They are painted metal as much as the textured faces round them: a faint mottle of
        // wear and weather (fixed to the piece) keeps them from reading as unpainted blanks
        // (Enhanced Classic's half as strong: TA's flat colour, worn a little as its edges are).
        let mottle = value_noise3(in.local * 0.22) * 0.65 + value_noise3(in.local * 0.9 + vec3<f32>(7.1)) * 0.35;
        s.albedo *= 1.0 + (mottle - 0.5) * select(0.08, 0.16, reborn);
    }
    // A skin's livery (livery.wgsl, model/skin.rs): its paint over the face's, trim along the
    // convex edges, worn through to the metal and dirtied as much as it says.
    var worn = 1.0;
    if (painted) {
        let l = bank;
        if (layer != 0xFFFFu) {
            mean = texture_mean(layer, reborn_paint);
        }
        let footprint = max(length(local_x), length(local_y));
        let trim = select(0.0, livery_trim(in.edge_dist, in.edge_weight, edge_aa), l.trim.w > 0.5);
        s.albedo = livery_paint(l, s.albedo, mean, in.local, cross(local_x, local_y), footprint, trim);
        s.material = livery_material(l, s.material);
        let bare = livery_chips(l, in.local, in.edge_dist, in.edge_weight, edge_aa);
        // (Bare steel, brushed bright.)
        s.albedo = mix(s.albedo, vec3<f32>(0.46, 0.45, 0.43) * (0.8 + 0.4 * livery_noise(in.local * 1.7)), bare);
        s.material = mix(s.material, vec2<f32>(0.36, 0.6), bare);
        let dirt = livery_grime(l, in.local, saturate(-edges), saturate(-n.y));
        s.albedo = mix(s.albedo, s.albedo * vec3<f32>(0.45, 0.41, 0.36) + vec3<f32>(0.010, 0.009, 0.007), dirt);
        s.material = vec2<f32>(mix(s.material.x, 0.92, dirt * 0.6), s.material.y * (1.0 - dirt * 0.5));
        // (Reborn's own wear below it, as much as the livery's up to Reborn's.)
        worn = min(l.weather.x * 2.0, 1.0);
    }
    s.normal = bevelled(s.normal, in.world, in.edge_dist, in.edge_weight, select(0.28, 0.5, reborn), select(0.7, 1.1, reborn));
    if (in.tint > 0u) {
        // The white frame's brightest shade (about 230 of 255) becomes the player's colour.
        s.albedo = s.albedo / 0.79 * team_cb(in.tint - 1u);
    }
    // Art direction is independent of camera and the terrain's HD-surface preference.
    // Enhanced Classic keeps the original paint; Reborn exposes more of the physical finish.
    let style = select(0.38, 1.0, reborn);
    let wear = edges * style * worn;
    s.wear = wear;
    s.albedo = s.albedo * (1.0 + 0.36 * max(wear, 0.0)) + vec3<f32>(0.018) * max(wear, 0.0);
    s.albedo *= 1.0 + 0.55 * min(wear, 0.0);
    if (in.damage > 0.02) {
        var glow = vec3<f32>(0.0);
        let scorch = battle_damage(in.local, in.damage, &glow);
        s.albedo = mix(s.albedo, s.albedo * 0.28 + vec3<f32>(0.014, 0.011, 0.008), scorch * 0.88);
        s.wear *= 1.0 - scorch;
        s.glow += glow;
    }
    return s;
}

// A model under construction (`in.build` of it done): the nano-lathe builds it up from the
// ground, unevenly (the front wanders with a noise fixed to its pieces). What is built is the
// model as lit (`lit`); the front being lathed glows in the stream's colour `tint`; what is not
// built yet is a dim hologram of itself: its form still shaded, a lattice of glowing lines and
// a fresnel rim in that colour, faint scan lines drifting up it.
// `fw`: the screen derivatives (fwidth) of the piece position and of the height.
fn under_construction(lit: vec3<f32>, in: VOut, n: vec3<f32>, v: vec3<f32>, tint: vec3<f32>, scan: f32, fw: vec4<f32>) -> vec3<f32> {
    let above = in.world.y - ground_height(in.world.xz);
    let threshold = 0.02 + 0.95 * (saturate(above / 56.0) * 0.75 + value_noise3(in.local * 0.09) * 0.25);
    let width = max(fw.w / 56.0 * 0.75 + length(fw.xyz) * 0.09 * 0.25, 0.004);
    let done = smoothstep(threshold - width, threshold + width, in.build);
    let front = 1.0 - smoothstep(0.0, 0.025 + width, abs(in.build - threshold));
    // The lattice: where the surface crosses planes 6 model units apart along each axis.
    let q = in.local / 6.0;
    let d = abs(fract(q + 0.5) - 0.5);
    let w = max(fw.xyz / 6.0 * 1.2, vec3<f32>(1e-4));
    let lines = 1.0 - smoothstep(vec3<f32>(0.0), w, d);
    let lattice = max(max(lines.x, lines.y), lines.z);
    let rim = pow(1.0 - saturate(abs(dot(n, v))), 3.0);
    let form = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
    let hologram = tint * (0.05 + form * 0.35 + rim * 0.55 + lattice * 0.7 + scan * 0.06);
    return mix(hologram, lit, done) + tint * front * 2.2;
}

// How much of a cloaked unit (only its own side sees it) is drawn: a third or so, more towards
// its edges so the silhouette reads, rippling like heat haze. A building's plan is a steadier
// hologram, half there.
fn cloak_share(in: VOut, n: vec3<f32>) -> f32 {
    let v = normalize(frame.camera_pos.xyz - in.world);
    let edge = pow(1.0 - max(dot(n, v), 0.0), 2.0);
    if ((in.flags & 2u) != 0u) {
        return 0.42 + 0.3 * edge;
    }
    let ripple = sin(in.local.y * 0.35 + frame.world.x * 5.0 + in.world.x * 0.05) * 0.5 + 0.5;
    return 0.22 + 0.35 * edge + 0.1 * ripple;
}

// (Its samples: all of them, but a cloaked unit's or a planned building's share, see
// `handover_mask`: multisampled, a translucent veil rather than a screen door.)
@fragment
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> Covered {
    // (A portrait of a unit standing on land: nothing of it under the ground, model.rs
    // `FLAG_ABOVE_GROUND`.)
    if ((in.flags & 128u) != 0u && in.world.y < 0.0) {
        discard;
    }
    let surface = model_surface(in, front);
    var out: Covered;
    out.color = shade_model(in, surface, in.flags);
    out.mask = 0xFFFFFFFFu;
    if ((in.flags & 4u) != 0u) {
        out.mask = handover_mask(in.clip.xy, cloak_share(in, surface.normal), SCENE_SAMPLES);
        if (out.mask == 0u) {
            discard;
        }
    }
    // (A structure remembered, a little less there than one that is.)
    if ((in.flags & 64u) != 0u) {
        out.mask = handover_mask(in.clip.xy, 0.86, SCENE_SAMPLES);
        if (out.mask == 0u) {
            discard;
        }
    }
    return out;
}

@fragment
fn fs_backs() -> @location(0) vec4<f32> {
    return vec4<f32>(8.0, 0.0, 8.0, 1.0);
}

// Models mirrored in the water (the reflection pass, see reflection.rs): only what stands above
// it where the water can show it, and without the selection highlight.
@fragment
fn fs_reflection(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let surface = model_surface(in, front);
    if (!seen_in_water(in.world) || ((in.flags & 4u) != 0u && handover_mask(in.clip.xy, cloak_share(in, surface.normal), 1u) == 0u)) {
        discard;
    }
    return shade_model(in, surface, in.flags & ~1u);
}

// A Blinn-Phong exponent widened as the specular anti-aliasing widens a GGX highlight: the
// exponent's roughness (alpha squared about 2 / (power + 2)) grown by `widen`.
fn widened_power(power: f32, widen: f32) -> f32 {
    return max(2.0 / (2.0 / (power + 2.0) + widen) - 2.0, 1.0);
}

// Lit colour of a model fragment; `flags` = the instance's flags (see model.rs).
fn shade_model(in: VOut, surface: Surface, flags: u32) -> vec4<f32> {
    // (Derivatives for the build effect, taken while every pixel runs alike.)
    let fw = vec4<f32>(fwidth(in.local), fwidth(in.world.y));
    let style = select(0.30, 1.0, frame.ao.w > 0.5);
    // Wet from the rain: darker, and shinier (below).
    let wet = frame.rain.x;
    let albedo = surface.albedo * (1.0 - 0.18 * wet);
    let v = normalize(frame.camera_pos.xyz - in.world);
    // A normal the paint's relief or a bevel turns away from the eye (a face seen edge-on) is
    // bent back to just face it: turned away, its highlight and its Fresnel ran to their
    // brightest along the outline, a pixel here and there.
    let n = normalize(surface.normal + v * max(0.05 - dot(surface.normal, v), 0.0));
    // Specular anti-aliasing (geometric, after Kaplanyan and Tokuyoshi): where the normal turns
    // within a pixel (the paint's panel lines and plate rims, a bevelled edge, a bolt's facets,
    // a small part's curve) a highlight narrower than the pixel lit it in some frames and missed
    // it in others as the unit or the camera moved, and edges and rivets glittered. Each
    // highlight is widened by as much as the normal varies over the pixel (squared roughness
    // grows by it), so it spreads to what the pixel can show, dimmer but steady.
    let dndx = dpdx(n);
    let dndy = dpdy(n);
    let widen = min(0.5 * (dot(dndx, dndx) + dot(dndy, dndy)), 0.18);
    let l = frame.sun_dir.xyz;
    let ndl = max(dot(n, l), 0.0);
    // Ray traced where the depth prepass saw this very surface (raytrace.rs).
    let rt = rt_visibility(in.clip);
    var shadow = 1.0;
    if (rt.traced) {
        shadow = min(min(mix(1.0, rt.sun, frame.screen.w), rt_feature_shadow(in.world, n)), terrain_sunlight(in.world));
    } else {
        shadow = shadow_factor(in.world, n);
    }
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }

    // Hemisphere ambient: cool sky from above, warm bounce from the ground. It takes on the
    // colour of the light on the painted ground (dusk's warmth, the moon's blue; nothing by
    // TA's own light), so models stand in the same light as the ground around them.
    let sky = frame.ambient.rgb * mix(vec3<f32>(1.0), frame.daylight.rgb, 0.6);
    var ground = frame.ambient.rgb * vec3<f32>(0.75, 0.62, 0.5);
    var ambient = mix(ground, sky, n.y * 0.5 + 0.5);
    if (frame.ground_bounce.a > 0.0) {
        ambient = mix(ambient, sky_and_ground_light(n, ground), frame.ground_bounce.a);
    }
    // On lava worlds the melt glows on the hull from below (and metal mirrors it).
    let lava = lava_light(in.world);
    ambient += lava_on(lava, n);
    ground += lava;

    // Metal-ish Blinn-Phong highlight, strongest on bright (painted metal) texels and on edges
    // worn to bare metal.
    let h = normalize(l + v);
    let spec_mask = 0.25 + 0.75 * smoothstep(0.1, 0.6, dot(albedo, vec3<f32>(0.333))) + 0.8 * max(surface.wear, 0.0);
    let spec = pow(max(dot(n, h), 0.0), widened_power(48.0, widen)) * 0.55 * spec_mask * (1.0 + 1.5 * wet);
    // Rim light keeps silhouettes readable against busy terrain.
    let rim = pow(1.0 - max(dot(n, v), 0.0), 4.0) * 0.25;

    let sun = frame.sun_color.rgb * frame.sun_dir.w;
    // Dynamic point lights (explosions, muzzle flashes, lasers): diffuse plus a metal glint.
    var pl = point_lights_eval(in.world, n, v, widened_power(32.0, widen));
    pl.diffuse *= rt.point;
    pl.specular *= rt.point;
    // Near the ground, whatever stands there (the model's own footprint included) hides part
    // of the sky: the ambient light fades, direct light a little (perspective camera).
    var ao = rt.ao * surface.occlusion;
    if (frame.ao.z > 0.0) {
        ao *= near_ground_occlusion(in.world, in.world.y - ground_height(in.world.xz), 8.0);
    }
    let direct = 0.5 + 0.5 * ao;
    var color = albedo * (ambient * ao + (sun * ndl * shadow * 1.1 + pl.diffuse) * direct)
        + (sun * spec * shadow + pl.specular * spec_mask * 0.5) * direct + ambient * rim * ao;
    // HD surfaces: bright painted metal mirrors the sky above and the ground below (Fresnel:
    // most at grazing angles).
    if (style > 0.0) {
        let r = reflect(-v, n);
        var sky_col = frame.ambient.rgb * 1.4;
        if (frame.sky.w > 0.5) {
            sky_col = mix(frame.fog.rgb, frame.sky_zenith.rgb, saturate(r.y * 1.5));
        }
        let env = mix(ground * 0.8, sky_col, smoothstep(-0.25, 0.2, r.y));
        let fresnel = 0.04 + 0.96 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
        color += env * spec_mask * fresnel * 0.6 * style * mix(0.5, 1.0, shadow) * ao;
    }
    // HD surfaces: physically based light. What the surface is made of is read from its paint:
    // pale, neutral texels and edges worn bright are bare metal (it mirrors in its own colour),
    // coloured ones paint under a lacquer, near-black ones rubber and shadowed detail. GGX
    // highlights with Smith visibility and Schlick's Fresnel; the sky and the ground mirrored,
    // blurred by roughness.
    if (style > 0.0) {
        let lum = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        let hi_c = max(max(albedo.r, albedo.g), albedo.b);
        let chroma = (hi_c - min(min(albedo.r, albedo.g), albedo.b)) / max(hi_c, 1e-3);
        let physical = (in.emissive & 16u) != 0u || (frame.ao.w > 0.5 && (in.layer & 0xFFFFu) != 0xFFFFu);
        let metal = select(saturate(smoothstep(0.2, 0.48, lum) * (1.0 - smoothstep(0.08, 0.3, chroma)) + max(surface.wear, 0.0) * 0.9), saturate(surface.material.y + max(surface.wear, 0.0) * 0.35), physical);
        let dark = 1.0 - smoothstep(0.02, 0.1, lum);
        let rough = clamp(select(mix(mix(0.5, 0.27, metal), 0.85, dark), surface.material.x, physical) * (1.0 - 0.45 * wet), 0.08, 1.0);
        let f0 = mix(vec3<f32>(0.04), albedo * 1.15, metal);
        let diffuse = albedo * (1.0 - 0.72 * metal);
        let a2 = min(rough * rough * rough * rough + widen, 1.0);
        let a = sqrt(a2);
        let nh = saturate(dot(n, h));
        let nv = max(dot(n, v), 1e-3);
        let vh = saturate(dot(v, h));
        let dd = nh * nh * (a2 - 1.0) + 1.0;
        let ggx = a2 / (PI * dd * dd);
        let k = a * 0.5;
        let vis = 0.25 / ((nv * (1.0 - k) + k) * (ndl * (1.0 - k) + k));
        let fres = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
        // (Capped well below the brightest a pinpoint could reach: the anti-aliasing above
        // widens what a pixel cannot show, and what it can show needs no more to read as a
        // glint; brighter, a texel's sparkle bloomed into a blinking halo.)
        let highlight = min(ggx * vis, 30.0) * fres * ndl;
        let env_f = f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - nv, 5.0);
        var env = model_env(reflect(-v, n), sqrt(a), ground);
        // Ray traced: the units and buildings around mirrored in smooth metal and paint (one
        // ray a pixel: on rougher surfaces, which would blur them, it would only sparkle).
        let mirrored = rt_reflection(in.clip);
        env = mix(env, mirrored.rgb, mirrored.a * (1.0 - smoothstep(0.15, 0.5, rough)));
        // A satin lacquer over the paint: a second, faint highlight (military paint, not a
        // toy's gloss).
        let coat = (1.0 - metal) * (1.0 - dark) * 0.08;
        let ca2 = 0.006;
        let cd = nh * nh * (ca2 - 1.0) + 1.0;
        let clear = min(ca2 / (PI * cd * cd), 120.0) * 0.25 * (0.04 + 0.96 * pow(1.0 - vh, 5.0)) * ndl * coat;
        var pbr = diffuse * ambient * ao
            + (sun * (diffuse * ndl * 1.1 + highlight + clear) * shadow + pl.diffuse * diffuse + pl.specular * (0.3 + 0.7 * metal)) * direct
            + env * env_f * mix(0.45, 1.0, metal) * ao * mix(0.55, 1.0, shadow)
            + ambient * rim * ao * 0.6;
        color = mix(color, pbr, style);
    }
    // A film of rain water mirrors the sky on every surface.
    if (wet > 0.0) {
        let r = reflect(-v, n);
        let sky_col = select(frame.fog.rgb, mix(frame.fog.rgb, frame.sky_zenith.rgb, saturate(r.y * 1.5)), frame.sky.w > 0.5);
        let fresnel = 0.03 + 0.97 * pow(1.0 - max(dot(n, v), 0.0), 5.0);
        color += sky_col * fresnel * 0.5 * wet * ao;
    }
    // Embers of battle damage glow by themselves.
    color += surface.glow;

    // Hit: a hot flash on the hull, brightest along the silhouette (bits 8-15 of the flags).
    let hit = f32((flags >> 8u) & 0xFFu) / 255.0;
    if (hit > 0.0) {
        color += vec3<f32>(1.0, 0.6, 0.3) * hit * (0.18 + rim * 6.0);
    }
    // Paralysed: electric blue crawling over the hull in bands, along the silhouette most, and
    // fine arcs running across it.
    if ((flags & 32u) != 0u) {
        let t = frame.world.x;
        let crawl = sin(in.local.y * 0.9 + in.local.x * 0.6 - t * 7.0) * sin(in.local.z * 0.8 - in.local.y * 0.3 + t * 4.5);
        let k = 0.3 + 0.7 * crawl * crawl;
        let arc = pow(1.0 - abs(sin(in.local.x * 0.45 + in.local.z * 0.35 + sin(in.local.y * 0.7 + t * 3.0) * 1.5 + t * 9.0)), 28.0);
        color += vec3<f32>(0.3, 0.62, 1.0) * k * (0.35 + rim * 6.0) + vec3<f32>(0.55, 0.8, 1.0) * arc * 1.6;
    }
    // Lights (lamps, beacons, glows) shine by themselves: their bright texels as bright as their
    // paint whatever light falls on them, and a little over in HDR for the bloom, their pattern
    // kept (not washed out to white by the sun and the glow added up). Only while the unit
    // works: switched off, unpowered or stunned, they are paint (bits 24-31 of the flags, how far
    // they are out, model.rs `FLAG_LIGHTS_OUT_SHIFT`).
    if ((in.emissive & 1u) != 0u) {
        let lum = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        let shines = smoothstep(0.08, 0.45, lum) * (1.0 - f32(flags >> 24u) / 255.0);
        color = mix(color, albedo, 0.7 * shines) + albedo * shines * mix(0.35, 0.25, style);
    }
    if ((flags & 2u) != 0u) {
        // Under construction the nano-lathe builds the model up behind a glowing front, what is
        // not built yet a hologram of itself (see `under_construction`). A placement ghost or a
        // plan is all hologram, its shape read in its own light: faces turned to the sun
        // brighter, those away dimmer, its outline bright; and fine scan lines, each a pixel's
        // soft line, climb it.
        let t = frame.world.x;
        var tint = vec3<f32>(0.2, 1.0, 0.35);
        if ((flags & 8u) != 0u) {
            tint = vec3<f32>(1.0, 0.15, 0.1);
        }
        // Colour-blind marks: blue where it may go, orange where not.
        if (frame.detail.w > 0.5) {
            tint = select(vec3<f32>(0.25, 0.65, 1.0), vec3<f32>(1.0, 0.5, 0.0), (flags & 8u) != 0u);
        }
        let rise = in.local.y * 0.15 - t * 1.5;
        let width = max(fwidth(rise), 1e-4);
        let scan = 1.0 - smoothstep(0.04, 0.04 + width * 1.5, abs(fract(rise + 0.5) - 0.5));
        if (in.build > 0.0) {
            color = under_construction(color, in, n, v, tint, scan, fw);
        } else {
            let form = 0.3 + 0.55 * max(dot(n, l), 0.0) + 0.15 * (n.y * 0.5 + 0.5);
            let outline = pow(1.0 - max(dot(n, v), 0.0), 2.5);
            color = tint * (form * 0.75 + outline * 0.9 + scan * 0.45 * saturate(1.0 - width * 4.0)) * 1.3;
        }
    }
    // Where it stands in a sea of water (a hull, a pontoon, a wading unit's legs): wet and darker
    // for a unit or two above the waterline, and a ragged line of foam lapping at it, rising and
    // falling a little with the swell (the collar of foam on the water round it is in water.wgsl).
    let lap = in.world.y - frame.world.y;
    if (frame.water.w > 0.5 && frame.water_extra.y < 0.5 && (flags & 22u) == 0u && lap > -0.6 && lap < 2.5 && ground_height(in.world.xz) < frame.world.y - 0.5) {
        let t = frame.world.x;
        let swell = 0.3 * sin(t * 1.6 + dot(in.world.xz, vec2<f32>(0.19, 0.13))) + 0.5 * (value_noise3(vec3<f32>(in.world.xz * 0.3, t * 0.45)) - 0.5);
        let rise = lap - swell;
        color *= 1.0 - 0.3 * (1.0 - smoothstep(0.0, 2.0, rise));
        // (Broken into flecks and gaps, a few tenths of a unit tall where it is thickest; from far
        // off as thin as the pixel can show it, and fainter.)
        let gaps = value_noise3(vec3<f32>(in.world.xz * 0.45, t * 0.5));
        let fleck = value_noise3(vec3<f32>(in.world.xz * 1.4 + 7.0, t * 1.1));
        let width = mix(0.08, 0.35, smoothstep(0.3, 0.8, gaps));
        let line = (1.0 - smoothstep(width * 0.4, width + fw.w, abs(rise - 0.05))) * min(1.0, 0.5 / (fw.w + 0.1)) * smoothstep(0.45, 0.75, gaps) * mix(0.35, 1.0, fleck);
        let foam = vec3<f32>(0.84, 0.87, 0.89) * (ambient * ao + sun * saturate(l.y) * mix(0.35, 1.0, shadow));
        color = mix(color, foam, line * 0.55 * min(frame.water.y, 1.0));
    }
    // Below the surface of a sea of water a model sinks into the water's colour with depth:
    // hulls under their waterline; submarines (flag 16) all of them, running as dim shapes as
    // in TA. (Over TA's painted sea beds the water surface itself stays mostly clear, so the
    // model does it.)
    var below = frame.world.y - in.world.y;
    if ((flags & 16u) != 0u) {
        below = max(below, SUBMERGED);
    }
    if (frame.water.w > 0.5 && frame.water_extra.y < 0.5 && below > 0.0) {
        // (A submarine sinks into it as in TA; anything else on the sea bed stays readable, its
        // shape and its player's colour, at most two thirds lost however deep. The water takes
        // a share at once, though: a hull a unit or two under its waterline is already in it.)
        var murk = max((1.0 - exp(-below * 0.045)) * 0.66, 0.2 * (1.0 - exp(-below * 0.8)));
        if ((flags & 16u) != 0u) {
            murk = (1.0 - exp(-below * 0.085)) * 0.9;
        }
        let deep = frame.water_color.rgb * frame.ambient.rgb * 0.7;
        color = mix(color, deep, murk);
    }
    if ((flags & 1u) != 0u) {
        // Selected / hovered: gentle team-neutral highlight on the silhouette.
        color += vec3<f32>(0.12, 0.16, 0.12) * (0.5 + rim * 4.0);
    }
    if ((flags & 64u) != 0u) {
        // A structure the viewer saw and no longer sees, remembered where it stood: a cool grey
        // memory of it rather than the thing itself (the fog of war dims it further), a faint scan
        // climbing it as a hologram's does.
        let grey = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
        color = mix(color, vec3<f32>(grey) * vec3<f32>(0.8, 0.94, 1.14), 0.4) * 0.92;
        color *= 1.0 - 0.07 * (0.5 + 0.5 * sin(in.world.y * 0.8 - frame.world.x * 1.2));
    }
    if ((flags & 4u) != 0u) {
        // Cloaked (only its own side sees it), and a building's plan: drawn in part (see
        // `cloak_share`), in a cool shimmer brightest at its edges so the silhouette still reads.
        let edge = pow(1.0 - max(dot(n, v), 0.0), 2.0);
        color = mix(color * 0.5, vec3<f32>(0.55, 0.8, 1.2), 0.35 + 0.4 * edge);
    }
    return vec4<f32>(apply_fog(color, in.world), 1.0);
}
