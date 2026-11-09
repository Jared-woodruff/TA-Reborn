// GAF sprites: feature billboards, ground decals (metal patches, painted shadows, scorch
// marks) and additive effect sprites (explosions).
//
// One sprite pixel is one world unit. Billboards face the camera using the frame's billboard
// basis, which for the classic projection is chosen so a billboard reproduces TA's 2D sprite
// exactly; decals lie flat on the ground at the anchor height (again exact in classic view;
// the perspective camera drapes them over the ground, see `vs_decal`).

struct SpriteFrame {
    // xy = uv min, zw = uv max
    uv: vec4<f32>,
    // xy = size in pixels, zw = origin (anchor pixel)
    geom: vec4<f32>,
};

@group(1) @binding(0) var<storage, read> frames: array<SpriteFrame>;
@group(1) @binding(1) var atlas: texture_2d<f32>;
@group(1) @binding(2) var atlas_sampler: sampler;

struct VIn {
    @builtin(vertex_index) vi: u32,
    @location(0) pos: vec3<f32>,
    @location(1) frame_index: u32,
    // x = alpha, y = scale, z = brightness (HDR multiplier for effects), w = rotation
    @location(2) params: vec4<f32>,
    // rgb tint; a = for additive effects, how much of their light is only light (1: plain light,
    // less: the sprite hides some of what lies behind it; see `additive_color`)
    @location(3) tint: vec4<f32>,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) params: vec4<f32>,
    @location(3) tint: vec4<f32>,
    // Height of the sprite's anchor (a billboard's foot on the ground).
    @location(4) @interpolate(flat) anchor_y: f32,
    // The sprite's size (world units: how far an effect fades out over the ground), and its
    // frame's corners in the atlas (uv: min xy, max zw; a magnified effect keeps to them).
    @location(5) @interpolate(flat) extent: f32,
    @location(6) @interpolate(flat) rect: vec4<f32>,
};

fn corner(vi: u32) -> vec2<f32> {
    // Two triangles: (0,0) (1,0) (1,1) / (0,0) (1,1) (0,1)
    let c = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    return c[vi % 6u];
}

@vertex
fn vs_billboard(in: VIn) -> VOut {
    return billboard_vertex(in, true);
}

@vertex
fn vs_effect(in: VIn) -> VOut {
    return billboard_vertex(in, false);
}

fn billboard_vertex(in: VIn, standing: bool) -> VOut {
    let f = frames[in.frame_index];
    let c = corner(in.vi);
    let px = (c * f.geom.xy - f.geom.zw) * in.params.y; // pixels relative to the anchor
    let rot = in.params.w;
    let cr = cos(rot);
    let sr = sin(rot);
    let p = vec2<f32>(px.x * cr - px.y * sr, px.x * sr + px.y * cr);
    // Screen-down pixels move against the billboard up vector.
    var world = in.pos + frame.billboard_right.xyz * p.x - frame.billboard_up.xyz * p.y;
    let sway = (1.0 - in.tint.a) * frame.wind.z;
    if (standing && sway > 0.0) {
        world += wind_bend(in.pos, max(f.geom.w * in.params.y, 1.0), -p.y) * sway;
    }
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    // Flatten the sprite to its anchor's depth (nudged towards the camera): TA's sprites are
    // painted images, so parts drawn below the anchor must not sink into the ground, while
    // terrain and units in front of the anchor still occlude correctly.
    // Sprites extending far below their anchor (rocks) are ground seen further towards the
    // camera, so take depth from that front-most ground point.
    let below = max(f.geom.y - f.geom.w, 0.0) * in.params.y;
    let ground_down = normalize(vec3<f32>(-frame.billboard_up.x, 0.0, -frame.billboard_up.z) + vec3<f32>(0.0, 0.0, 1e-5));
    let base = in.pos + ground_down * below;
    let to_cam = normalize(frame.camera_pos.xyz - base);
    // What is painted above the anchor stands up off the ground: as near the eye as an upright
    // tree's crown as high as the card shows it would be (nearer by the eye's elevation), so the
    // ground rising behind a tree's foot (a slope facing the eye, a cliff it stands under) does
    // not cut its crown off along a line. (The card's height in the world for its height on it,
    // over the upright's: the card's up and the eye's way as the world's up shares in them.)
    let up = frame.billboard_up.xyz;
    let lift = clamp(to_cam.y / max(normalize(up).y, 0.25), 0.0, 4.0);
    let rise = max(-p.y, 0.0) * length(up) * lift;
    let anchor = frame.view_proj * vec4<f32>(base + to_cam * (6.0 + rise), 1.0);
    out.clip.z = anchor.z / anchor.w * out.clip.w;
    out.uv = mix(f.uv.xy, f.uv.zw, c);
    out.world = world;
    out.params = in.params;
    out.tint = in.tint;
    out.anchor_y = in.pos.y;
    out.extent = max(f.geom.x, f.geom.y) * in.params.y;
    out.rect = f.uv;
    return out;
}

