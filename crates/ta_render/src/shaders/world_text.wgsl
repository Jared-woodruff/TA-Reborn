// Text and signs in the world (see world_text.rs): glyphs of a signed distance atlas (and one
// picture) laid on the ground, draped over its relief as a decal is, or standing free in the air
// (banners, signs). Drawn with the opaque scene right after the terrain, under the decals (the
// scorch marks of the blasts that fall on them), the units and the trees: what stands in front of
// a letter hides it, units' shadows fall across it, and it is lit, hazed and fogged as the ground
// it lies on.
//
// Then a second, lighter pass (`fs_text_over`), once the opaque scene is drawn: the glyphs again
// where something stands in front of them, as much as that something stands on the letter's own
// ground (a unit walking over a name), so the name still reads through it; a ridge or a building
// far in front hides it as before. Still the letter of the ground, lit as the ground under it is.
//
// Every glyph is a quad (`GRID` x `GRID` cells when it drapes, so the relief bends it) whose
// middle, edge vectors, atlas cell and look the game gives it (the style, how hot it glows, how
// far it has been revealed...): see `GlyphInstance`.

struct VIn {
    @builtin(vertex_index) vi: u32,
    // xyz: the quad's middle; w: the style.
    @location(0) centre: vec4<f32>,
    // xyz: from the middle to the middle of the right edge; w: 1 to drape over the ground (the
    // middle's y is then a lift over it), 0 to stand free.
    @location(1) right: vec4<f32>,
    // xyz: from the middle to the middle of the top edge; w: world units per atlas texel.
    @location(2) up: vec4<f32>,
    @location(3) uv: vec4<f32>,
    @location(4) color: vec4<f32>,
    // x: heat, y: reveal, z: age (s), w: seed.
    @location(5) state: vec4<f32>,
    // rgb: light of its own; a: the least light its paint shows in.
    @location(6) glow: vec4<f32>,
    // Cloth: x amplitude, y wave number, z speed, w phase.
    @location(7) wave: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // 0..1 across the quad (x to the right, y up).
    @location(1) local: vec2<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) style: f32,
    @location(4) @interpolate(flat) color: vec4<f32>,
    @location(5) @interpolate(flat) state: vec4<f32>,
    @location(6) @interpolate(flat) glow: vec4<f32>,
    // World units per texel, and the world position of the middle.
    @location(7) @interpolate(flat) texel: f32,
};

@group(1) @binding(0) var glyphs: texture_2d<f32>;
@group(1) @binding(1) var pictures: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;

// Cells a draped quad is cut into along each side, and the texels of an atlas cell's spread (the
// distance field covers this many texels either side of an outline: world_text.rs's SPREAD).
const GRID: u32 = 6u;
const SPREAD: f32 = 8.0;

const STYLE_PAINT: f32 = 0.0;
const STYLE_LIGHT: f32 = 1.0;
const STYLE_LATTICE: f32 = 2.0;
const STYLE_PICTURE: f32 = 3.0;
const STYLE_CLOTH: f32 = 4.0;

fn corner(vi: u32) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    return corners[vi % 6u];
}