@vertex
fn vs_decal(in: VIn) -> VOut {
    let f = frames[in.frame_index];
    // Under the perspective camera a decal is draped over the ground (as many of its 4 x 4
    // cells as its size needs: one for a footprint, all for a big scorch mark), every corner as
    // high over the ground as the anchor is over the ground under it: laid flat at the anchor's
    // height it hung in the air over a slope's downhill side (seen low, a dark slit across the
    // hillside) and sank into its uphill side. The classic camera keeps TA's flat decal (one
    // cell; the others are left empty).
    let perspective = frame.camera_pos.w > 0.5;
    let extent = max(f.geom.x, f.geom.y) * in.params.y;
    let cells = select(1u, u32(clamp(ceil(extent / 24.0), 1.0, 4.0)), perspective);
    let cell = vec2<u32>((in.vi / 6u) % 4u, in.vi / 24u);
    var out: VOut;
    if (any(cell >= vec2<u32>(cells))) {
        // (A cell it does not need: nothing drawn.)
        out.clip = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    let c = (vec2<f32>(cell) + corner(in.vi)) / f32(cells);
    let px = (c * f.geom.xy - f.geom.zw) * in.params.y;
    // Rotation about the vertical: texture v runs along (-sin r, cos r) on the ground.
    let rot = in.params.w;
    let cr = cos(rot);
    let sr = sin(rot);
    let p = vec2<f32>(px.x * cr - px.y * sr, px.x * sr + px.y * cr);
    var world = in.pos + vec3<f32>(p.x, 0.0, p.y);
    if (perspective) {
        world.y += ground_height(world.xz) - ground_height(in.pos.xz);
    }
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    if (!perspective) {
        // (Flat at its anchor's height, where the ground under it rises above that, a painted
        // shadow beside a lava spire on a bump, it was cut off along the heightmap's cells, a
        // ruler-straight edge and corner: its depth is taken nearer the eye, so it lies over the
        // ground as TA drew it.)
        let near = frame.view_proj * vec4<f32>(world + normalize(frame.camera_pos.xyz - world) * CLASSIC_DECAL_LIFT, 1.0);
        out.clip.z = near.z / near.w * out.clip.w;
    }
    out.uv = mix(f.uv.xy, f.uv.zw, c);
    out.world = world;
    out.params = in.params;
    out.tint = in.tint;
    out.anchor_y = in.pos.y;
    out.extent = extent;
    out.rect = f.uv;
    return out;
}

// How far nearer the eye than where it lies a decal's depth is taken in the classic camera (see
// `vs_decal`): more than the ground's bumps under one rise.
const CLASSIC_DECAL_LIFT: f32 = 24.0;

// Features painted lying (cars, pipes, barriers, ruins; see `painted_lying` in sprites.rs) under
// the perspective camera: TA painted them from its camera high above, round their footprints,
// so their paintings are laid on the ground, north up, where TA showed them, as a decal is; and
// draped over the ground (8 x 8 cells, every corner as high over the ground as the anchor is
// over the ground under it).
@vertex
fn vs_lying(in: VIn) -> VOut {
    let f = frames[in.frame_index];
    let cell = in.vi / 6u;
    let c = (vec2<f32>(f32(cell % 8u), f32(cell / 8u)) + corner(in.vi)) / 8.0;
    let px = (c * f.geom.xy - f.geom.zw) * in.params.y;
    var world = in.pos + vec3<f32>(px.x, 0.0, px.y);
    world.y += ground_height(world.xz) - ground_height(in.pos.xz);
    var out: VOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.uv = mix(f.uv.xy, f.uv.zw, c);
    out.world = world;
    out.params = in.params;
    out.tint = vec4<f32>(in.tint.rgb, 1.0);
    out.anchor_y = in.pos.y;
    out.rect = f.uv;
    return out;
}

// Shrunk sprites (several of their texels to a pixel: the map editor's overview, the far zoom).
// The atlas has no mips, so one tap a pixel would show every few texels and drop the rest: thin
// lines broke into dots and specks, crawling as the view moved. Instead `n` bilinear taps
// across the pixel's footprint in the frame (`dx`, `dy`: the uv a pixel spans, so the ground's
// slant is followed), averaged, their alpha weighting the colour (the transparent texels round
// a painting carry no dark fringe in) and kept to the frame's `rect` (a neighbour's texels
// never bleed in: off the frame is transparent). Up to `FOOTPRINT_TAPS` a side; at 1:1 (TA's
// classic camera) a single tap, as before.
const FOOTPRINT_TAPS: f32 = 4.0;

fn footprint_taps(dx: vec2<f32>, dy: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(textureDimensions(atlas, 0));
    return clamp(ceil(vec2<f32>(length(dx * size), length(dy * size)) - 0.25), vec2<f32>(1.0), vec2<f32>(FOOTPRINT_TAPS));
}

fn footprint(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, rect: vec4<f32>, n: vec2<f32>) -> vec4<f32> {
    let half = 0.5 / vec2<f32>(textureDimensions(atlas, 0));
    var rgb = vec3<f32>(0.0);
    var a = 0.0;
    for (var j = 0; j < i32(n.y); j++) {
        for (var i = 0; i < i32(n.x); i++) {
            let o = (vec2<f32>(f32(i), f32(j)) + 0.5) / n - 0.5;
            let p = uv + dx * o.x + dy * o.y;
            if (all(p >= rect.xy) && all(p <= rect.zw)) {
                let s = textureSampleLevel(atlas, atlas_sampler, clamp(p, rect.xy + half, rect.zw - half), 0.0);
                rgb += s.rgb * s.a;
                a += s.a;
            }
        }
    }
    return vec4<f32>(rgb / max(a, 1e-4), a / (n.x * n.y));
}

// A magnified sprite's colour at `uv` (fewer than one of its texels to a pixel: a tree or rock
// next to a unit's own eyes): TA's dithered painting, enlarged, would show its pixels as blots.
// A cubic through the four by four texels round the sample instead, a sharp one (Catmull-Rom,
// held to the middle texels' range against ringing) mixed with a smoothing one (B-spline), as
// the ground's HD surfaces do.
fn hd_sprite(uv: vec2<f32>, rect: vec4<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(atlas, 0));
    let p = uv * size - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    let c2 = f * f;
    let c3 = c2 * f;
    let cx = vec4<f32>(-c3.x + 2.0 * c2.x - f.x, 3.0 * c3.x - 5.0 * c2.x + 2.0, -3.0 * c3.x + 4.0 * c2.x + f.x, c3.x - c2.x) * 0.5;
    let cy = vec4<f32>(-c3.y + 2.0 * c2.y - f.y, 3.0 * c3.y - 5.0 * c2.y + 2.0, -3.0 * c3.y + 4.0 * c2.y + f.y, c3.y - c2.y) * 0.5;
    let u = 1.0 - f;
    let bx = vec4<f32>(u.x * u.x * u.x, 3.0 * c3.x - 6.0 * c2.x + 4.0, -3.0 * c3.x + 3.0 * c2.x + 3.0 * f.x + 1.0, c3.x) / 6.0;
    let by = vec4<f32>(u.y * u.y * u.y, 3.0 * c3.y - 6.0 * c2.y + 4.0, -3.0 * c3.y + 3.0 * c2.y + 3.0 * f.y + 1.0, c3.y) / 6.0;
    // A cubic reads two texels either side, but atlas padding is only one texel. Clamp to
    // this painting's texels so a neighbouring frame cannot colour its magnified outline.
    let lo_texel = vec2<i32>(floor(rect.xy * size + 0.5));
    let hi_texel = vec2<i32>(floor(rect.zw * size + 0.5)) - 1;
    var sharp = vec3<f32>(0.0);
    var soft = vec3<f32>(0.0);
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var around = vec3<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let q = clamp(base + vec2<i32>(x - 1, y - 1), lo_texel, hi_texel);
            let t = textureLoad(atlas, q, 0).rgb;
            sharp += t * (cx[x] * cy[y]);
            soft += t * (bx[x] * by[y]);
            around += t;
            if (x == 1 || x == 2) && (y == 1 || y == 2) {
                lo = min(lo, t);
                hi = max(hi, t);
            }
        }
    }
    let c = mix(clamp(sharp, lo, hi), soft, 0.45);
    // TA's palette dither scatters single texels of other hues (blue, yellow and red flecks in a
    // grey) through the colours it mixes; enlarged, they would be blots of colour. Their light
    // stays, and their hue goes most of the way to that of the texels round them.
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let mean = around / 16.0;
    let calm = max(vec3<f32>(dot(c, lum)) + mean - vec3<f32>(dot(mean, lum)), vec3<f32>(0.0));
    return mix(c, calm, 0.8);
}

// Opaque-ish billboards: alpha tested so they write depth and need no sorting. Their
// alpha is the distance to the outline (0.5 on it; see `SpriteAtlas::add_gaf_outlined`):
// a pixel wide ramp across it becomes the coverage, which with multisampling
// (alpha to coverage) anti-aliases the edge.
@fragment
fn fs_cutout(in: VOut) -> Covered {
    var t = textureSample(atlas, atlas_sampler, in.uv);
    // Shrunk, the texels the pixel covers (see `footprint`).
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let taps = footprint_taps(dx, dy);
    if (taps.x * taps.y > 1.0) {
        t = footprint(in.uv, dx, dy, in.rect, taps);
    }
    var coverage = clamp((t.a - 0.5) / max(fwidth(t.a), 0.0001) + 0.5, 0.0, 1.0);
    // Magnified (see `hd_sprite`): how many of its texels a pixel spans.
    let texels = fwidth(in.uv * vec2<f32>(textureDimensions(atlas, 0)));
    let magnified = smoothstep(0.7, 0.3, max(texels.x, texels.y));
    // Right in front of the eye (a unit's own view among the trees), a painted sprite would
    // fill the picture: it fades away within a couple of dozen units of the eye (through the
    // coverage, smoothly where the picture is multisampled; else it goes at the last).
    if (frame.camera_pos.w > 0.5) {
        coverage *= smoothstep(12.0, 46.0, distance(in.world, frame.camera_pos.xyz));
    }
    // Handing over to the 3D tree or rock standing in for it (`SpriteInstance::shown`): the
    // samples the model has not taken (see `handover_mask`).
    let mask = ((1u << SCENE_SAMPLES) - 1u) & ~handover_mask(in.clip.xy, 1.0 - in.params.z, SCENE_SAMPLES);
    if (coverage <= 0.0 || mask == 0u) {
        discard;
    }
    if (magnified > 0.0) {
        t = vec4<f32>(mix(t.rgb, hd_sprite(in.uv, in.rect), magnified), t.a);
    }
    var c = cutout_color(in, t);
    c.a = coverage;
    return Covered(c, mask);
}