@vertex
fn vs_text(in: VIn) -> VOut {
    var out: VOut;
    let cell = in.vi / 6u;
    let drape = in.right.w > 0.5;
    if (!drape && cell > 0u) {
        // (A free quad is one cell: the rest are left empty.)
        out.clip = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    var local = corner(in.vi);
    if (drape) {
        local = (vec2<f32>(f32(cell % GRID), f32(cell / GRID)) + local) / f32(GRID);
    }
    let across = local * 2.0 - 1.0;
    var world = in.centre.xyz + in.right.xyz * across.x + in.up.xyz * across.y;
    if (drape) {
        // Lying a little over the ground (or over the sea, where the ground is under it).
        world.y = max(ground_height(world.xz), frame.world.y) + in.centre.y;
    } else if (in.wave.x != 0.0) {
        // Cloth: waving along its length, more towards its free end (local.x = 1), and a little
        // up and down; the wave is along the quad's own normal.
        let normal = normalize(cross(in.right.xyz, in.up.xyz));
        let phase = local.x * in.wave.y - frame.world.x * in.wave.z + in.wave.w;
        let sway = sin(phase) * in.wave.x * smoothstep(0.0, 0.35, local.x);
        world += normal * sway + vec3<f32>(0.0, 1.0, 0.0) * sin(phase * 0.7 + 1.3) * in.wave.x * 0.18 * local.x;
    }
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = mix(in.uv.xy, in.uv.zw, vec2<f32>(local.x, 1.0 - local.y));
    out.local = local;
    out.world = world;
    out.style = in.centre.w;
    out.color = in.color;
    out.state = in.state;
    out.glow = in.glow;
    out.texel = in.up.w;
    return out;
}

// ---- Small things ---------------------------------------------------------------------------

fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q += dot(q, q.yx + 33.33);
    return fract((q.x + q.y) * q.x);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

// What glows at `heat` (0 cold, 1 white hot): deep red, orange, yellow, white; HDR.
fn ember(heat: f32) -> vec3<f32> {
    let h = max(heat, 0.0);
    let r = smoothstep(0.0, 0.22, h);
    let g = smoothstep(0.18, 0.7, h);
    let b = smoothstep(0.62, 1.15, h);
    return vec3<f32>(r * 1.7, g * 1.15, b * 0.95) * (0.2 + h * 3.2) * smoothstep(0.0, 0.05, h);
}

// The slope of the ground at `p` (for lighting what lies on it).
fn ground_normal(p: vec2<f32>) -> vec3<f32> {
    let e = 6.0;
    let dx = ground_height(p + vec2<f32>(e, 0.0)) - ground_height(p - vec2<f32>(e, 0.0));
    let dz = ground_height(p + vec2<f32>(0.0, e)) - ground_height(p - vec2<f32>(0.0, e));
    return normalize(vec3<f32>(-dx, 2.0 * e, -dz));
}

// The light a surface lying in the world takes, as a decal takes it (see `fs_decal` in
// sprite.wgsl): the sun's or moon's through the shadows of the units and the relief and the
// clouds, the sky's fill in the shade, the night's, the blasts' and lamps'; no less than `least`.
fn surface_light(world: vec3<f32>, n: vec3<f32>, clip: vec4<f32>, least: f32) -> vec3<f32> {
    let rt = rt_visibility(clip);
    var shadow = 1.0;
    if (rt.traced) {
        shadow = min(min(mix(1.0, rt.sun, frame.screen.w), rt_feature_shadow(world, n)), terrain_sunlight(world));
    } else {
        shadow = shadow_factor(world, n);
    }
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(world);
    }
    let sun = mix(shade_fill(), vec3<f32>(1.0), shadow) * frame.sun_dir.w * frame.daylight.rgb;
    let ao = contact_occlusion(world.xz) * rt.ao;
    let light = (sun + frame.night.rgb) * ao + point_lighting(world, n) * rt.point;
    return max(light, vec3<f32>(least));
}

// Into the distance haze, as the ground under it.
fn hazed(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    return mix(c, fog_color_at(world), fog_factor(world));
}