// A billboard's lit colour for its atlas texel `t`.
fn cutout_color(in: VOut, t: vec4<f32>) -> vec4<f32> {
    // Sprites carry TA's painted lighting; apply dynamic shadowing lightly for integration,
    // and let nearby point lights (fires, explosions) light them up.
    var shadow = shadow_factor(in.world, vec3<f32>(0.0, 1.0, 0.0));
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }
    var light = mix(shade_fill() * 1.3, vec3<f32>(1.0), shadow) * frame.sun_dir.w * frame.daylight.rgb + frame.night.rgb + point_lighting_omni(in.world) * 0.8 + lava_light(in.world) * 0.6;
    // The foot of a tree or rock darkens where it meets the ground (perspective camera).
    if (frame.ao.z > 0.0) {
        light *= near_ground_occlusion(in.world, in.world.y - in.anchor_y, 10.0);
    }
    // (In the world's colour grade, as the ground and its 3D stand-in, see `world_graded`;
    // darker when wet from the rain.)
    var albedo = world_graded(t.rgb * in.tint.rgb) * (1.0 - 0.15 * frame.rain.x);
    // By moonlight colours fade, as on the ground (see daylight.rs).
    if (frame.night.a > 0.0) {
        albedo = mix(albedo, vec3<f32>(dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722))), 0.5 * frame.night.a);
    }
    let lit = albedo * light;
    return vec4<f32>(apply_fog(lit, in.world), 1.0);
}

// Translucent decals (premultiplied alpha): painted shadows, scorch marks, metal patches.
@fragment
fn fs_decal(in: VOut) -> @location(0) vec4<f32> {
    var t = textureSample(atlas, atlas_sampler, in.uv);
    // Shrunk, the texels the pixel covers (see `footprint`).
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let taps = footprint_taps(dx, dy);
    let shrunk = taps.x * taps.y > 1.0;
    if (shrunk) {
        t = footprint(in.uv, dx, dy, in.rect, taps);
    }
    // Magnified, TA's paintings as the billboards' (see `hd_sprite`).
    let texels = fwidth(in.uv * vec2<f32>(textureDimensions(atlas, 0)));
    let magnified = smoothstep(0.7, 0.3, max(texels.x, texels.y));
    if (magnified > 0.0) {
        t = vec4<f32>(mix(t.rgb, hd_sprite(in.uv, in.rect), magnified), t.a);
    }
    // (The face of the ground under it, taken while every pixel runs alike.)
    let face = cross(dpdx(in.world), dpdy(in.world));
    var a = t.a * in.params.x;
    var c: vec3<f32>;
    if (frame.camera_pos.w > 0.5) {
        // (Where the draping cannot follow the ground, a big decal over rough relief, what
        // hangs in the air or sinks into the ground fades away.)
        a *= 1.0 - smoothstep(2.5, 6.0, abs(in.world.y - ground_height(in.world.xz)));
        // Seen edge on (ground falling away beyond a crest), a painting of a texel to a world
        // unit is all stretched texels, a smear of its palette across the ground, whose own
        // detail holds up: it fades as the ground turns from the eye.
        a *= smoothstep(0.02, 0.12, abs(dot(normalize(face), view_dir(in.world))));
        if (a <= 0.004) {
            discard;
        }
        // (Perspective camera.) TA's palette dither in the painting (a metal patch's pastel
        // flecks): its light kept, its hue most of the way to the mean round it. (Shrunk, the
        // footprint has averaged it already.)
        let texel = 1.0 / vec2<f32>(textureDimensions(atlas, 0));
        var around = vec4<f32>(t.rgb * t.a, t.a);
        if (!shrunk) {
            around = vec4<f32>(0.0);
            for (var k = 0; k < 4; k++) {
                let o = vec2<f32>(select(-0.75, 0.75, (k & 1) == 1), select(-0.75, 0.75, k >= 2));
                let s = textureSampleLevel(atlas, atlas_sampler, in.uv + o * texel, 0.0);
                around += vec4<f32>(s.rgb * s.a, s.a);
            }
        }
        let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
        let calm = around.rgb / max(around.a, 1e-4);
        var albedo = t.rgb * in.tint.rgb;
        albedo = mix(albedo, calm * in.tint.rgb * clamp(dot(t.rgb, lum) / max(dot(calm, lum), 1e-4), 0.4, 2.0), 0.7);
        // A metal deposit's patch is ore in the ground beside its 3D crystals: steel grey, its
        // pastel palette tints and its brightest painted sheen toned down (`1 - tint.a`: how
        // rich it is; the colour stays the classic camera's).
        if (in.tint.a < 1.0) {
            let l = dot(albedo, lum);
            albedo = mix(albedo, vec3<f32>(l) * vec3<f32>(0.9, 0.95, 1.05), 0.6) * mix(1.0, 0.72, smoothstep(0.25, 0.7, l));
        }
        // In the world's colour grade, as the ground it lies on (see `world_graded`).
        albedo = world_graded(albedo);
        // Lit as the ground it lies on: the units' and buildings' shadows (ray traced or the
        // shadow map), the relief's and the clouds', the sky's fill in the shade; darker under
        // and between the things standing on it; in the haze.
        let up = vec3<f32>(0.0, 1.0, 0.0);
        let rt = rt_visibility(in.clip);
        var shadow = 1.0;
        if (rt.traced) {
            shadow = min(min(mix(1.0, rt.sun, frame.screen.w), rt_feature_shadow(in.world, up)), terrain_sunlight(in.world));
        } else {
            shadow = shadow_factor(in.world, up);
        }
        if (frame.clouds.x > 0.0) {
            shadow *= cloud_light(in.world);
        }
        let sun = mix(shade_fill(), vec3<f32>(1.0), shadow) * frame.sun_dir.w * frame.daylight.rgb;
        let ao = contact_occlusion(in.world.xz) * rt.ao;
        c = fog_of_war(albedo * ((sun + frame.night.rgb) * ao + point_lighting(in.world, up) * rt.point), in.world);
    } else {
        c = fog_of_war(world_graded(t.rgb * in.tint.rgb) * (frame.sun_dir.w * frame.daylight.rgb + frame.night.rgb + point_lighting(in.world, vec3<f32>(0.0, 1.0, 0.0))), in.world);
    }
    // Metal deposits (perspective camera): the ore glints in the sun, twinkling as it and the
    // eye move (see `ore_glints`); `1 - tint.a` says how rich the deposit is.
    let ore = 1.0 - in.tint.a;
    if (ore > 0.0 && frame.camera_pos.w > 0.5) {
        let footprint = max(length(dpdx(in.world.xz)), length(dpdy(in.world.xz)));
        let v = view_dir(in.world);
        let metal = smoothstep(0.08, 0.3, dot(t.rgb, vec3<f32>(0.333)));
        let sun = frame.sun_color.rgb * frame.sun_dir.w * terrain_sunlight(in.world) * mix(0.55, 1.0, cloud_light(in.world));
        c += sun * vec3<f32>(0.9, 0.95, 1.0) * ore_glints(in.world.xz, v, frame.sun_dir.xyz, footprint, frame.world.x, ore) * metal * los_light(in.world);
    }
    // Into the distance haze as the ground under it (a dark mark would otherwise darken it).
    c = mix(c, fog_color_at(in.world), fog_factor(in.world));
    return vec4<f32>(c * a, a);
}