// A glyph at its pixel, premultiplied (the pass blends it over what is drawn: one and one less
// its alpha): its body over its dark halo, or for projected light the light it adds and the
// shade under it. Called first thing, while every pixel runs alike (derivatives need it).
fn glyph_look(in: VOut, front: bool) -> vec4<f32> {
    // The sampling first, while every pixel runs alike (derivatives need it).
    let field = textureSample(glyphs, atlas_sampler, in.uv).r;
    let picture = textureSample(pictures, atlas_sampler, in.uv);
    // Texels from the outline (+ inside), its change over a pixel for the soft edge.
    let inside = (field - 0.5) * 2.0 * SPREAD;
    let soft = max(fwidth(inside), 1e-3);
    let cover = clamp(inside / soft * 0.5 + 0.5, 0.0, 1.0);
    let heat = in.state.x;
    let reveal = in.state.y;
    let age = in.state.z;
    let seed = in.state.w;
    let opacity = in.color.a;

    // (The face of what it lies on, taken while every pixel runs alike; the ground's slope for
    // what lies on it.)
    let face = normalize(cross(dpdx(in.world), dpdy(in.world)));
    var n = select(-face, face, front);
    if (in.style == STYLE_PAINT || in.style == STYLE_LATTICE || in.style == STYLE_PICTURE) {
        n = ground_normal(in.world.xz);
    }

    // Revealed from the left by a bright front crossing the glyph, as a scan does.
    let front_at = reveal * 1.25 - 0.12;
    let shown = smoothstep(front_at, front_at - 0.12, in.local.x);
    let scan = exp(-abs(in.local.x - front_at) * 14.0) * step(0.001, reveal) * step(reveal, 0.999);

    var body = vec3<f32>(0.0);
    var emit = vec3<f32>(0.0);
    var alpha = 0.0;
    var additive = 0.0;
    var halo_alpha = 0.0;

    if (in.style == STYLE_PAINT) {
        // Paint on the ground: worn a little, lit as the ground is, glowing where it is hot;
        // dark round its edge so it reads over any ground.
        let wear = mix(0.8, 1.0, value_noise(in.world.xz * 0.45 + seed * 17.0));
        let albedo = world_graded(in.color.rgb * wear);
        let light = surface_light(in.world, n, in.clip, in.glow.a);
        // Hot: embers along the edge and through the middle where it is cracked.
        let rim = exp(-max(inside, 0.0) * 0.35);
        let crack = smoothstep(0.35, 0.8, value_noise(in.world.xz * 0.22 + seed * 31.0 + age * 0.15));
        let heated = ember(heat) * (0.4 + 0.6 * crack + 1.0 * rim) * 0.8;
        body = albedo * light + in.glow.rgb + heated * cover;
        alpha = cover * shown;
        halo_alpha = (1.0 - smoothstep(0.0, SPREAD * 0.85, -inside)) * (1.0 - cover) * 0.6 * shown;
        // The scan's front, as it crosses.
        emit = vec3<f32>(1.0, 0.95, 0.8) * scan * cover * 2.4;
    } else if (in.style == STYLE_LIGHT) {
        // Projected light: a bright line along the edge, a fainter fill, a glow round it, a
        // scan's lines drifting over it; added to the ground, a little shade under to read by.
        let line = exp(-abs(inside) * 0.8);
        let fill = cover * 0.75;
        // (The glow dies away before the cell's edge: no outline of the quad.)
        let glow_out = exp(-max(-inside, 0.0) * 0.35) * (1.0 - smoothstep(4.0, 8.0, -inside)) * (1.0 - cover) * 0.5;
        let lines = 0.82 + 0.18 * sin(in.world.z * 0.9 - age * 6.0);
        // (Its flicker is in its colour, world_text.rs: steady with reduced flashes.)
        emit = in.color.rgb * (line * 2.2 + fill + glow_out) * lines * shown;
        emit += vec3<f32>(1.0, 0.97, 0.85) * scan * (cover + line) * 3.0;
        additive = 1.0;
        halo_alpha = (1.0 - smoothstep(0.0, SPREAD * 0.9, -inside)) * 0.42 * shown;
        alpha = 0.0;
    } else if (in.style == STYLE_LATTICE) {
        // A nanolathe's work: the part built so far (`reveal` up the glyph) solid metal, above it
        // the lattice of its frame, green, and a bright line where the streams reach.
        let up = in.local.y;
        let built = smoothstep(reveal, reveal - 0.05, up * 1.0);
        let lattice_grid = abs(fract((in.world.x + in.world.z) / 7.0 + age * 0.15) - 0.5) * 2.0;
        let crossing = abs(fract((in.world.x - in.world.z) / 7.0) - 0.5) * 2.0;
        let wires = max(smoothstep(0.82, 0.95, lattice_grid), smoothstep(0.82, 0.95, crossing));
        let albedo = world_graded(in.color.rgb);
        let light = surface_light(in.world, n, in.clip, in.glow.a);
        let rim = exp(-abs(inside) * 0.7);
        let nano = vec3<f32>(0.25, 1.0, 0.55);
        let frame_look = nano * (wires * 0.9 + rim * 1.4) * 0.8;
        let solid = albedo * light + in.glow.rgb;
        body = mix(frame_look, solid, built);
        let front_line = exp(-abs(up - reveal) * 18.0) * step(0.001, reveal) * step(reveal, 0.999);
        emit = nano * front_line * cover * 3.2;
        // (The frame is thin: its wires and rim only; the built part is the whole glyph.)
        alpha = cover * mix(clamp(wires * 0.55 + rim * 0.8, 0.0, 1.0), 1.0, built);
        halo_alpha = (1.0 - smoothstep(0.0, SPREAD * 0.85, -inside)) * (1.0 - cover) * 0.5;
    } else if (in.style == STYLE_PICTURE) {
        // A picture on the ground (the emblem), lit as the ground is, glowing as it is told; while
        // a nanolathe builds it (`reveal` under 1), as the letters are: solid up to the line the
        // streams reach, the green lattice of its frame beyond (where the picture is).
        let light = surface_light(in.world, n, in.clip, in.glow.a);
        let albedo = world_graded(picture.rgb);
        let solid = (albedo * light + in.glow.rgb) * (1.0 + heat * 0.8);
        let up = in.local.y;
        let built = select(smoothstep(reveal, reveal - 0.05, up), 1.0, reveal >= 0.999);
        let lattice_grid = abs(fract((in.world.x + in.world.z) / 7.0 + age * 0.15) - 0.5) * 2.0;
        let crossing = abs(fract((in.world.x - in.world.z) / 7.0) - 0.5) * 2.0;
        let wires = max(smoothstep(0.82, 0.95, lattice_grid), smoothstep(0.82, 0.95, crossing));
        let nano = vec3<f32>(0.25, 1.0, 0.55);
        body = mix(nano * (wires * 0.9 + 0.3) * 0.8, solid, built);
        let front_line = exp(-abs(up - reveal) * 18.0) * step(0.001, reveal) * step(reveal, 0.999);
        alpha = picture.a * mix(clamp(wires * 0.55 + 0.2, 0.0, 1.0), 1.0, built);
        emit = ember(heat) * picture.a * 0.5 + nano * front_line * picture.a * 3.2;
        halo_alpha = 0.0;
    } else {
        // Cloth, printed: lit from both sides, waving.
        let light = surface_light(in.world, n, in.clip, in.glow.a);
        let albedo = world_graded(in.color.rgb);
        let back = select(0.78, 1.0, front);
        body = albedo * light * back + in.glow.rgb;
        alpha = cover;
        halo_alpha = 0.0;
    }
    // Contrast: never less than the picture has to read by; the haze takes all of it far off.
    var colour = hazed(body, in.world);
    // A light of its own dims into the haze (less than what it lights), with none of the haze's
    // colour added: the ground under a projected light is fogged already, as a painted glyph's
    // body is, and the clear margin round a glyph in its quad stays clear.
    emit = emit * (1.0 - fog_factor(in.world) * 0.5);
    let o = clamp(opacity, 0.0, 1.0);
    let a_body = alpha * o;
    let a_halo = halo_alpha * o;
    let a = clamp(a_body + (1.0 - a_body) * a_halo, 0.0, 1.0);
    if (additive > 0.5) {
        // Light: added over the ground (alpha is what it shades).
        return vec4<f32>(emit * o, a_halo);
    }
    // Premultiplied: the body over its dark halo.
    return vec4<f32>(colour * a_body + emit * a_body, a);
}