// A magnified effect's colour and coverage at `uv` (fewer than one of its texels to a pixel:
// TA's explosions seen up close, the classic camera zoomed in): a cubic B-spline through the
// 4 × 4 texels round the sample, premultiplied (the transparent ones round a painting carry no
// dark fringe in), kept to its own frame `rect` and the transparent texel round it. TA's
// dithering melts into smooth shading and its on/off outline into a soft edge, instead of
// bilinear blocks with stair-stepped edges.
fn hd_effect(uv: vec2<f32>, rect: vec4<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(atlas, 0));
    let p = uv * size - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    let c2 = f * f;
    let c3 = c2 * f;
    let u = 1.0 - f;
    let bx = vec4<f32>(u.x * u.x * u.x, 3.0 * c3.x - 6.0 * c2.x + 4.0, -3.0 * c3.x + 3.0 * c2.x + 3.0 * f.x + 1.0, c3.x) / 6.0;
    let by = vec4<f32>(u.y * u.y * u.y, 3.0 * c3.y - 6.0 * c2.y + 4.0, -3.0 * c3.y + 3.0 * c2.y + 3.0 * f.y + 1.0, c3.y) / 6.0;
    let lo = vec2<i32>(floor(rect.xy * size + 0.5)) - 1;
    let hi = vec2<i32>(floor(rect.zw * size + 0.5));
    var sum = vec4<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let t = textureLoad(atlas, clamp(base + vec2<i32>(x - 1, y - 1), lo, hi), 0);
            sum += vec4<f32>(t.rgb * t.a, t.a) * (bx[x] * by[y]);
        }
    }
    return vec4<f32>(sum.rgb / max(sum.a, 1e-4), sum.a);
}

// Features painted lying (see `vs_lying`): their outline smooth (its alpha is the distance to
// it, as a billboard's), their paint enlarged as a billboard's, and lit as the ground they lie
// on: the sun past the models' shadows, the relief's and the clouds', the sky's fill in the
// shade, the lamps and fires round them, and darker under and between what stands about.
@fragment
fn fs_lying(in: VOut) -> @location(0) vec4<f32> {
    var t = textureSample(atlas, atlas_sampler, in.uv);
    // Shrunk, the texels the pixel covers (see `footprint`).
    let dx = dpdx(in.uv);
    let dy = dpdy(in.uv);
    let taps = footprint_taps(dx, dy);
    if (taps.x * taps.y > 1.0) {
        t = footprint(in.uv, dx, dy, in.rect, taps);
    }
    let coverage = clamp((t.a - 0.5) / max(fwidth(t.a), 0.0001) + 0.5, 0.0, 1.0) * in.params.x;
    if (coverage <= 0.0) {
        discard;
    }
    let texels = fwidth(in.uv * vec2<f32>(textureDimensions(atlas, 0)));
    let magnified = smoothstep(0.7, 0.3, max(texels.x, texels.y));
    if (magnified > 0.0) {
        t = vec4<f32>(mix(t.rgb, hd_sprite(in.uv, in.rect), magnified), t.a);
    }
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let rt = rt_visibility(in.clip);
    var shadow = 1.0;
    if (rt.traced) {
        shadow = min(min(mix(1.0, rt.sun, frame.screen.w), rt_feature_shadow(in.world, up)), terrain_sunlight(in.world));
    } else {
        shadow = shadow_factor(in.world, up);
    }
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }
    let sun = mix(shade_fill(), vec3<f32>(1.0), shadow) * frame.sun_dir.w * frame.daylight.rgb;
    let point = point_lighting(in.world, up) * rt.point;
    let ao = contact_occlusion(in.world.xz) * rt.ao;
    // (In the world's colour grade, as the ground it lies on, see `world_graded`.)
    var albedo = world_graded(t.rgb * in.tint.rgb) * (1.0 - 0.15 * frame.rain.x);
    // By moonlight colours fade, as on the ground (see daylight.rs).
    if (frame.night.a > 0.0) {
        albedo = mix(albedo, vec3<f32>(dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722))), 0.5 * frame.night.a);
    }
    let lit = albedo * ((sun + frame.night.rgb) * ao + point * (0.5 + 0.5 * ao));
    return vec4<f32>(apply_fog(lit, in.world) * coverage, coverage);
}