// Whether a premultiplied colour adds or hides next to nothing (its pixel is left alone).
fn negligible(c: vec4<f32>) -> bool {
    return c.a < 0.003 && max(c.r, max(c.g, c.b)) < 0.003;
}

@fragment
fn fs_text(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = glyph_look(in, front);
    if (negligible(c)) {
        discard;
    }
    return c;
}

// ---- Over what stands on it ------------------------------------------------------------------
//
// The second pass, after the opaque scene (world_text.rs, `draw_over`): its depth test lets a
// glyph through only where something was drawn in front of it (a test the other way round from
// the first pass's, with the same bias: never where the first pass drew it). There it finds what
// that is from the scene's depth: the ground itself (a ridge in front, a bump of the relief over
// the letter) is left alone; what stands over the ground at its own place is drawn over as far
// as it stands on the letter's ground, near it and not far above it (a unit on the name): fully
// up to a kbot's or a tank's height over the letter, fading out by a tall unit's; a building or
// a hill far in front, an aircraft high above, never.

// The scene's depth, every sample (`texture_depth_2d` without antialiasing: world_text.rs).
@group(2) @binding(0) var scene_depth: texture_depth_multisampled_2d;

// How much of the letter is drawn over what stands on it (its body, halo and light alike).
const OVER: f32 = 0.62;
// What stands on the letter: over its ground by no more than this (world units), fully...
const OVER_NEAR: f32 = 34.0;
// ...and not at all from this height up.
const OVER_FAR: f32 = 60.0;
// Over its own ground by this much what is in front is not the ground itself (the ground map
// the letters are draped by and the mesh the ground is drawn with part by a unit or two).
const STANDING_FROM: f32 = 2.0;
const STANDING_FULL: f32 = 4.5;

// The nearest of a pixel's samples in the scene's depth (on an edge, what stands in front).
fn nearest_scene_depth(px: vec2<i32>) -> f32 {
    var z = textureLoad(scene_depth, px, 0);
    for (var i = 1u; i < SCENE_SAMPLES; i++) {
        z = min(z, textureLoad(scene_depth, px, i));
    }
    return z;
}

// The world position the scene shows at screen position `px` (pixels), depth `z`.
fn scene_point(px: vec2<f32>, z: f32) -> vec3<f32> {
    let ndc = vec2<f32>(px.x / frame.screen.x * 2.0 - 1.0, 1.0 - px.y / frame.screen.y * 2.0);
    let h = frame.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return h.xyz / h.w;
}

// How much a glyph at `world`, seen at pixel `px`, shows through what is in front of it there
// (0..1; see above).
fn over_share(px: vec2<f32>, world: vec3<f32>) -> f32 {
    let p = scene_point(px, nearest_scene_depth(vec2<i32>(px)));
    let standing = p.y - max(ground_height(p.xz), frame.world.y);
    let above = p.y - world.y;
    return smoothstep(STANDING_FROM, STANDING_FULL, standing) * (1.0 - smoothstep(OVER_NEAR, OVER_FAR, above));
}

@fragment
fn fs_text_over(in: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = glyph_look(in, front) * (OVER * over_share(in.clip.xy, in.world));
    if (negligible(c)) {
        discard;
    }
    return c;
}