// Additive HDR effects (explosions, muzzle flashes).
@fragment
fn fs_additive(in: VOut) -> @location(0) vec4<f32> {
    var t = textureSample(atlas, atlas_sampler, in.uv);
    let texels = fwidth(in.uv * vec2<f32>(textureDimensions(atlas, 0)));
    let magnified = smoothstep(0.7, 0.3, max(texels.x, texels.y));
    if (magnified > 0.0) {
        t = mix(t, hd_effect(in.uv, in.rect), magnified);
    }
    return additive_color(in, t);
}

fn additive_color(in: VOut, t: vec4<f32>) -> vec4<f32> {
    // (Out of sight, light dims as the GPU's particles' does.)
    var a = t.a * in.params.x * los_light(in.world);
    // Soft (perspective camera): a flash or glow at the ground fades out where its quad meets
    // it (over a quarter of its size), instead of ending along the straight line where the
    // ground in front passes over its flattened depth.
    if (frame.heat.y > 0.0) {
        a *= smoothstep(0.0, max(frame.heat.y, in.extent * 0.25), in.world.y - ground_height(in.world.xz));
    }
    // Fireballs (perspective camera) hide part of what lies behind them, so over pale ground
    // they stay the colour of fire instead of adding up to white.
    var cover = 0.0;
    if (frame.camera_pos.w > 0.5) {
        cover = (1.0 - in.tint.a) * a;
    }
    return vec4<f32>(t.rgb * in.tint.rgb * in.params.z * a, cover);
}

// Alpha-blended billboards (smoke, dust): premultiplied, lit by ambient + sun.
@fragment
fn fs_smoke(in: VOut) -> @location(0) vec4<f32> {
    return smoke_color(in, textureSample(atlas, atlas_sampler, in.uv));
}

fn smoke_color(in: VOut, t: vec4<f32>) -> vec4<f32> {
    var a = t.a * in.params.x;
    // Smoke glows with the fires and explosions around it.
    var sun = frame.sun_color.rgb * frame.sun_dir.w * 0.45;
    if (frame.clouds.x > 0.0) {
        sun *= cloud_light(in.world);
    }
    let light = frame.ambient.rgb * 0.8 + sun + point_lighting_omni(in.world) * 0.6 + lava_light(in.world) * 0.6;
    // Soft particles (perspective camera): smoke thins out as it meets the ground instead of
    // being cut off where the ground passes in front of it.
    if (frame.heat.y > 0.0) {
        a *= smoothstep(0.0, frame.heat.y, in.world.y - ground_height(in.world.xz));
    }
    let c = t.rgb * in.tint.rgb * light * in.params.z;
    return vec4<f32>(apply_fog(c, in.world) * a, a);
}

// ---- Mirrored in the water (the reflection pass, see reflection.rs) ------------------------
// The billboards, effects and smoke above the water, where the water can show them. The frame's
// billboard basis is turned over for this pass, so sprites hang upside down from their mirrored
// anchor.

@fragment
fn fs_cutout_reflection(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, atlas_sampler, in.uv);
    // Handing over to its 3D model (`SpriteInstance::shown`; see `handover_dither`).
    if (t.a < 0.5 || !seen_in_water(in.world) || handover_dither(in.clip.xy) < 1.0 - in.params.z) {
        discard;
    }
    return cutout_color(in, t);
}

@fragment
fn fs_additive_reflection(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, atlas_sampler, in.uv);
    if (!seen_in_water(in.world)) {
        discard;
    }
    return additive_color(in, t);
}

@fragment
fn fs_smoke_reflection(in: VOut) -> @location(0) vec4<f32> {
    let t = textureSample(atlas, atlas_sampler, in.uv);
    if (!seen_in_water(in.world)) {
        discard;
    }
    return smoke_color(in, t);
}
