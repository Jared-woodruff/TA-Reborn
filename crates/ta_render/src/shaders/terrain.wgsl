// Terrain: a heightmap mesh textured with TA's painted tiles (see terrain_common.wgsl for the
// bindings and the virtual-image sampling), and the extended ground around the map (the
// "skirt", sky.rs), which shares the fragment shader. Prepended: common.wgsl,
// terrain_common.wgsl, noise.wgsl, water_common.wgsl.

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) art_px: vec2<f32>,
    @location(2) normal: vec3<f32>,
    // Horizontal distance beyond the map edge (0 on the map itself).
    @location(3) beyond: f32,
    // The slow fields of the ground in the wind (see `life_fields`; zero when the ground is still).
    @location(4) life_a: vec4<f32>,
    @location(5) life_b: vec4<f32>,
};

// Six vertices a cell, its two triangles, split along the diagonal `split_flipped` picks.
@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @location(0) chunk: vec2<u32>) -> VsOut {
    let q = vi / 6u;
    let origin = vec2<i32>(chunk * params.chunk_cells + vec2<u32>(q % params.chunk_cells, q / params.chunk_cells));
    let flipped = split_flipped(height_at(origin), height_at(origin + vec2<i32>(1, 0)), height_at(origin + vec2<i32>(0, 1)), height_at(origin + vec2<i32>(1, 1)));
    // (Both splits wind the same way.)
    var corners = array<vec2<i32>, 6>(vec2<i32>(0, 0), vec2<i32>(1, 1), vec2<i32>(1, 0), vec2<i32>(0, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));
    if (flipped) {
        corners = array<vec2<i32>, 6>(vec2<i32>(0, 0), vec2<i32>(0, 1), vec2<i32>(1, 0), vec2<i32>(1, 0), vec2<i32>(0, 1), vec2<i32>(1, 1));
    }
    let cell = min(origin + corners[vi % 6u], vec2<i32>(params.cells));
    let h = height_at(cell);
    let world = vec3<f32>(f32(cell.x) * 16.0, h, f32(cell.y) * 16.0);

    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    // In space a cell whose four corners are void (marked in the heightmap's second channel,
    // terrain.rs's VOID_MARK) is nothing: its triangles fall outside the view, and the sky shows
    // through.
    if (frame.liquid.y > 0.5 && textureLoad(heightmap, clamp(origin, vec2<i32>(0), vec2<i32>(params.cells) - 1), 0).g > 1.5) {
        out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    }
    out.world = world;
    out.art_px = vec2<f32>(world.x, world.z - h * 0.5);
    out.normal = vertex_normal(cell);
    out.beyond = 0.0;
    out.life_a = vec4<f32>(0.0);
    out.life_b = vec4<f32>(0.0);
    if (frame.rain.z > 0.0) {
        let fields = life_fields(world.xz);
        out.life_a = fields.a;
        out.life_b = fields.b;
    }
    return out;
}

// Extended ground: one vertex per skirt mesh point (world xz, see sky.rs); height and normal
// from `skirt_vertex` (terrain_common.wgsl).
@vertex
fn vs_skirt(@location(0) xz: vec2<f32>) -> VsOut {
    let v = skirt_vertex(xz);
    let world = vec3<f32>(xz.x, v.height, xz.y);
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.art_px = vec2<f32>(world.x, world.z - v.height * 0.5);
    out.normal = v.normal;
    out.beyond = v.beyond;
    out.life_a = vec4<f32>(0.0);
    out.life_b = vec4<f32>(0.0);
    return out;
}

struct Rock {
    color: vec3<f32>,
    normal: vec3<f32>,
};

// ---- Walls (perspective camera) -------------------------------------------------------------
//
// TA's art was painted for one oblique view from the south: a steep face turned from it (east,
// west, north, and a single-cell step) only gets a smear of what lay above it in the picture,
// and a steep face towards it was painted dark. Walls are drawn as a material of their own
// instead, in the colours of the ground around: the world's wall material (rock, volcanic rock,
// lunar rock or plating) projected on the wall along fixed axes (never stretched), rock in
// layered beds, plating in rows of plates.

// Bedding of a rock wall at `world`: beds a few units thick (their surfaces a solid texture in
// world space, waving gently, so they run on across the mesh's facets), grouped in formations;
// each bed its own shade, a shadowed recess under each ledge, joints through alternate beds.
// Every term fades to its mean as it gets finer than the pixels (`dwx`, `dwy`: the world
// position's change per pixel; `footprint` the horizontal one). x = shade, y = the ledges'
// tilt of the normal (up), z = joints (0..1).
fn bedding(world: vec3<f32>, dwx: vec3<f32>, dwy: vec3<f32>, footprint: f32) -> vec3<f32> {
    let w1 = noise_d(world.xz * 0.006);
    let w2 = noise_d(world.xz * 0.03 + vec2<f32>(7.1, 3.3));
    let s = (world.y + w1.x * 5.0 + w2.x * 0.8) * 0.16;
    // Beds 3.5 to 9 units thick (`depth` keeps rising: 1 + 0.276 cos > 0).
    let depth = s + 0.12 * sin(2.3 * s);
    let ds = 1.0 + 0.276 * cos(2.3 * s);
    let grad = vec3<f32>(w1.y * 0.03 + w2.y * 0.024, 1.0, w1.z * 0.03 + w2.z * 0.024) * 0.16 * ds;
    // Beds per pixel.
    let rate = max(abs(dot(grad, dwx)), abs(dot(grad, dwy)));
    let bed = floor(depth);
    let bed_k = 1.0 - smoothstep(0.2, 0.5, rate);
    var shade = 1.0 + (hash01(bed) - 0.5) * 0.26 * bed_k;
    // Formations of about four beds, their edges softened as they get finer.
    let d2 = depth * 0.25 + 0.37;
    let f2 = floor(d2);
    let form = mix(hash01(f2 + 1000.0 - 1.0), hash01(f2 + 1000.0), smoothstep(0.0, 0.2 + rate * 0.25, fract(d2)));
    shade *= 1.0 + (form - 0.5) * 0.3 * (1.0 - smoothstep(0.25, 0.7, rate * 0.25));
    // A recess under each ledge (the top fifth of each bed), its edges filtered over the
    // pixel; far off, its mean.
    let lip_k = 1.0 - smoothstep(0.12, 0.35, rate);
    let w = max(rate, 0.05);
    let recess = pulse_filtered(depth, 0.8, 0.2, w);
    shade *= 1.0 - 0.3 * mix(0.2, recess, lip_k);
    let tilt = (pulse_filtered(depth, 0.0, 0.08, w) - 0.5 * recess) * 0.5 * lip_k;
    let joint = (1.0 - smoothstep(0.0, 0.06, abs(noise(world.xz * 0.07 + vec2<f32>(bed * 3.1, bed * 1.7)))))
        * step(0.5, fract(bed * 0.618)) * (1.0 - smoothstep(0.8, 2.0, footprint));
    return vec3<f32>(shade, tilt, joint);
}

// A random number (0..1) for a whole number `b`.
fn hash01(b: f32) -> f32 {
    return f32(hash_u32(bitcast<u32>(i32(b)))) / 4294967295.0;
}

// The integral of a pulse train (1 over [a, a + b) of every unit), and its mean over a window
// `w` wide around `x`: the pulse filtered by a pixel.
fn pulse_integral(x: f32, a: f32, b: f32) -> f32 {
    return floor(x) * b + clamp(fract(x) - a, 0.0, b);
}

fn pulse_filtered(x: f32, a: f32, b: f32, w: f32) -> f32 {
    return (pulse_integral(x + w * 0.5, a, b) - pulse_integral(x - w * 0.5, a, b)) / w;
}

// A wall at `world` (normal `n`) in colour `base`, of material `m` (see `wall_material`):
// the material's grain and relief projected on the wall along the x and z axes (weighted by
// how far the wall faces along each), rock in beds (`strata`: how much, 0..1), plating in rows
// (`plates`). `hd` = the HD surfaces' share (the material's texture only with them).
fn wall_surface(world: vec3<f32>, n: vec3<f32>, base: vec3<f32>, m: u32, art: ArtFilter, hd: f32, reborn: f32, strata: f32, plates: bool) -> Rock {
    var r: Rock;
    var color = base;
    var normal = n;
    if (hd > 0.0) {
        var wa = n.z * n.z;
        var wb = n.x * n.x;
        wa *= wa;
        wb *= wb;
        let sum = max(wa + wb, 1e-4);
        wa /= sum;
        wb /= sum;
        let da = material_detail(m, world.xy, art.dwx.xy, art.dwy.xy);
        let db = material_detail(m, world.zy, art.dwx.zy, art.dwy.zy);
        let d = da * wa + db * wb;
        color *= (1.0 + d.x * MATERIAL_GRAIN[m] * mix(0.16, 0.76, reborn) * hd) * (1.0 - max(d.w, 0.0) * 0.15 * reborn * hd);
        let lit = clamp(frame.sun_color.a / 0.35, 0.0, 2.0);
        normal = normalize(normal - (vec3<f32>(da.y, da.z, 0.0) * wa + vec3<f32>(0.0, db.z, db.y) * wb) * lit * mix(0.45, 0.88, reborn) * hd);
    }
    if (plates) {
        // Rows of plates every 12 units up the wall, seams every 32 along it, grime streaking
        // down; the seams filtered over the pixel.
        let up = max(abs(art.dwx.y), abs(art.dwy.y));
        let along = select(world.z, world.x, abs(n.z) > abs(n.x));
        let across = max(length(art.dwx.xz), length(art.dwy.xz));
        let row = pulse_filtered(world.y / 12.0, 0.0, 0.05, max(up / 12.0, 0.02));
        let seam = pulse_filtered(along / 32.0, 0.0, 0.02, max(across / 32.0, 0.01));
        let streak = noise(vec2<f32>(along * 0.15, world.y * 0.01)) * 0.5 + 0.5;
        color *= (1.0 - 0.45 * max(row, seam)) * mix(1.0, 0.82, streak);
    } else if (strata > 0.0) {
        let b = bedding(world, art.dwx, art.dwy, art.footprint);
        color *= mix(1.0, b.x * (1.0 - 0.35 * b.z), strata);
        normal = normalize(normal + vec3<f32>(0.0, b.y * strata, 0.0));
    }
    r.color = color;
    r.normal = normal;
    return r;
}

// The world's wall material: plating on metal worlds, volcanic rock on lava worlds, lunar rock
// on the moon's, else rock.
fn wall_material() -> u32 {
    if (detail_world.flags.x > 0.5) {
        return 8u;
    }
    if (detail_weight(7u) >= 1.0) {
        return 7u;
    }
    if (detail_weight(9u) >= 1.0) {
        return 9u;
    }
    return 3u;
}

// The sky a wet surface mirrors in direction `r`: the haze at the horizon, the zenith above.
fn wet_sky(r: vec3<f32>) -> vec3<f32> {
    if (beyond_on()) {
        return mix(haze_color(r), frame.sky_zenith.rgb, smoothstep(0.0, 0.7, r.y));
    }
    return frame.fog.rgb;
}

fn luminance(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// How much a painted colour looks like the painted sea (0..1): far along the beach-to-sea
// colour axis *and* close to it, or deeper still: darker than the sea's colour (the mean of
// the open sea) and of its hue, as TA painted its deepest water (Acid Pools' deep acid, purer
// and darker than that mean); not where the sea's colour is too grey to have a hue.
// `painted_water` only measures the first, on which dark vegetation and rock score as high as
// the sea does. Needs the shoreline colours (`frame.shore_land.a > 0`).
fn painted_sea_likeness(c: vec3<f32>) -> f32 {
    let s = sqrt(max(c, vec3<f32>(0.0)));
    let land = frame.shore_land.rgb;
    let sea = frame.shore_water.rgb;
    let axis = sea - land;
    let t = saturate(dot(s - land, axis) / max(dot(axis, axis), 1e-6));
    let off = distance(s, land + axis * t);
    let along = smoothstep(0.35, 0.7, t) * (1.0 - smoothstep(0.05, 0.1, off));
    // (How light along the sea's colour, and how far from its hue, as chords of unit colours.)
    let k = dot(s, sea) / max(dot(sea, sea), 1e-6);
    let hue = normalize(sea + vec3<f32>(1e-5));
    let apart = distance(s / max(length(s), 1e-4), hue);
    let chroma = smoothstep(0.2, 0.35, distance(hue, vec3<f32>(0.57735)));
    let deeper = smoothstep(0.3, 0.45, k) * (1.0 - smoothstep(1.0, 1.15, k)) * (1.0 - smoothstep(0.17, 0.26, apart)) * chroma;
    return max(along, deeper);
}

// How deep (world units; < 0 on land) the water drawn over the extended ground at `xz`,
// `height` high and `beyond` units past the map (its art `shown`, see `shown_art`), lies there,
// as the water pass decides it (`fs_water_outer`): next to the map by the ground's height, where
// the vertices carry the coast, further out by the per-pixel shoreline (see `beyond_wet`). Under
// the water's level judged where the eye's ray meets the water in front of this ground, by the
// water there (on the map, the map's: the ground's height alone): the coast's band lies just
// under the water, and seen low the ground there is many units further along the ray than the
// surface over it (judged at the ground, along the far side of every bank the water said land
// and the ground sea, a hard dark rim). Above it, at the ground itself: the ray meets no water in
// front of it. (That point's mirror image lies a few units from this one's, `shown.at`, the mirror
// running the map backwards along the axes it flips.)
fn beyond_ground_depth(xz: vec2<f32>, height: f32, beyond: f32, shown: ShownArt) -> f32 {
    let sea = frame.world.y;
    let v = view_dir(vec3<f32>(xz.x, height, xz.y));
    let shift = v.xz * (max(sea - height, 0.0) / max(v.y, 0.05));
    let d = beyond_distance(xz + shift);
    let k = beyond_coast_mix(d);
    if (k <= 0.0) {
        return sea - height;
    }
    let g = beyond_ground_from(shown.at + shift * shown.mirrored, xz + shift, d, shown.far, false);
    let w = beyond_wet_from(g.y, xz + shift, d);
    let ramp = (w - 0.5) * 200.0;
    return mix(sea - height, mix(ramp, min(ramp, max(sea - g.x, 6.0)), smoothstep(0.9, 0.98, w)), k);
}

// Painted ground colour at art pixel `a` (trilinear, `lod`, sharpened by `sharp`).
//
// When the world continues beyond the map, ground whose art pixel lies outside the painted
// image (the strip along the north edge TA never showed, and the extended ground; `xz` =
// world position, `beyond` = world distance from the map, `height` = ground height) shows the
// art `shown` (see `shown_art`: beyond the map, the mirrored ground's own, softening with the
// distance; further out the far land's own colours, `far_color`). Land and sea there follow the
// ground, not the art, where the two disagree (the
// land and sea of the far ground are blurred, its art much less): dry ground painted like the
// sea shows the painted beach, ground under the sea the painted sea bed. The extended ground
// also varies slowly in brightness and gets a little darker and less saturated with distance,
// so the playable area still stands out. Continuous at the image edge and at the map edge.
//
// `depth`: how deep the water lies over it, as the water pass decides it (see
// `beyond_ground_depth`; on the map, under the sea level).
fn ground_albedo(a: vec2<f32>, lod: f32, sharp: f32, xz: vec2<f32>, beyond: f32, height: f32, shown: ShownArt, depth: f32) -> vec3<f32> {
    if (!beyond_on()) {
        return art_sample(a, lod, sharp);
    }
    let size = vec2<f32>(params.tiles * 32u);
    // (Only the dither's finest speckle, here: the HD surfaces calm more, see `seam_calm`.)
    let seam = seam_calm(xz, beyond) * 0.7;
    if (beyond <= 0.0 && all(a >= vec2<f32>(0.0)) && all(a <= size)) {
        let c = art_sample(a, lod, sharp);
        if (seam > 0.0) {
            return mix(c, art_soft(a, 1u), seam);
        }
        return c;
    }
    var c = vec3<f32>(0.0);
    if (shown.far < 1.0) {
        c = shown_color(art_sample(shown.px, max(lod, shown.blur), sharp * (1.0 - saturate(shown.blur))), shown);
        if (seam > 0.0) {
            c = mix(c, shown_color(art_soft(shown.px, 1u), shown), seam);
        }
    }
    if (shown.far > 0.0) {
        // (Under its sea, the sea as the map's is painted round there.)
        let wet = select(0.0, smoothstep(0.5, 6.0, depth), frame.world.y > 0.0);
        var far = vec3<f32>(0.0);
        if (wet < 1.0) {
            // (On a metal world its blocks' own shades vary the plating, not soft patches,
            // which lay over it like smoke.)
            far = far_color(xz, beyond, exp2(lod), select(1.0, 0.25, detail_world.weights[2].x >= 1.5));
        }
        if (wet > 0.0) {
            far = mix(far, far_sea_color(xz, beyond), wet);
        }
        c = mix(c, far, shown.far);
    }
    let sea = frame.world.y;
    // (How much the ground lies under the sea.)
    var under = 0.0;
    if (sea > 0.0) {
        // Under the sea where the water pass draws it (`depth`). (Ground standing out of the water
        // where that is sea, where the mesh's triangles cut the per-pixel shoreline, is painted as
        // the sea: from above it lies on the picture as the sea round it does; seen low it
        // mirrors the sky as the water does, see `ground_color`. Not on lava or mist, which have
        // no such surface to be drawn as: ground standing out of them is dry, never flat paint.)
        var wet = smoothstep(0.5, 6.0, depth);
        var dry = smoothstep(-0.5, 0.5, -depth);
        if (frame.water.w < 0.5 || frame.water_extra.y > 0.5) {
            wet = min(wet, smoothstep(0.5, 6.0, sea - height));
            dry = max(dry, smoothstep(sea - 0.5, sea + 0.5, height));
        }
        if (frame.shore_land.a > 0.0 && frame.beyond_shore.a > 0.0) {
            var sea_like = painted_sea_likeness(c);
            // (And within a few units of the coast the dark outline TA painted round its islands
            // at the water's edge, darker than its sea: the reflection's painted coast falls a
            // few units beside its heights' (their mips, the art's shift), and the outline lay
            // on the dry bank as a hard dark rim. And on the flats the per-pixel shoreline leaves
            // dry under the water's level (the coast's band, see `skirt_height`): a beach by
            // construction, hundreds of units wide far out, where the art is the painted shallows
            // and sea round the reflected coast, blurred into blue-greys; from low over the sea
            // each far island's flats, seen edge-on, were a dark hairline across the horizon.)
            let outline = painted_water(c) * dry;
            // (Not the far land's flats: its colours are its own, no painted sea to set right.)
            if (beyond > 0.0 && outline > sea_like) {
                let flats = (1.0 - smoothstep(sea - 0.2, sea + 0.2, height)) * (1.0 - shown.far);
                sea_like = mix(sea_like, outline, max(1.0 - smoothstep(2.0, 8.0, -shore_distance(xz)), flats));
            }
            c = mix(c, frame.beyond_shore.rgb, sea_like * dry);
        }
        if (frame.beyond_sea.a > 0.0) {
            c = mix(c, frame.beyond_sea.rgb, (1.0 - painted_water(c)) * wet);
        }
        under = wet;
    }
    if (beyond > 0.0) {
        // (The mirror image's: the far land has patches of its own, see `far_color`.)
        if (shown.far < 1.0) {
            let n = noise(xz / 2600.0) + 0.5 * noise(xz / 1100.0 + vec2<f32>(7.1, 3.3));
            c *= 1.0 + n * 0.3 * smoothstep(0.0, 1800.0, beyond) * (1.0 - shown.far);
        }
        // (A touch at once, over the first few hundred units, so the edge of the playable
        // area reads without a line; then slowly. Only on the land: the sea's surface runs on
        // unbroken, and its bed, seen through it, keeps its tone, so the edge never shows as a
        // band across the sea.)
        let desaturate = mix(1.0, 0.9, smoothstep(0.0, 800.0, beyond)) * mix(1.0, 0.92, smoothstep(800.0, 5000.0, beyond));
        let dim = mix(1.0, 0.9, smoothstep(0.0, 500.0, beyond)) * mix(1.0, 0.92, smoothstep(500.0, 4000.0, beyond));
        c = mix(vec3<f32>(luminance(c)), c, mix(desaturate, 1.0, under)) * mix(dim, 1.0, under);
    }
    return max(c, vec3<f32>(0.0));
}

// How much the painted speckle at world `xz` (`beyond` units past the map) is calmed to the art
// averaged over four pixels, a few pixels either side of the map's edges when the ground runs on
// beyond them as their mirror image: there the reflection's speckle closes up with the map's
// own into a column of kaleidoscope chevrons along the edge. (Further out the reflection is
// moved along the edge, `beyond_art_shift`.)
fn seam_calm(xz: vec2<f32>, beyond: f32) -> f32 {
    let size = map_size_px();
    let inside = min(min(xz.x, xz.y), min(size.x - xz.x, size.y - xz.y));
    return 0.85 * (1.0 - smoothstep(0.0, 12.0, select(inside, beyond, beyond > 0.0)));
}

// HD surfaces, up close: magnified, TA's ground art is a grid of square pixels, reduced to 256
// colours with dithering. A Catmull-Rom spline through the 4×4 art pixels around the sample
// keeps every painted pixel's colour (the sand's grains, the rocks' speckle) but joins them
// with smooth curves instead of square steps; a quarter of a cubic B-spline (which averages
// the dither pattern out) calms the dithering's harshest contrast, and the spline's overshoot
// is clamped to the pixels around. The smoothed luminance's slope becomes relief under the
// dynamic sun.
struct HdGround {
    color: vec3<f32>,
    // Luminance gradient (per art pixel) of the smoothed art.
    grad: vec2<f32>,
    // Local contrast of the painted speckle (standard deviation of the luminance).
    energy: f32,
};

// Catmull-Rom weights for the four texels around fraction `t`.
fn catmull_rom(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(-t3 + 2.0 * t2 - t, 3.0 * t3 - 5.0 * t2 + 2.0, -3.0 * t3 + 4.0 * t2 + t, t3 - t2) * 0.5;
}

// Uniform cubic B-spline weights for the four texels around fraction `t`, and their
// derivatives.
fn bspline(t: f32) -> vec4<f32> {
    let u = 1.0 - t;
    return vec4<f32>(u * u * u, 3.0 * t * t * t - 6.0 * t * t + 4.0, -3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0, t * t * t) / 6.0;
}

fn bspline_d(t: f32) -> vec4<f32> {
    let u = 1.0 - t;
    return vec4<f32>(-u * u, 3.0 * t * t - 4.0 * t, -3.0 * t * t + 2.0 * t + 1.0, t * t) * 0.5;
}

fn hd_ground(px: vec2<f32>) -> HdGround {
    let p = px - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    let bx = bspline(f.x);
    let by = bspline(f.y);
    let cx = catmull_rom(f.x);
    let cy = catmull_rom(f.y);
    let dx = bspline_d(f.x);
    let dy = bspline_d(f.y);
    var sharp = vec3<f32>(0.0);
    var soft = vec3<f32>(0.0);
    var lo = vec3<f32>(1e9);
    var hi = vec3<f32>(-1e9);
    var out: HdGround;
    out.grad = vec2<f32>(0.0);
    var m = 0.0;
    var m2 = 0.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let t = art_texel(base + vec2<i32>(x - 1, y - 1), 0u);
            sharp += t * (cx[x] * cy[y]);
            soft += t * (bx[x] * by[y]);
            let l = sqrt(max(luminance(t), 0.0));
            out.grad += l * vec2<f32>(dx[x] * by[y], bx[x] * dy[y]);
            m += l;
            m2 += l * l;
            if (x == 1 || x == 2) && (y == 1 || y == 2) {
                lo = min(lo, t);
                hi = max(hi, t);
            }
        }
    }
    out.color = mix(clamp(sharp, lo, hi), soft, 0.25);
    m /= 16.0;
    out.energy = sqrt(max(m2 / 16.0 - m * m, 0.0));
    return out;
}

// The art at mip `lod` through a cubic B-spline: its colours averaged over a few art pixels,
// smoothly (none of a mip's square steps).
fn art_soft(px: vec2<f32>, lod: u32) -> vec3<f32> {
    let p = px / f32(1u << lod) - 0.5;
    let base = vec2<i32>(floor(p));
    let f = p - floor(p);
    let bx = bspline(f.x);
    let by = bspline(f.y);
    var c = vec3<f32>(0.0);
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            c += art_texel(base + vec2<i32>(x - 1, y - 1), lod) * (bx[x] * by[y]);
        }
    }
    return c;
}

// Micro detail finer than the art pixels at art pixel `a`: octaves of gradient noise, each
// fading in only once its features span a few screen pixels (`rho` = art pixels per screen
// pixel), so it never shimmers. x = brightness variation (about ±1), yz = its slope. (The
// finest octave only shows from a unit's own eyes.)
fn micro_detail(a: vec2<f32>, rho: f32) -> vec3<f32> {
    var d = vec3<f32>(0.0);
    let freqs = vec4<f32>(1.1, 2.3, 4.7, 9.8);
    for (var k = 0; k < 4; k++) {
        let f = freqs[k];
        let w = smoothstep(0.45 / f * 1.1, 0.3 / f * 1.1, rho);
        if (w > 0.0) {
            let n = noise_d(a * f + vec2<f32>(f32(k) * 17.3, f32(k) * 5.1));
            d += vec3<f32>(n.x, n.yz * f) * w * (1.0 / f32(k + 1));
        }
    }
    return d * 2.5;
}

// ---- The ground's detail materials (terrain_detail.rs) ------------------------------------------
//
// Up close, TA's painted ground gets the surface it shows: sand, soil, grass, rock, gravel, snow,
// ice, volcanic rock, worn metal or lunar dust (`tools/terrain/detail.py`). At each place the two
// materials that its painted colour, speckle and slope look most like, weighed by what the
// world's ground is made of (`detail_world`), lighten and darken the painted colour with their
// grain and add their relief.

@group(2) @binding(0) var detail_maps: texture_2d_array<f32>;
@group(2) @binding(1) var detail_sampler: sampler;
struct DetailWorld {
    weights: array<vec4<f32>, 3>,
    // x: the walls are metal (1) or rock (0); y: show each place's material (a check); z: how
    // much of the ground's colour round them the walls keep (`world_wall_colour`); w: how light
    // they are kept at the least, a share of the ground round them (`world_wall_lift`).
    flags: vec4<f32>,
    // The world's broad look (WorldLook in terrain_detail.rs): dry, dark and shore (rgb a hue
    // at luminance 1, a its value), strength (xyz theirs, w 1 for dark flows), grade (x
    // desaturation, y value swing, z patch size, w region size).
    look: array<vec4<f32>, 5>,
};
@group(2) @binding(2) var<uniform> detail_world: DetailWorld;

// Each material's tile (world units), how much its grain lightens and darkens the painted colour,
// and its relief (world units of height over its range).
var<private> MATERIAL_TILE: array<f32, 10> = array<f32, 10>(36.0, 18.0, 12.0, 56.0, 18.0, 72.0, 72.0, 26.0, 40.0, 56.0);
var<private> MATERIAL_GRAIN: array<f32, 10> = array<f32, 10>(0.55, 0.6, 0.7, 0.6, 0.7, 0.3, 0.45, 0.6, 0.5, 0.55);
// (The relief tilts the surface a few tens of degrees at most where the material's texture is
// sharp: sand, soil, grass, gravel and volcanic grit are fine stuff, rock and regolith rougher.)
var<private> MATERIAL_RELIEF: array<f32, 10> = array<f32, 10>(0.5, 0.35, 0.65, 0.8, 0.3, 2.4, 1.4, 0.35, 0.75, 1.0);
// How much of the paint's light and shade a material evens out (its own grain stands in): much on
// sand and snow, little on rock, where the painted cracks and ledges are the point.
var<private> MATERIAL_SMOOTH: array<f32, 10> = array<f32, 10>(0.6, 0.3, 0.35, 0.12, 0.2, 0.6, 0.45, 0.2, 0.3, 0.35);
// Reborn gives each surface a physical response. Enhanced Classic keeps the painted finish,
// using these same classified details only as restrained, filtered grain.
var<private> MATERIAL_ROUGHNESS: array<f32, 10> = array<f32, 10>(0.9, 0.94, 0.9, 0.8, 0.78, 0.7, 0.29, 0.88, 0.43, 0.95);
var<private> MATERIAL_TINT: array<vec3<f32>, 10> = array<vec3<f32>, 10>(
    vec3<f32>(1.04, 1.0, 0.87), vec3<f32>(1.02, 0.95, 0.85), vec3<f32>(0.88, 1.0, 0.78),
    vec3<f32>(0.96, 0.98, 1.02), vec3<f32>(1.0, 0.97, 0.91), vec3<f32>(0.96, 1.01, 1.05),
    vec3<f32>(0.85, 1.01, 1.10), vec3<f32>(0.97, 0.95, 0.95), vec3<f32>(0.92, 0.99, 1.06), vec3<f32>(1.0, 0.99, 0.97),
);

fn detail_weight(i: u32) -> f32 {
    return detail_world.weights[i / 4u][i % 4u];
}

struct GroundMaterial {
    a: u32,
    b: u32,
    // How much of the second, and how sure the guess is (0: none applies).
    mix_b: f32,
    sure: f32,
};

// What the ground is made of where its painted colour (averaged over eight art pixels or so,
// linear: the dither's speckle would make the guess speckle too) is `c` and the slope `steep` (0
// level): scores for each material from the colour's lightness, saturation and hue and the slope,
// times the world's weights; the best two.
fn ground_material(c: vec3<f32>, steep: f32) -> GroundMaterial {
    let s = sqrt(max(c, vec3<f32>(0.0)));
    let l = dot(s, vec3<f32>(0.2126, 0.7152, 0.0722));
    let hi = max(max(s.r, s.g), s.b);
    let lo = min(min(s.r, s.g), s.b);
    let sat = (hi - lo) / max(hi, 1e-3);
    let warm = s.r - s.b;
    let green = s.g - max(s.r, s.b);
    let blue = s.b - s.r;
    var score: array<f32, 10>;
    // Sand: warm and light. Soil: warm and darker. Grass: green.
    score[0] = smoothstep(0.02, 0.12, warm) * smoothstep(0.4, 0.58, l) * (1.0 - smoothstep(0.02, 0.08, green));
    score[1] = smoothstep(0.0, 0.08, warm) * smoothstep(0.1, 0.22, l) * (1.0 - smoothstep(0.42, 0.58, l)) * (1.0 - smoothstep(0.03, 0.1, green));
    score[2] = smoothstep(0.01, 0.07, green);
    // Rock: grey, or steep. Gravel: greyish (only worlds of it weigh it much).
    score[3] = (1.0 - smoothstep(0.1, 0.3, sat)) * 0.6 + smoothstep(0.3, 0.6, steep);
    score[4] = (1.0 - smoothstep(0.2, 0.4, sat)) * 0.55;
    // Snow: near white. Ice: light and blue.
    score[5] = smoothstep(0.68, 0.85, l) * (1.0 - smoothstep(0.15, 0.3, sat));
    score[6] = smoothstep(0.02, 0.1, blue) * smoothstep(0.45, 0.7, l);
    // Volcanic rock: dark. Metal and lunar dust: grey (only their worlds weigh them).
    score[7] = 1.0 - smoothstep(0.15, 0.35, l);
    score[8] = 1.0 - smoothstep(0.15, 0.35, sat);
    score[9] = 1.0 - smoothstep(0.12, 0.3, sat);
    // A world's prior is also evidence: painted colored metal is still metal, shaded snow
    // is still snow, and warm lunar dust must not become untreated earth. The colour scores
    // continue to distinguish mixed beaches, vegetation and rock within those worlds.
    score[0] = max(score[0], smoothstep(1.1, 1.4, detail_weight(0u)) * smoothstep(0.32, 0.6, l) * 0.48);
    score[5] = max(score[5], smoothstep(1.0, 1.5, detail_weight(5u)) * (1.0 - smoothstep(0.16, 0.45, sat)) * smoothstep(0.28, 0.6, l) * 0.56);
    score[7] = max(score[7], smoothstep(1.0, 1.5, detail_weight(7u)) * (1.0 - smoothstep(0.45, 0.7, l)) * 0.58);
    score[8] = max(score[8], select(0.0, 0.75, detail_world.flags.x > 0.5));
    score[9] = max(score[9], smoothstep(1.0, 1.6, detail_weight(9u)) * 0.62);
    var g: GroundMaterial;
    var best = 0.0;
    var second = 0.0;
    for (var i = 0u; i < 10u; i++) {
        let w = score[i] * detail_weight(i);
        if (w > best) {
            second = best;
            g.b = g.a;
            best = w;
            g.a = i;
        } else if (w > second) {
            second = w;
            g.b = i;
        }
    }
    // (By the cube of the scores: the two blend where they score alike, a clear winner keeps its
    // own surface. By the scores themselves, pale sand carried a quarter of the rock's cracks,
    // a net of dried mud over the beaches.)
    let best3 = best * best * best;
    let second3 = second * second * second;
    g.mix_b = second3 / max(best3 + second3, 1e-6);
    g.sure = smoothstep(0.08, 0.35, best);
    // On lava worlds, molten rock (bright and fiery) takes none.
    if (detail_weight(7u) > 0.0) {
        g.sure *= 1.0 - smoothstep(0.7, 0.85, l) * smoothstep(0.3, 0.45, sat);
    }
    return g;
}

// ---- The far land's own ground (perspective camera; see `far_color`) ---------------------------
//
// On most worlds the far land is patches of the map's own colours, its hills its vertices'. On a
// metal world or the moon such patches, soft and grey, read as smoke or cloud lying on the land:
// there it has a surface of its own, lit by the sun, blocks of plating or the moon's craters.

struct FarSurface {
    // How much lighter or darker its ground is (a factor), and its relief's slope along x and z.
    shade: f32,
    slope: vec2<f32>,
};

// A random number (0..1) for cell `c` (whole numbers).
fn cell_hash(c: vec2<f32>) -> f32 {
    return f32(hash_u32((bitcast<u32>(i32(c.x)) * 1597334677u) ^ hash_u32(bitcast<u32>(i32(c.y)) + 2654435769u))) / 4294967295.0;
}

// A metal world's blocks of plating, a cell `size` units a side each (some split in two by a
// lane), standing a random height over the streets between them, at `xz`: x = the height (0..1
// of the tallest), y = the shade of what is there (a block's own, or the street's).
fn plating_at(xz: vec2<f32>, size: f32, edge_w: f32) -> vec2<f32> {
    let c = floor(xz / size);
    var f = xz / size - c;
    let h = cell_hash(c);
    let split = cell_hash(c + vec2<f32>(37.0, 11.0));
    var part = 0.0;
    if (split > 0.55) {
        if (split > 0.78) {
            part = floor(f.x * 2.0);
            f.x = fract(f.x * 2.0);
        } else {
            part = floor(f.y * 2.0);
            f.y = fract(f.y * 2.0);
        }
    }
    let street = mix(0.07, 0.15, fract(split * 7.31));
    let edge = min(min(f.x, f.y), min(1.0 - f.x, 1.0 - f.y)) - street;
    let block = smoothstep(-edge_w, edge_w, edge);
    let own = cell_hash(c + vec2<f32>(part * 53.0 + 5.0, 17.0));
    return vec2<f32>(block * mix(0.15, 1.0, h), mix(0.74, mix(0.86, 1.14, own), block));
}

// The plating at `xz` seen `footprint` units a pixel: blocks of 210 units in districts of 840
// (each a shade of its own), the blocks' sides lit and shaded as the sun has them; faded to their
// mean as a street gets finer than the pixels.
fn far_plating(xz: vec2<f32>, footprint: f32) -> FarSurface {
    var out = FarSurface(1.0, vec2<f32>(0.0));
    let size = 210.0;
    let k = 1.0 - smoothstep(size * 0.06, size * 0.22, footprint);
    let district = 1.0 + 0.12 * (cell_hash(floor(xz / (size * 4.0)) + vec2<f32>(91.0, 7.0)) - 0.5);
    if (k <= 0.0) {
        out.shade = district * 0.95;
        return out;
    }
    // (The blocks' sides a pixel wide or a few units, whichever is wider; their slope from the
    // heights either side of the point, as far apart.)
    let e = max(footprint, 4.0);
    let edge_w = 0.5 * e / size;
    let here = plating_at(xz, size, edge_w);
    let hx = plating_at(xz + vec2<f32>(e, 0.0), size, edge_w).x - plating_at(xz - vec2<f32>(e, 0.0), size, edge_w).x;
    let hz = plating_at(xz + vec2<f32>(0.0, e), size, edge_w).x - plating_at(xz - vec2<f32>(0.0, e), size, edge_w).x;
    // (The tallest blocks 40 units over the streets.)
    out.slope = vec2<f32>(hx, hz) * 40.0 / (2.0 * e) * k;
    out.shade = district * mix(0.95, here.y, k);
    return out;
}

// The moon's craters: its regolith's own (the detail material's), enlarged into two fields of
// craters some tens and some hundreds of units across, their slopes as steep as the material's
// (the craters enlarged whole), and their floors and rims as dark and light.
fn far_craters(xz: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> FarSurface {
    let a = crater_field(xz, dx, dy, 1100.0, mat2x2<f32>(1.0, 0.0, 0.0, 1.0), vec2<f32>(0.0));
    let b = crater_field(xz, dx, dy, 4300.0, mat2x2<f32>(0.8, 0.6, -0.6, 0.8), vec2<f32>(0.31, 0.67));
    return FarSurface(a.shade * b.shade, a.slope + b.slope);
}

// One field of `far_craters`: the regolith's tile enlarged to `tile` units, turned by `turn` and
// moved by `offset` (of a tile).
fn crater_field(xz: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, tile: f32, turn: mat2x2<f32>, offset: vec2<f32>) -> FarSurface {
    let t = textureSampleGrad(detail_maps, detail_sampler, turn * xz / tile + offset, 9, turn * dx / tile, turn * dy / tile);
    let slope = transpose(turn) * ((t.gb * 255.0 - 128.0) / 127.0) * (512.0 / (6.0 * MATERIAL_TILE[9])) * MATERIAL_RELIEF[9];
    return FarSurface(1.0 + (t.r * 2.0 - 1.0) * 0.3, slope);
}

// The far land's own surface at `xz` (`dx`, `dy`: its change per screen pixel), as its world has
// one: blocks of plating on a metal or urban world, craters on the moon; none elsewhere.
fn far_surface(xz: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> FarSurface {
    if (detail_world.weights[2].x >= 1.5) {
        return far_plating(xz, max(length(dx), length(dy)));
    }
    if (detail_world.weights[2].y >= 1.5) {
        return far_craters(xz, dx, dy);
    }
    return FarSurface(1.0, vec2<f32>(0.0));
}

// Material `m`'s detail at `xz` (world units; `dx`, `dy` their change per screen pixel): the
// brightness it adds (-1..1) and its height's slope along x and z (world units of height per
// world unit, its relief in full).
fn material_detail(m: u32, xz: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let tile = MATERIAL_TILE[m];
    let t = textureSampleGrad(detail_maps, detail_sampler, xz / tile, m, dx / tile, dy / tile);
    // A copy three times larger and turned lightens and darkens it, so the tiling does not show.
    // (Sand's is not turned, only a little larger: its ripples keep the wind's direction.)
    let sand = m == 0u;
    var rot = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    if (sand) {
        rot = mat2x2<f32>(1.0, 0.0, 0.0, 1.0);
    }
    let big = tile * select(3.3, 1.13, sand);
    let t2 = textureSampleGrad(detail_maps, detail_sampler, rot * xz / big + vec2<f32>(0.37, 0.71), m, rot * dx / big, rot * dy / big);
    var bright = mix(t.r, t2.r, 0.35) * 2.0 - 1.0;
    // (Stored as the height's change per texel times six; a texel is tile / 512 world units.)
    var slope = ((t.gb * 255.0 - 128.0) / 127.0) * (512.0 / (6.0 * tile)) * MATERIAL_RELIEF[m];
    var height = t.a;
    if (sand) {
        // Wind ripples: the copy takes over in patches, relief and all, so the crests fork
        // where the two meet and the tiling does not show; in sheltered patches the ripples lie
        // low; and they fade out before a ripple is too narrow on the screen to draw (into
        // corduroy, or a shimmer).
        let swap = smoothstep(-0.12, 0.12, noise(xz / 47.0) + 0.5 * noise(xz / 19.0 + vec2<f32>(3.1, 7.3)));
        let slope2 = ((t2.gb * 255.0 - 128.0) / 127.0) * (512.0 / (6.0 * big)) * MATERIAL_RELIEF[m];
        let lie = mix(0.4, 1.0, smoothstep(-0.2, 0.15, noise(xz / 130.0 + vec2<f32>(5.3, 1.7)) + 0.5 * noise(xz / 53.0 + vec2<f32>(7.7, 1.9))));
        let fine = 1.0 - smoothstep(0.15, 0.4, min(length(dx), length(dy)));
        bright = (mix(t.r, t2.r, swap) * 2.0 - 1.0) * mix(0.6, 1.0, lie) * mix(0.5, 1.0, fine);
        slope = mix(slope, slope2, swap) * lie * fine;
        height = mix(t.a, t2.a, swap);
    }
    if (m == 2u) {
        // Turf reads as patches of growth, not uniformly sharpened carpet. Reuse the
        // larger sample for clump variation; finer blades contribute quieter normals.
        // This remains inside the HD material path, with no additional texture reads.
        let turf = saturate(frame.ao.w);
        let growth = mix(0.45, 0.9, smoothstep(-0.35, 0.4, noise(xz / 26.0)));
        bright = mix(bright, (mix(t.r, t2.r, 0.45) * 2.0 - 1.0) * 0.85, turf);
        let broad_slope = transpose(rot) * ((t2.gb * 255.0 - 128.0) / 127.0)
            * (512.0 / (6.0 * big)) * MATERIAL_RELIEF[m];
        slope = mix(slope, (slope * 0.28 + broad_slope * 0.3) * growth, turf);
    }
    // Relief and cavities disappear gradually as the texture footprint grows. This keeps
    // distant gravel, snow crystals and brushed metal from turning into sparkling noise.
    let rho = max(length(dx), length(dy));
    let relief_filter = 1.0 - smoothstep(tile / 48.0, tile / 10.0, rho);
    // (Limited by its length, not per axis: clamped per axis, the steepest texels' normals all
    // snapped to the four diagonals, a grid of scales in the sun.)
    slope *= min(1.0, 0.9 / max(length(slope), 1e-4));
    return vec4<f32>(bright, slope * relief_filter, (0.5 - height) * relief_filter);
}

// How much the art at `a` is the map's plain ground (1) rather than a feature, a road, a
// marking or a hill painted on it (0): how near its tile's mean colour is to a plain tile's.
fn plain_ground(a: vec2<f32>) -> f32 {
    let count = min(plain_tiles.count.x, 16u);
    if (count == 0u) {
        return 0.0;
    }
    let s = sqrt(max(art_sample(a, 5.0, 0.0), vec3<f32>(0.0)));
    var d = 1e9;
    for (var i = 0u; i < count; i++) {
        d = min(d, distance(s, plain_tiles.tiles[i].rgb));
    }
    return 1.0 - smoothstep(0.025, 0.07, d);
}

// `c` drawn `k` of the way towards a tone of the world's look (its tint and value multiplying
// `c`): the art's grain and the details survive, per pixel, and sand stays sand.
fn toward_look(c: vec3<f32>, look: vec4<f32>, k: f32) -> vec3<f32> {
    return mix(c, c * look.rgb * look.a, k);
}

// The world's broad look over colour `c` of the open ground at `xz` (`footprint` world units a
// pixel; `beyond` units past the map, where the relief's terms fade out): patches of drier and
// darker ground hundreds of units across, the dark ones gathering in hollows (or winding
// flows), wider regions lighter or darker, and a band by the water or the melt. All in world
// space, continuous, so it runs on past the map's edge.
fn macro_ground(c: vec3<f32>, xz: vec2<f32>, footprint: f32, beyond: f32) -> vec3<f32> {
    let g = detail_world.look[4];
    let strength = detail_world.look[3];
    // Hollows (positive) and crests: the heightmap's coarse mip less a fine one, a difference of
    // Gaussians; the share of the ground under the sea within ten cells or so, not here.
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(params.cells);
    let fade = 1.0 - smoothstep(0.0, 128.0, beyond);
    let fine_h = textureSampleLevel(heightmap, linear_clamp, uv, 1.0);
    let hollow = clamp((textureSampleLevel(heightmap, linear_clamp, uv, 4.0).r - fine_h.r) * 255.0 / 10.0, -1.0, 1.0) * fade;
    let shore = smoothstep(0.02, 0.35, textureSampleLevel(heightmap, linear_clamp, uv, 3.5).g) * (1.0 - fine_h.g) * fade;
    let region = noise(xz / g.w) + 0.5 * noise(xz / (0.45 * g.w) + vec2<f32>(3.7, 1.3));
    let warp = vec2<f32>(noise(xz / (1.6 * g.z) + vec2<f32>(5.1, 2.2)), noise(xz / (1.6 * g.z) + vec2<f32>(-3.3, 8.4))) * 0.6 * g.z;
    // (Its finer octave only where it is wide enough on the screen not to shimmer.)
    let fine = 1.0 - smoothstep(0.03 * g.z, 0.1 * g.z, footprint);
    let patches = noise((xz + warp) / g.z) + 0.45 * fine * noise((xz + warp) / (0.35 * g.z) + vec2<f32>(1.9, -4.6));
    // (Soft-edged: patches fade in over most of their width.)
    var dry = smoothstep(-0.05, 0.4, patches + 0.6 * region - 0.4 * hollow);
    var dark = smoothstep(-0.02, 0.4, -patches - 0.4 * region + 0.7 * hollow);
    if (strength.w > 0.5) {
        let ridge = 1.0 - 2.2 * abs(noise((xz - warp) / (2.2 * g.z) + vec2<f32>(-7.7, 2.9)));
        dark = smoothstep(0.55, 0.95, ridge + 0.12 * hollow + 0.08 * region);
        dry *= 1.0 - dark;
    }
    var o = toward_look(c, detail_world.look[0], dry * strength.x);
    o = toward_look(o, detail_world.look[1], dark * strength.y);
    o = toward_look(o, detail_world.look[2], shore * (1.0 - 0.5 * dark) * strength.z);
    return o * (1.0 + g.y * (0.6 * region + 0.4 * patches));
}

// Each material's colour in the check view (`TA_GROUND_DEBUG=1`).
fn material_check_colour(m: u32) -> vec3<f32> {
    var c = array<vec3<f32>, 10>(
        vec3<f32>(1.0, 0.85, 0.3), vec3<f32>(0.45, 0.25, 0.1), vec3<f32>(0.1, 0.8, 0.1), vec3<f32>(0.5, 0.5, 0.55), vec3<f32>(0.8, 0.4, 0.8),
        vec3<f32>(1.0, 1.0, 1.0), vec3<f32>(0.3, 0.8, 1.0), vec3<f32>(0.9, 0.1, 0.05), vec3<f32>(0.2, 0.3, 0.9), vec3<f32>(0.75, 0.75, 0.6),
    );
    return c[m];
}

// How a fragment's art is filtered: mip level, sharpening and world units per pixel. From
// screen-space derivatives, so taken before any discard.
struct ArtFilter {
    lod: f32,
    sharp: f32,
    footprint: f32,
    // Art pixels per screen pixel (below 1: magnified), along the more squeezed of the two
    // ways; and how much more squeezed that way is than the other (1 face on; large where the
    // view grazes the ground, as from a unit's own eyes).
    rho: f32,
    aniso: f32,
    // The world position's change per screen pixel, across and down.
    dwx: vec3<f32>,
    dwy: vec3<f32>,
};

fn art_filter(in: VsOut) -> ArtFilter {
    let dx = dpdx(in.art_px);
    let dy = dpdy(in.art_px);
    let rho = max(length(dx), length(dy));
    let least = min(length(dx), length(dy));
    var f: ArtFilter;
    f.lod = clamp(log2(max(rho, 1e-6)), 0.0, 12.0);
    // Only sharpen when magnified (less than one art pixel per screen pixel).
    f.sharp = params.detail * clamp(1.0 - rho, 0.0, 1.0);
    f.footprint = max(length(dpdx(in.world.xz)), length(dpdy(in.world.xz)));
    f.rho = rho;
    f.aniso = rho / max(least, 1e-6);
    f.dwx = dpdx(in.world);
    f.dwy = dpdy(in.world);
    return f;
}

// The ground under a map overlay: the overlay texture's colours, or (mode 2) heights as a
// colour ramp from the map's lowest ground to its highest with a contour line every 16 units
// and a heavier one every 64, anti-aliased by the height's screen-space rate. A line is drawn
// where the ground crosses its level, not over ground lying flat on one (TA's seas lie at 0 for
// acres: the whole floor was blackened, one-unit bumps on it standing out as stripes), and
// none at 0, the lowest ground there is.
fn map_overlay_color(lit: vec3<f32>, world: vec3<f32>, footprint: f32) -> vec3<f32> {
    if (frame.detail.z < 1.5) {
        let uv = world.xz / (vec2<f32>(params.cells) * 16.0);
        let o = textureSampleLevel(map_overlay, linear_clamp, uv, 0.0);
        return mix(lit, o.rgb, o.a);
    }
    let lo = frame.world.y - 32.0;
    let t = saturate((world.y - lo) / 200.0);
    // (Deeper than the ramp's foot, darker down to 0, the lowest ground there can be.)
    let deep = mix(0.55, 1.0, saturate(world.y / max(lo, 1.0)));
    let ramp = deep * mix(mix(vec3<f32>(0.05, 0.25, 0.55), vec3<f32>(0.2, 0.6, 0.25), smoothstep(0.0, 0.35, t)), mix(vec3<f32>(0.75, 0.6, 0.25), vec3<f32>(0.95, 0.95, 0.95), smoothstep(0.65, 1.0, t)), smoothstep(0.3, 0.7, t));
    let rate = max(fwidth(world.y), 1e-3);
    // (Flat ground's height does not change at all across a pixel; TA's gentlest slope is a
    // unit over a 16-unit cell, seen edge-on a few times less.)
    let sloped = smoothstep(0.001, 0.01, fwidth(world.y) / max(footprint, 1e-3));
    let minor_level = round(world.y / 16.0);
    let major_level = round(world.y / 64.0);
    let minor = (1.0 - smoothstep(0.0, rate * 1.2, abs(world.y - minor_level * 16.0))) * sloped * step(0.5, minor_level);
    let major = (1.0 - smoothstep(0.0, rate * 1.6, abs(world.y - major_level * 64.0))) * sloped * step(0.5, major_level);
    var c = mix(lit, ramp * (0.35 + 0.65 * luminance(lit) / max(luminance(lit), 0.05)), 0.6);
    c = mix(c, vec3<f32>(0.02), minor * 0.35 + major * 0.5);
    return c;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return ground_color(in, art_filter(in), false);
}

// The ground mirrored in the water (the reflection pass, see reflection.rs): only what lies
// above the water, where the water can show it.
@fragment
fn fs_reflection(in: VsOut) -> @location(0) vec4<f32> {
    let art = art_filter(in);
    if (!seen_in_water(in.world)) {
        discard;
    }
    return ground_color(in, art, true);
}

// How nearly level the view is at `p` (perspective camera): 0 from TA's elevation and the usual
// RTS heights, 1 within a few degrees of the ground. (Both ways: the reflection pass's eye is
// mirrored below the water.)
fn low_view(p: vec3<f32>) -> f32 {
    if (frame.camera_pos.w < 0.5) {
        return 0.0;
    }
    return 1.0 - smoothstep(0.3, 0.6, abs(view_dir(p).y));
}

// How many world units across a metal deposit's stones of ore are (see the ore bed in
// `ground_color`).
const ORE_STONE: f32 = 5.0;

// The stones of an ore bed at `p` (in stones): cells round random points, most of them holding a
// lump of ore of its own size (not a paving of cells: rubble round and between them). x: how high
// a lump is here (1 over most of it, 0 in the rubble and the crevices between lumps), yz: its
// slope here (outwards from its middle, steeper towards its edge), w: a random value of its own.
fn ore_stones(p: vec2<f32>) -> vec4<f32> {
    let cell = floor(p);
    var f1 = 9.0;
    var f2 = 9.0;
    var middle = p;
    var own = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = cell + vec2<f32>(f32(i), f32(j));
            let h = hash_u32(bitcast<u32>(i32(c.x)) ^ hash_u32(bitcast<u32>(i32(c.y)) + 0x51ED27u));
            let at = c + 0.1 + 0.8 * vec2<f32>(f32(h & 0xFFFu), f32((h >> 12u) & 0xFFFu)) / 4095.0;
            let d = distance(p, at);
            if (d < f1) {
                f2 = f1;
                f1 = d;
                middle = at;
                own = f32(h >> 24u) / 255.0;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    let size = 0.3 + 0.4 * fract(own * 7.3);
    let height = smoothstep(0.0, 0.2, f2 - f1) * smoothstep(size, size - 0.15, f1) * step(0.25, own);
    let slope = (p - middle) / max(f1, 1e-3) * smoothstep(0.05, size, f1) * height;
    return vec4<f32>(height, slope, own);
}

// ---- The ground in the wind (perspective camera; `Lighting::ground_life`) ------------------------
//
// The painted ground stood still. With the perspective camera, grass, sand and snow move in the
// map's wind (`frame.wind`), carried by `frame.drift` as the gusts over the trees are (`wind_bend`:
// the swell crossing a field is the gust that bends the trees standing in it): over grass cat's
// paws run downwind, patches where the blades bend over and show their paler undersides; sand
// smokes with thin strands of blown grains in the gusts and, now and then, a veil of dust; snow
// drifts as powder. All of it is laid out in the world and carried by the drift, never along the
// wind's heading, which the map rolls anew every eight seconds (a pattern laid out along it swings
// round the map's corner as the wind turns, however slowly it is eased): the strands lie along one
// of sixteen fixed headings, the two nearest the wind's, and a turn of the wind rolls through the
// map as a front, place by place, from the one to the next.
//
// The slow part of it (gusts, the packets of cat's paws, dust: hundreds of units across) is found
// at the mesh's vertices (`life_fields`, sixteen units apart) and interpolated; only what is fine,
// the strands and the paws themselves, is found for each pixel.

// The slow fields at a point of the ground: a = the grass's swell (the trees' gusts), the packets
// of its cat's paws, (free) and where the wind's turn has got to; b = the sand's gusts and dust,
// the snow's gusts and drifts. Each 0..1, as `gust_noise`.
struct LifeFields {
    a: vec4<f32>,
    b: vec4<f32>,
};

fn life_fields(xz: vec2<f32>) -> LifeFields {
    let d = frame.drift.xy;
    var f: LifeFields;
    f.a = vec4<f32>(
        gust_noise((xz - d * GUST_CARRY) / GUST_SIZE),
        gust_noise((xz - d * (0.6 * GUST_CARRY)) / 190.0 + vec2<f32>(8.2, 1.4)),
        0.0,
        gust_noise(xz / 260.0 + vec2<f32>(6.1, 2.3)),
    );
    f.b = vec4<f32>(
        gust_noise((xz - d * GUST_CARRY) / (0.6 * GUST_SIZE) + vec2<f32>(11.0, 3.0)),
        gust_noise((xz - d * (0.35 * GUST_CARRY)) / 170.0 + vec2<f32>(5.0, 9.0)),
        gust_noise((xz - d * GUST_CARRY) / (0.7 * GUST_SIZE) + vec2<f32>(4.0, 13.0)),
        gust_noise((xz - d * (0.4 * GUST_CARRY)) / 210.0 + vec2<f32>(2.0, 6.0)),
    );
    return f;
}

struct Life {
    // The paint's light and shade, a factor.
    shade: f32,
    // The tone of what blows over the paint (a factor on its colour), and how much it covers.
    tone: vec3<f32>,
    cover: f32,
    // The surface's tilt (its slope along x and z), catching the sun.
    tilt: vec2<f32>,
};

fn still_ground() -> Life {
    return Life(1.0, vec3<f32>(1.0), 0.0, vec2<f32>(0.0));
}

// A sum of lives to start from (`add_life`, `total_life`): where the ground is taken for a mix of
// materials each moves it by its share. The tone is kept as what each covers times its tone.
fn no_life() -> Life {
    return Life(1.0, vec3<f32>(0.0), 0.0, vec2<f32>(0.0));
}

fn add_life(sum: Life, l: Life, share: f32) -> Life {
    return Life(sum.shade + share * (l.shade - 1.0), sum.tone + share * l.cover * l.tone, sum.cover + share * l.cover, sum.tilt + share * l.tilt);
}

fn total_life(sum: Life) -> Life {
    return Life(sum.shade, select(vec3<f32>(1.0), sum.tone / max(sum.cover, 1e-4), sum.cover > 1e-4), sum.cover, sum.tilt);
}

// Sixteen headings, a sixteenth of a turn apart: what waves and strands lie along.
const HEADINGS = array<vec2<f32>, 16>(
    vec2<f32>(1.0, 0.0), vec2<f32>(0.92388, 0.38268), vec2<f32>(0.70711, 0.70711), vec2<f32>(0.38268, 0.92388),
    vec2<f32>(0.0, 1.0), vec2<f32>(-0.38268, 0.92388), vec2<f32>(-0.70711, 0.70711), vec2<f32>(-0.92388, 0.38268),
    vec2<f32>(-1.0, 0.0), vec2<f32>(-0.92388, -0.38268), vec2<f32>(-0.70711, -0.70711), vec2<f32>(-0.38268, -0.92388),
    vec2<f32>(0.0, -1.0), vec2<f32>(0.38268, -0.92388), vec2<f32>(0.70711, -0.70711), vec2<f32>(0.92388, -0.38268),
);

// The two fixed headings the wind lies between, and how far its turn from the first to the second
// has rolled through a place (0 not at all, 1 through): each place turns when the turn passes its
// own mark (`mark_noise`: the fields' fourth), marks scattered over the map in patches a few
// hundred units across.
struct Heading {
    a: vec2<f32>,
    b: vec2<f32>,
    turned: f32,
};

fn wind_heading(mark_noise: f32) -> Heading {
    let turns = atan2(frame.wind.y, frame.wind.x) * (16.0 / TAU);
    let base = floor(turns);
    let k = u32(i32(base) & 15);
    let mark = 0.12 + 0.76 * smoothstep(0.15, 0.85, mark_noise);
    return Heading(HEADINGS[k], HEADINGS[(k + 1u) & 15u], smoothstep(mark - 0.1, mark + 0.1, turns - base));
}

// A strand field along heading `d`: soft lines `width` units across and about `length` long, at the
// carried position `at` (0..1).
fn strand_field(at: vec2<f32>, d: vec2<f32>, width: f32, length: f32, seed: vec2<f32>) -> f32 {
    let uv = vec2<f32>(dot(at, d) / length, dot(at, vec2<f32>(-d.y, d.x)) / width) + seed;
    let v = smoothstep(0.5, 0.92, gust_noise(uv));
    return v * v;
}

// Strands of blown grains at `xz`: lines along the wind's heading (the two nearest, where the wind
// is turning), carried `carry` times as far as the wind's drift.
fn strands(xz: vec2<f32>, h: Heading, width: f32, length: f32, carry: f32, seed: vec2<f32>) -> f32 {
    let at = xz - frame.drift.xy * carry;
    var s = 0.0;
    if (h.turned < 1.0) {
        s += (1.0 - h.turned) * strand_field(at, h.a, width, length, seed);
    }
    if (h.turned > 0.0) {
        s += h.turned * strand_field(at, h.b, width, length, seed);
    }
    return s;
}

// The cat's paws on grass (see `grass_life`): how far across their noise's lattice is (units; the
// patches come out about as wide), and how many times as fast as the wind's drift they are carried
// (some seventy units a second in a moderate wind, a hundred in a gale).
const PAW_SIZE: f32 = 110.0;
const PAW_CARRY: f32 = 70.0;

// Grass: in the gusts (the trees' own, `f.a.x`: the swell crossing a field is the gust bending the
// trees standing in it) cat's paws run downwind, soft patches some hundred units across where the
// blades bend over and show their paler, yellower undersides, in packets with calm between
// (`f.a.y`). No rows of waves, and nothing laid out along the wind's heading: crests in rows over
// a whole field read as a fault of the screen, and two sets of them crossing while the wind turned
// (the more often, the faster the game runs) as moiré. Gentle: a few hundredths of the paint's
// light at the most, and no tilt (the sun catching rows of leaning blades drew bands of light and
// shade across the field).
fn grass_life(xz: vec2<f32>, f: LifeFields, footprint: f32, wind: f32) -> Life {
    let swell = smoothstep(0.25, 0.85, f.a.x);
    let packets = 0.3 + 0.7 * smoothstep(0.3, 0.75, f.a.y);
    // (Two octaves of gradient noise, round where the gusts' value noise drew squares, each turned
    // off the map's axes so that no lattice shows, the finer carried a little faster than the
    // broader, so the patches change shape as they go; a fifth or so of the ground in them.)
    let broad = mat2x2<f32>(0.878, 0.479, -0.479, 0.878) * (xz - frame.drift.xy * PAW_CARRY) / PAW_SIZE;
    let fine = mat2x2<f32>(0.358, -0.934, 0.934, 0.358) * (xz - frame.drift.xy * (1.35 * PAW_CARRY)) / (0.45 * PAW_SIZE);
    let n = 0.65 * noise(broad) + 0.35 * noise(fine + vec2<f32>(7.3, 2.9));
    let paw = smoothstep(0.08, 0.3, n) * packets * (1.0 - smoothstep(12.0, 24.0, footprint));
    let k = 0.5 + 0.5 * min(wind, 1.0);
    var l = still_ground();
    l.shade = 1.0 + k * (0.035 * paw * (0.4 + 0.6 * swell) + 0.012 * (swell - 0.5));
    l.tone = vec3<f32>(1.05, 1.05, 0.9);
    l.cover = 0.3 * k * paw * swell;
    return l;
}

// Sand: strands of blown grains in the gusts, and veils of dust now and then.
fn sand_life(xz: vec2<f32>, h: Heading, f: LifeFields, footprint: f32, wind: f32) -> Life {
    let gust = smoothstep(0.25, 0.7, f.b.x);
    var l = still_ground();
    if (gust <= 0.0) {
        return l;
    }
    let dry = 1.0 - frame.rain.x;
    // Fade fine streaks earlier, before they turn into flickering pixels as the view pulls out.
    let thin = 1.0 - smoothstep(0.2, 0.65, footprint / 5.0);
    let long_strands = strands(xz, h, 5.0, 130.0, 0.85 * GUST_CARRY, vec2<f32>(2.7, 8.1));
    let short_strands = strands(xz, h, 9.0, 60.0, 1.3 * GUST_CARRY, vec2<f32>(6.3, 1.9));
    let strand = max(long_strands, 0.6 * short_strands) * gust * thin * (0.6 + 0.4 * wind);
    let veil = smoothstep(0.45, 0.85, f.b.y) * gust * (1.0 - smoothstep(30.0, 60.0, footprint)) * wind;
    l.tone = vec3<f32>(1.6, 1.48, 1.2);
    // Blown sand stays a faint veil over the painted ground, even in the strongest gusts.
    l.cover = 0.15 * saturate(strand * 1.5 + veil * 0.45) * dry;
    return l;
}

// Snow: powder blown along the surface, finer and longer than sand, and drifts of it in the gusts.
fn snow_life(xz: vec2<f32>, h: Heading, f: LifeFields, footprint: f32, wind: f32) -> Life {
    let gust = smoothstep(0.25, 0.7, f.b.z);
    var l = still_ground();
    if (gust <= 0.0) {
        return l;
    }
    let dry = 1.0 - frame.rain.x;
    let thin = 1.0 - smoothstep(0.4, 1.1, footprint / 7.0);
    let long_strands = strands(xz, h, 7.0, 240.0, 0.7 * GUST_CARRY, vec2<f32>(8.3, 1.1));
    let short_strands = strands(xz, h, 12.0, 110.0, 1.1 * GUST_CARRY, vec2<f32>(1.7, 5.9));
    let strand = max(long_strands, 0.6 * short_strands) * gust * thin * (0.6 + 0.4 * wind);
    let veil = smoothstep(0.4, 0.85, f.b.w) * gust * (1.0 - smoothstep(30.0, 60.0, footprint)) * wind;
    l.shade = 1.0 - 0.06 * gust * min(wind, 1.0) * dry;
    l.tone = vec3<f32>(1.35, 1.4, 1.5);
    l.cover = saturate(strand * 1.7 + veil * 0.5) * dry;
    return l;
}

// The ground's colour at a fragment; `mirror` in the water's reflection (the swash zone's foam,
// a sliver lying on the beach, is left out of it there).
fn ground_color(in: VsOut, art: ArtFilter, mirror: bool) -> vec4<f32> {
    // How far beyond the map the ground lies (0 on the map): only the skirt's shader has ground
    // there (`FAR_LAND`), and the map's leaves out everything beyond it.
    let in_beyond = select(0.0, in.beyond, FAR_LAND);
    var lod = art.lod;
    var sharp = art.sharp;
    let footprint = art.footprint;
    // Art style is independent of projection. HD quality still gates expensive texture
    // reconstruction and relief; the inexpensive palette finish below works at every preset.
    let reborn = saturate(frame.ao.w);
    // (Mirrored in the water the ground is wobbled by the waves: its detail up close, the HD
    // surfaces and materials, is left out there.)
    let finish = select(saturate(frame.detail.x), 0.0, mirror);

    // Under water the painted sea bed catches caustics (fading out towards the waterline, so the
    // painted shoreline stays exactly where it was drawn and the classic view keeps the original
    // art); the water bends it with the waves (see `refracted` in water.wgsl).
    let t = frame.world.x;
    let depth = frame.world.y - in.world.y;
    let water_on = frame.water.w > 0.5;
    // (The extended ground's sea bed only gets them right next to the map.)
    let water_fx = frame.water.w * frame.water_extra.x * (1.0 - smoothstep(16.0, 64.0, in_beyond));
    let art_px = in.art_px;
    var caustic = 0.0;
    // How much the water between the eye and the bed softens it: light scattered in the water
    // blurs the painted bed more the deeper it lies (TA's painted shallows lose their hard
    // outer contours); its fine detail goes too. (Over a sixteenth of the depth, in world units,
    // so only where that is wider than the pixels, and by a mip level or so at most: TA painted
    // its shallows over beds the heights put tens of units down, and blurred by two mip levels
    // on top of the pixels' at every zoom, Gods of War's painted lilac shallows were pink smoke
    // lying on the water rather than a sea floor seen through it.)
    var murk = 0.0;
    if (water_fx > 0.0 && depth > 0.0) {
        murk = smoothstep(0.0, 14.0, depth) * water_fx;
        let blur = min(log2(max(depth / 16.0, 1e-3)), lod + select(0.6, 1.2, beyond_on()));
        lod = max(lod, mix(lod, blur, water_fx));
        sharp *= 1.0 - murk;
        // Caustics: focused sunlight on the shallow shelf, strongest a unit or few down, faded
        // out by 26.
        let band = smoothstep(0.5, 3.0, depth) * exp(-depth / 10.0) * (1.0 - smoothstep(14.0, 26.0, depth));
        if (band > 0.0) {
            caustic = caustics(in.world.xz, liquid_time(t), footprint, depth) * band * water_fx;
        }
    }
    // Swash zone: the strip of ground (often painted water) that lies just above sea level gets
    // the same shoreline foam as the water surface continues with beyond the waterline (the
    // painted shoreline only means something on the map and right next to it). Only where the
    // ground lies nearly flat, as a beach does, or where the view looks down on it (see
    // `foam_from_above`): seen from low down, a bank steeper than ~30 degrees stands like a
    // wall, where foam (anchored in world xz) would be drawn as white bars standing up the
    // rock; there the water's own foam at the bank's foot marks the waterline.
    var foam = vec2<f32>(0.0);
    // (And as far under the water as the ground falls across a pixel: with antialiasing a pixel
    // the waterline crosses is shaded once for the ground and once for the water, each at its
    // centre. Where that lay under the water the ground had no foam, so neither had its samples
    // on the land side, which the water does not cover: a dotted line of bare ground ran along
    // the waterline between the ground's surf and the water's.)
    let seam = min(max(abs(art.dwx.y), abs(art.dwy.y)), 2.0);
    if (water_on && frame.water.y > 0.0 && depth <= seam && depth > -13.0 && in_beyond < 64.0 && !mirror) {
        // (The distance from the waterline, as on the water.)
        let waterline = shore_distance(in.world.xz);
        let slope = ground_gradient(in.world.xz);
        let rise = max(length(slope), 0.03);
        let above = foam_from_above(view_dir(in.world).y);
        let strip = foam_strip(above);
        let lies = foam_runs_up(rise, above) * smoothstep(-strip, -strip + 1.5, depth);
        if (lies > 0.0 && waterline > -SURF_REACH) {
            let seaward = -slope / rise;
            let band = length(vec2<f32>(dot(seaward, art.dwx.xz), dot(seaward, art.dwy.xz)));
            let f = shore_foam(in.art_px, in.world.xz, waterline, rise, t, FoamPixel(art.dwx.xz, art.dwy.xz, max(band, 0.25 * footprint), above));
            foam = vec2<f32>(f.x * frame.water.y * lies * (1.0 - smoothstep(16.0, 64.0, in_beyond)), f.y);
        }
    }
    // The art shown: beyond the map, the mirrored ground's (see `shown_art`), with its detail
    // (materials, HD surfaces, rock, occlusion) as on the map near it, gone where it blurs or the
    // far land takes over (a thousand units or so out).
    let beyond = select(0.0, beyond_distance(in.world.xz), in_beyond > 0.0);
    let shown = shown_art(art_px, in.world.xz, beyond);
    var detail_k = art_edge_fade(art_px);
    if (beyond_on()) {
        detail_k = (1.0 - smoothstep(0.25, 1.0, shown.blur)) * (1.0 - shown.far);
    }
    // (How deep the water drawn over this ground lies, see `beyond_ground_depth`.)
    var water_depth = depth;
    if (beyond > 0.0 && frame.world.y > 0.0) {
        water_depth = beyond_ground_depth(in.world.xz, in.world.y, beyond, shown);
    }
    var albedo = ground_albedo(art_px, lod, sharp, in.world.xz, beyond, in.world.y, shown, water_depth);
    // The far land's own surface (see `far_surface`), on its dry ground: its shade here, its
    // relief with the walls' below.
    var far_surf = FarSurface(1.0, vec2<f32>(0.0));
    var far_k = 0.0;
    if (shown.far > 0.0) {
        far_k = shown.far * (1.0 - smoothstep(0.5, 6.0, water_depth) * f32(frame.world.y > 0.0 && water_on));
        if (far_k > 0.0) {
            far_surf = far_surface(in.world.xz, art.dwx.xz, art.dwy.xz);
            albedo *= mix(1.0, far_surf.shade, far_k);
        }
    }
    // Lava worlds: melt TA painted on ground a little above the lava's level (the heightmap's
    // waterline falls short of the painted shore), which the water pass cannot draw: it is drawn
    // here, as the surface is (see `lava_melt`), not left as red paint smeared over the bank.
    var dry_melt = 0.0;
    if (water_on && frame.water_extra.y > 0.5 && frame.shore_land.a > 0.0 && in_beyond <= 0.0 && depth <= 0.0 && depth > -12.0) {
        dry_melt = lava_melt(in.art_px, depth, footprint);
    }
    let painted_albedo = albedo;
    // By moonlight colours fade (see daylight.rs).
    if (frame.night.a > 0.0) {
        albedo = mix(albedo, vec3<f32>(luminance(albedo)), 0.5 * frame.night.a);
    }
    // Rain (perspective camera): the ground darkens as it soaks, its colours deepening (a film
    // of water scatters less white light off it), and water stands in its low, flat places
    // (puddles: blotches of noise, more of them in hollows and the longer it rains). (Not the
    // sea bed: the water surface has its own rain rings.)
    let wet = frame.rain.x * (1.0 - smoothstep(64.0, 400.0, in_beyond)) * (1.0 - smoothstep(0.0, 0.5, depth) * f32(water_on));
    var puddle = 0.0;
    if (wet > 0.0) {
        let n0 = normalize(in.normal);
        // (Water stands only on ground near level: a puddle on a hillside, seen across the
        // slope, was a pale sheet lying on it.)
        let flat = 1.0 - smoothstep(0.005, 0.025, 1.0 - n0.y);
        let low = 1.0 - terrain_occlusion(in.world.xz, in_beyond);
        let blotch = noise(in.world.xz / 70.0) * 0.55 + noise(in.world.xz / 23.0 + vec2<f32>(3.1, 7.3)) * 0.45;
        // (Grass drinks the rain: few puddles stand in it. Judged by the art's colour over a few
        // of its pixels: by each one's, every greener pixel of the painting holed the puddle, a
        // pepper of dark specks.)
        let around = shown_color(art_sample(shown.px, lod + 2.0, 0.0), shown);
        let grass = smoothstep(0.02, 0.1, around.g - max(around.r, around.b));
        let pool = blotch * 0.5 + 0.5 + low * 1.6 + (wet - 1.0) * 0.35;
        let gate = flat * smoothstep(0.35, 0.85, wet) * (1.0 - 0.8 * grass);
        puddle = smoothstep(0.6, 0.68, pool) * gate;
        // (Round each, a band of soaked ground.)
        let margin = smoothstep(0.5, 0.6, pool) * gate * (1.0 - puddle);
        albedo *= mix(1.0, 0.68, wet) * mix(1.0, 0.55, puddle) * mix(1.0, 0.8, margin);
        albedo = max(mix(vec3<f32>(luminance(albedo)), albedo, 1.0 + 0.3 * wet * (1.0 - puddle)), vec3<f32>(0.0));
    }
    // Walls (perspective camera, see `wall_surface`): faces TA's art cannot paint, edge-on to or
    // hidden from its oblique camera (`painted`: n . (0, 2, 1) / 2, 1 on level ground, 0 edge-on),
    // and faces steep whatever their facing. From the bilinear heightmap's slope, so a step a
    // cell wide is measured at its full slope (the vertex normals halve it) and the weight has no
    // seams along the triangles; beyond the map from the mirrored ground's own (at the mip the
    // extended ground's vertices carry there, turned as the mirror turns it: the vertex normals,
    // measured over two cells, halved a step's slope, and the mirrored towers' and cliffs' sides
    // were drawn with the paint stretched down them, not as walls), facing as the face it mirrors.
    var n = normalize(in.normal);
    var stretched = 0.0;
    var steep = 0.0;
    var painted = 1.0;
    var rise = 0.0;
    if (beyond_on()) {
        var gs: vec2<f32>;
        if (in_beyond <= 0.0) {
            gs = smooth_gradient(in.world.xz);
        } else if (shown.far < 1.0) {
            // (The far land's own slopes are its vertices'.)
            gs = mip_gradient(shown.at, beyond_lod(beyond)) * shown.mirrored * (1.0 - shown.far);
        } else {
            gs = vec2<f32>(0.0);
        }
        let inv = inverseSqrt(1.0 + dot(gs, gs));
        painted = (1.0 - 0.5 * gs.y * shown.mirrored.y) * inv;
        stretched = 1.0 - smoothstep(0.22, 0.45, painted);
        steep = 1.0 - smoothstep(0.38, 0.6, inv);
        rise = 16.0 * length(gs);
        n = normalize(mix(n, vec3<f32>(-gs.x, 1.0, -gs.y) * inv, max(stretched, steep)));
    }
    let wall = max(stretched, steep);
    if (far_k > 0.0) {
        n = normalize(n + vec3<f32>(-far_surf.slope.x, 0.0, -far_surf.slope.y) * far_k);
    }
    // The ground's material (see `ground_material`) as the view comes close: by how magnified
    // the art is across the view (seen grazing, as from a unit's own eyes, the ground is near
    // enough to show it, and the anisotropic filter keeps it steady), none past the filter's
    // reach.
    let least = art.rho / max(art.aniso, 1.0);
    // (On the far land too, judged on its own colours: it has no art of its own.)
    var mat_k = finish * (1.0 - smoothstep(mix(0.35, 0.55, reborn), mix(1.0, 2.25, reborn), least))
        * (1.0 - smoothstep(6.0, 8.0, art.aniso)) * max(detail_k, shown.far) * (1.0 - wall) * (1.0 - murk) * (1.0 - dry_melt);
    var mat_d = vec4<f32>(0.0);
    var mat_grain = 0.0;
    var mat_smooth = 0.0;
    var mat_check = vec4<f32>(0.0);
    var mat_tint = vec3<f32>(1.0);
    var mat_rough = 0.9;
    var mat_metal = 0.0;
    var mat_ice = 0.0;
    // How the ground moves in the wind here (perspective camera; see `ground_life`).
    var life = still_ground();
    // How much of the material's relief the pixel still resolves (see `material_detail`).
    var mat_detail_kept = 1.0;
    // Magnified well past its own pixels (the closest zoom, the Archives' stage), TA's paint has
    // no pattern left to keep, only soft blots: Enhanced Classic takes most of Reborn's grain
    // there too, or the ground under a unit is a cloudy smear.
    let grain_style = max(reborn, 0.65 * smoothstep(0.5, 0.15, art.rho));
    if (mat_k > 0.0) {
        var seen = painted_albedo;
        if (shown.far < 1.0) {
            seen = mix(shown_color(art_soft(shown.px, 3u), shown), painted_albedo, shown.far);
        }
        let gm = ground_material(seen, 1.0 - n.y);
        if (detail_world.flags.y > 0.5) {
            mat_check = vec4<f32>(material_check_colour(gm.a), gm.sure * mat_k);
        }
        mat_k *= gm.sure;
        if (mat_k > 0.0) {
            mat_d = material_detail(gm.a, in.world.xz, art.dwx.xz, art.dwy.xz);
            let tile_a = MATERIAL_TILE[gm.a];
            mat_detail_kept = 1.0 - smoothstep(tile_a / 48.0, tile_a / 10.0, max(length(art.dwx.xz), length(art.dwy.xz)));
            if (gm.mix_b > 0.02) {
                mat_d = mix(mat_d, material_detail(gm.b, in.world.xz, art.dwx.xz, art.dwy.xz), gm.mix_b);
            }
            mat_grain = mix(MATERIAL_GRAIN[gm.a], MATERIAL_GRAIN[gm.b], gm.mix_b);
            mat_smooth = mix(MATERIAL_SMOOTH[gm.a], MATERIAL_SMOOTH[gm.b], gm.mix_b);
            mat_tint = mix(MATERIAL_TINT[gm.a], MATERIAL_TINT[gm.b], gm.mix_b);
            mat_rough = mix(MATERIAL_ROUGHNESS[gm.a], MATERIAL_ROUGHNESS[gm.b], gm.mix_b);
            mat_metal = mix(select(0.0, 0.8, gm.a == 8u), select(0.0, 0.8, gm.b == 8u), gm.mix_b);
            mat_ice = mix(select(0.0, 1.0, gm.a == 6u), select(0.0, 1.0, gm.b == 6u), gm.mix_b);
            if (frame.rain.z > 0.0 && mat_k > 0.02) {
                let wind = clamp(length(frame.wind.xy) / 0.8, 0.35, 1.3);
                let fields = LifeFields(in.life_a, in.life_b);
                let h = wind_heading(fields.a.w);
                // (Each material's share of the ground, taken for a mix of two; soil as green as its
                // paint, the dark moss and turf of a meadow taken for bare earth, sways as grass.)
                let tint = sqrt(max(seen, vec3<f32>(0.0)));
                let green = smoothstep(-0.01, 0.04, tint.g - max(tint.r, tint.b));
                let ka = 1.0 - gm.mix_b;
                let kb = gm.mix_b;
                let grass_k = select(0.0, ka, gm.a == 2u) + select(0.0, kb, gm.b == 2u) + green * (select(0.0, ka, gm.a == 1u) + select(0.0, kb, gm.b == 1u));
                let sand_k = select(0.0, ka, gm.a == 0u) + select(0.0, kb, gm.b == 0u);
                let snow_k = select(0.0, ka, gm.a == 5u) + select(0.0, kb, gm.b == 5u);
                var sum = no_life();
                if (grass_k > 0.01) {
                    sum = add_life(sum, grass_life(in.world.xz, fields, footprint, wind), grass_k);
                }
                if (sand_k > 0.01) {
                    sum = add_life(sum, sand_life(in.world.xz, h, fields, footprint, wind), sand_k);
                }
                if (snow_k > 0.01) {
                    sum = add_life(sum, snow_life(in.world.xz, h, fields, footprint, wind), snow_k);
                }
                life = total_life(sum);
            }
        }
    } else {
        mat_k = 0.0;
    }
    // HD surfaces up close (see `hd_ground`) on the art shown: smooth colour, fine grain,
    // and a relief normal for the relighting below.
    var relief_n = n;
    let hd = finish * (1.0 - smoothstep(0.3, 1.2, art.rho)) * detail_k * (1.0 - murk);
    if (hd > 0.0) {
        let g = hd_ground(shown.px);
        // Very close (a unit's own eyes, the closest zoom): the dither, magnified tenfold and
        // more, is blots of colour; the colour comes from the art averaged over a few pixels
        // instead, and the texture from the grain, turned up.
        // (The painted speckle's brightness stays, as grain and pebbles; its colours, the
        // dither's rainbow, come from the art averaged over four pixels.)
        // Grazing (level ground seen from a unit's eyes): the art squeezed along the view, its
        // dither would stretch into stripes of colour that shimmer as the eye moves; the same
        // calm, and less of the grain.
        let close = smoothstep(0.5, 0.12, art.rho);
        let grazing = smoothstep(1.3, 2.8, art.aniso);
        let seam = select(0.0, seam_calm(in.world.xz, beyond), beyond_on());
        let calmed = max(max(close, grazing), seam);
        var color = shown_color(g.color, shown);
        if (calmed > 0.0) {
            let wide = shown_color(art_soft(shown.px, 2u), shown);
            let calm = wide * clamp(luminance(g.color) / max(luminance(wide), 1e-3), 0.4, 2.0);
            color = mix(color, mix(calm, wide, mix(0.12, 0.42, reborn) + 0.5 * grazing), calmed);
        }
        // Grain as strong as the painted speckle there (smooth painted ground stays smooth);
        // little of it where a material's own shows.
        let micro = micro_detail(art_px, art.rho) * g.energy * (1.0 + close * 1.3) * (1.0 - 0.6 * grazing)
            * (1.0 - 0.75 * mat_k) * mix(0.3, 0.55, grain_style);
        albedo = mix(albedo, color * (1.0 + micro.x * 0.9), hd);
        // Luminance as height (a bright painted rock face stands out, a dark crack sinks in)
        // plus the grain's own relief, as strong as the relighting of slopes
        // (`terrain_relight`, 0.35 by default); and the material's relief, in place of most of
        // the painted luminance's.
        // (Less where the paint is high-contrast linework, TA's drawn cracks and contour hatching:
        // as grooves lit by the sun they read as a web of scratches.)
        let linework = 1.0 - 0.5 * smoothstep(0.08, 0.2, g.energy);
        let relief = mix(0.75, 1.5, reborn) * linework * hd * clamp(frame.sun_color.a / 0.35, 0.0, 2.0) * (1.0 - 0.5 * mat_k) * (1.0 - wall);
        // (The art's own gradient turned round where the art shown runs backwards, and as
        // subdued as its contrast on the unpainted strip.)
        let slope = g.grad * shown.flip * shown.keep * (1.0 - seam) + micro.yz * 0.15;
        relief_n = normalize(n + vec3<f32>(-slope.x, 0.0, -slope.y) * relief);
    }
    if (mat_k > 0.0) {
        // The material's own grain takes the place of the dither's: the colour from the paint
        // averaged over four art pixels or so (the dither's rainbow gone), its light and shade
        // (and the rain's darkening) evened out only as much as the material calls for; its
        // relief lit as the slopes are (`terrain_relight`).
        var calm = painted_albedo;
        if (shown.far < 1.0) {
            calm = mix(shown_color(art_soft(shown.px, 2u), shown), painted_albedo, shown.far);
        }
        let calm_l = max(luminance(calm), 1e-3);
        let l = mix(luminance(albedo), calm_l, mat_k * mat_smooth * mix(0.15, 1.0, reborn));
        albedo = mix(albedo, calm / calm_l * l, mat_k * mix(0.12, 0.85, reborn));
        // Preserve the map's broad hue/value patterns and every strategic marking. The
        // physical finish only calms extreme pigments and supplies modest material tint.
        let physical = mix(vec3<f32>(luminance(albedo)), albedo, 0.87) * mat_tint;
        let preserved_value = physical * luminance(albedo) / max(luminance(physical), 1e-3);
        albedo = mix(albedo, preserved_value, reborn * mat_k * 0.72);
        albedo *= 1.0 + mat_d.x * mat_grain * mat_k * mix(0.16, 0.76, grain_style);
        albedo *= 1.0 - max(mat_d.w, 0.0) * reborn * mat_k * 0.15;
        let lit = clamp(frame.sun_color.a / 0.35, 0.0, 2.0);
        relief_n = normalize(relief_n + vec3<f32>(-mat_d.y, 0.0, -mat_d.z) * mat_k * lit * mix(0.45, 0.88, grain_style));
        // The ground in the wind: its blades and grains lighten and darken, what blows over it
        // covers the paint, and its tilt catches the sun. (Nothing in the water's reflection or
        // on an airless world; less in Enhanced Classic, which keeps the painted finish.)
        let moving = frame.rain.z * (1.0 - frame.celestial.w) * mix(0.55, 1.0, reborn) * mat_k;
        albedo *= mix(1.0, life.shade, moving);
        albedo = mix(albedo, albedo * life.tone, life.cover * moving);
        relief_n = normalize(relief_n + vec3<f32>(-life.tilt.x, 0.0, -life.tilt.y) * moving * lit);
    }
    // The world's broad look (Reborn; see `macro_ground`) over its open ground: none on
    // features, roads, markings and hills painted on it, on walls or under the water; and a
    // quiet physical palette (each world's desaturation). Enhanced Classic retains all of the
    // installed painting's original pigments.
    if (reborn > 0.0) {
        // (Past 64 units out, all of it: the art's plain ground is the map's.)
        var plain = 1.0;
        if (beyond < 64.0) {
            plain = mix(plain_ground(shown.px), 1.0, smoothstep(0.0, 64.0, beyond));
        }
        let open = reborn * select(plain, 0.0, mirror) * smoothstep(0.55, 0.8, n.y)
            * (1.0 - smoothstep(0.0, 3.0, depth)) * (1.0 - smoothstep(1500.0, 4000.0, beyond));
        if (open > 0.0 && !mirror) {
            albedo = mix(albedo, macro_ground(albedo, in.world.xz, footprint, beyond), open);
        }
        albedo = mix(albedo, vec3<f32>(luminance(albedo)), reborn * detail_world.look[4].x);
    }
    // A low shore seen nearly level: TA painted an island's outline at its rim, so the dry bank
    // below it carries painted sea, magnified down the bank into a band of blue. It shows the
    // painted beach instead, going darker and glossier towards the water. (Fading out past the
    // map's edge as the swash's foam does, and darker only by the waterline: Coast To Coast's
    // beach plains lie a unit or two above the sea far from it, and were darkened to a straight
    // line 64 units past the edge, the mirrored plain beyond it as painted.)
    var shore_wet = 0.0;
    let above = in.world.y - frame.world.y;
    if (water_on && frame.water_extra.y < 0.5 && frame.shore_land.a > 0.0 && in_beyond < 64.0 && above > 0.0 && above < 8.0) {
        let bank = low_view(in.world) * (1.0 - smoothstep(5.0, 8.0, above)) * (1.0 - smoothstep(16.0, 64.0, in_beyond));
        if (bank > 0.0) {
            let soft = art_soft(shown.px, 1u);
            let grain = clamp(luminance(albedo) / max(luminance(soft), 1e-3), 0.75, 1.25);
            let sand = frame.shore_land.rgb * frame.shore_land.rgb;
            albedo = mix(albedo, sand * grain, painted_sea_likeness(soft) * bank);
            let by_water = 1.0 - smoothstep(8.0, 24.0, -shore_distance(in.world.xz));
            shore_wet = (1.0 - smoothstep(0.0, 2.5, above)) * by_water * bank;
            albedo *= mix(1.0, 0.6, shore_wet);
        }
    }
    // The swash wets a beach a few units up from the water (perspective camera; TA's own camera
    // keeps the art as painted): darker sand, glossier, its upper edge wandering along the coast.
    // Only on gentle beaches low over the sea, and not where the art paints sea (the water's).
    if (beyond_on() && water_on && frame.water_extra.y < 0.5 && in_beyond < 64.0 && above > -0.5 && above < 3.0) {
        let inland = -shore_distance(in.world.xz);
        // (Further up than the swash's foam runs, which covers its first few units.)
        let reach = 8.0 + 5.0 * noise(in.world.xz / 23.0 + vec2<f32>(4.3, 1.1));
        let beach = foam_beach(length(ground_gradient(in.world.xz)));
        let sea = select(0.0, painted_sea_likeness(painted_albedo), frame.shore_land.a > 0.0);
        let soaked = (1.0 - smoothstep(reach * 0.4, reach, inland)) * (1.0 - smoothstep(1.5, 3.0, above)) * beach * (1.0 - sea);
        albedo *= mix(1.0, 0.64, soaked);
        shore_wet = max(shore_wet, soaked);
    }
    if (mat_check.a > 0.0) {
        albedo = mix(vec3<f32>(luminance(albedo)), mat_check.rgb, mat_check.a);
    }

    // Walls: the world's wall material in the ground's colours around, the paint's own light
    // and smears gone (the art averaged over 16 px or so, much less saturated on most worlds,
    // at least three quarters as light as the ground of the region on most worlds: TA painted
    // the faces towards its camera dark, `world_wall_lift`); on a face TA did paint, its painted
    // detail without its shade.
    // Enhanced Classic keeps more of its paint where it painted the face.
    var wall_k = 0.0;
    if (wall > 0.0) {
        let wide = shown_color(art_sample(shown.px, 4.5 + shown.blur, 0.0), shown);
        let region = shown_color(art_sample(shown.px, 8.0, 0.0), shown);
        let m = wall_material();
        let keep = detail_world.flags.z;
        var base = mix(vec3<f32>(luminance(wide)), wide, keep) * MATERIAL_TINT[m];
        // (Where TA painted a whole slope near black, its shade, the region is no guide: as
        // light, then, as the map's ground is on average (perspective camera), in the wall
        // material's own tint where scaling the paint up cannot get there, and as much of the
        // ground's average hue as the walls keep of its colour beyond most worlds' (a red
        // world's crater walls red-brown, not grey).)
        let average = frame.ground_bounce.rgb / max(frame.sun_color.rgb * frame.sun_dir.w * max(frame.sun_dir.y, 0.0) + frame.ambient.rgb, vec3<f32>(1e-3));
        let want = detail_world.flags.w * max(luminance(region), luminance(average) * frame.ground_bounce.a);
        base *= clamp(want / max(luminance(base), 1e-3), 1.0, 2.2);
        let hue = mix(vec3<f32>(1.0), average / max(luminance(average), 1e-4), saturate((keep - 0.4) / 0.6) * frame.ground_bounce.a);
        base = mix(MATERIAL_TINT[m] * hue * want, base, smoothstep(0.35, 0.8, luminance(base) / max(want, 1e-3)));
        let own = smoothstep(0.55, 0.8, painted);
        if (own > 0.0) {
            let detail = clamp(luminance(art_sample(shown.px, lod, 0.0)) / max(luminance(art_soft(shown.px, 3u)), 1e-3), 0.6, 1.5);
            base *= mix(1.0, detail, 0.7 * own);
        }
        // (Beds only on faces near upright that rise a good way: small steps and slopes are
        // plain rock.)
        let strata = (1.0 - smoothstep(0.5, 0.72, n.y)) * smoothstep(20.0, 60.0, rise) * (1.0 - smoothstep(0.25, 1.0, shown.blur));
        let rock = wall_surface(in.world, n, base, m, art, finish * detail_k, reborn, strata, detail_world.flags.x > 0.5);
        wall_k = max(stretched * mix(0.8, 1.0, reborn), steep * mix(0.35, 1.0, reborn)) * (1.0 - murk);
        albedo = mix(albedo, rock.color, wall_k);
        relief_n = normalize(mix(relief_n, rock.normal, wall_k));
    }

    // TA's art shows each place as its camera saw it from the south: dry ground that camera never
    // saw (the far side of a hill, turned away from it) only gets what lay in front of it in the
    // picture, on a pool's far shore the pool itself. Painted sea there (as first read, before
    // the detail and the rock above re-read the art; near the sea, paint darker than the sea's
    // too: the pool's darkest water, the shadowed rim TA painted along its banks) gives way to
    // what is painted where the ground stands (its art at sea level's offset: the bank or the
    // hillside), or failing that the painted beach's colour, keeping the light and shade the
    // detail gave it.
    if (water_on && frame.shore_land.a > 0.0 && depth < -1.0 && in_beyond <= 0.0) {
        let n0 = normalize(in.normal);
        let unseen = 1.0 - smoothstep(0.7, 0.95, n0.y + 0.5 * n0.z);
        let sea_light = max(luminance(frame.shore_water.rgb * frame.shore_water.rgb), 1e-4);
        // (Not by the melt: a lava world's banks are dark basalt. How near the sea, only where
        // it is asked.)
        let rimmed = step(frame.water_extra.y, 0.5) * unseen;
        var near = 0.0;
        var rim = (1.0 - smoothstep(0.35, 0.7, luminance(painted_albedo) / sea_light)) * rimmed;
        if (rim > 0.0) {
            near = smoothstep(0.1, 0.4, sea_fraction(in.world.xz, 3.0));
            rim *= near;
        }
        let wrong = unseen * max(painted_sea_likeness(painted_albedo), rim) * smoothstep(-1.0, -4.0, depth);
        if (wrong > 0.0) {
            let stand = art_sample(art_px + vec2<f32>(0.0, (in.world.y - frame.world.y) * 0.5), max(lod, 2.0), 0.0);
            let beach = frame.shore_land.rgb * frame.shore_land.rgb;
            let tone = clamp(luminance(albedo) / max(luminance(painted_albedo), 1e-3), 0.5, 1.6);
            var stand_rim = (1.0 - smoothstep(0.35, 0.7, luminance(stand) / sea_light)) * rimmed;
            if (stand_rim > 0.0 && rim <= 0.0) {
                near = smoothstep(0.1, 0.4, sea_fraction(in.world.xz, 3.0));
            }
            stand_rim *= near;
            albedo = mix(albedo, mix(stand, beach, max(painted_sea_likeness(stand), stand_rim)) * tone, wrong);
        }
    }

    // Ground deep under the sea painted as land (perspective camera): the same, under the water.
    // A beach's steep bank into the deep is painted with what lies above it in the picture (the
    // beach, the bank's face) dozens of units down, and through the water it was a pale sheet
    // over the sea, ending where the painting's sea began. Below the depth of TA's painted
    // shallows it takes the painted sea's colour, as the ground beyond the map does.
    if (water_on && beyond_on() && frame.water_extra.y < 0.5 && frame.beyond_sea.a > 0.0 && in_beyond <= 0.0 && depth > 12.0) {
        albedo = mix(albedo, frame.beyond_sea.rgb, (1.0 - painted_water(painted_albedo)) * smoothstep(12.0, 30.0, depth));
    }

    // Ore (perspective camera): a metal deposit's bed in the ground (TA's painted patch is not
    // drawn there, see the game's features.rs; the classic camera keeps it), as TA paints one: a
    // dark stain of rubble (the ground's own colour, greyed) with stones of steel-grey ore in it,
    // each a lump lit on its sun side, crevices between them, some lighter than others (smaller
    // than a pixel, their mean), brighter veins here and there in its heart; darker than bright
    // ground and lighter than ash. Shaped from the beds' cover of the cells (see the game's
    // ore.rs; four taps round, for a round shape) into a ragged bed whose edge goes round its
    // stones, a faint stain round it, a little metal in its stones' finish; lit, shadowed and
    // hazed as the ground. (`ore_k`: how much of it is bed; its glints below keep to it.)
    var ore_k = 0.0;
    if (frame.camera_pos.w > 0.5 && in_beyond <= 0.0) {
        let cover = textureSampleLevel(ground_map, los_sampler, in.world.xz * frame.terrain_shadow.yz, 0.0).b;
        if (cover > 0.001) {
            var dome = 0.0;
            for (var k = 0; k < 4; k++) {
                let o = vec2<f32>(select(-6.0, 6.0, (k & 1) == 1), select(-6.0, 6.0, k >= 2));
                dome += textureSampleLevel(ground_map, los_sampler, (in.world.xz + o) * frame.terrain_shadow.yz, 0.0).b * 0.25;
            }
            let shape = mix(dome, cover, 0.4);
            let ragged = (noise(in.world.xz / 13.0) * 0.65 + noise(in.world.xz / 5.0 + vec2<f32>(17.3)) * 0.35) * 0.9;
            let st = ore_stones(in.world.xz / ORE_STONE);
            let seen = 1.0 - smoothstep(0.3, 0.7, footprint / ORE_STONE);
            let lump = mix(0.4, st.x, seen);
            ore_k = smoothstep(0.36, 0.52, shape + ragged + (lump - 0.4) * 0.3) * (1.0 - 0.5 * smoothstep(0.0, 24.0, depth));
            let ground_l = luminance(albedo);
            let rubble = mix(albedo, vec3<f32>(ground_l), 0.5) * 0.5;
            let bed_l = max(mix(0.11, ground_l, 0.35), luminance(rubble) * 1.4);
            var bed = mix(rubble, vec3<f32>(0.9, 0.95, 1.03) * bed_l * mix(1.0, 0.75 + 0.5 * st.w, seen), lump);
            if (finish > 0.0) {
                let grain = material_detail(4u, in.world.xz, art.dwx.xz, art.dwy.xz);
                bed *= 1.0 + 0.45 * grain.x * finish;
                relief_n = normalize(relief_n + vec3<f32>(-grain.y, 0.0, -grain.z) * ore_k * finish);
            }
            // (Its stones' lumps catch the sun.)
            relief_n = normalize(relief_n + vec3<f32>(st.y, 0.0, st.z) * 0.6 * ore_k * seen);
            let q = mat2x2<f32>(0.8, 0.6, -0.6, 0.8) * in.world.xz;
            let vein = smoothstep(0.9, 0.97, 1.0 - abs(noise(q / vec2<f32>(4.0, 12.0)))) * smoothstep(0.05, 0.3, noise(q / 19.0 + vec2<f32>(5.1))) * smoothstep(0.7, 0.95, shape);
            bed = mix(bed, vec3<f32>(0.85, 0.92, 1.0) * bed_l * 1.6, vein * lump);
            albedo *= 1.0 - 0.2 * smoothstep(0.2, 0.36, shape + ragged) * (1.0 - ore_k);
            albedo = mix(albedo, bed, ore_k);
            mat_metal = max(mat_metal, 0.6 * ore_k * lump);
            mat_rough = mix(mat_rough, 0.45, ore_k * lump);
        }
    }
    // The world's colour grade (Reborn, see `world_graded` in common.wgsl), over the ground's
    // whole palette: open ground, the features and markings painted on it, walls and banks alike.
    albedo = world_graded(albedo);

    // The art already contains painted lighting for a flat surface lit from above. Re-light
    // relative to that: flat ground is unchanged, slopes facing the sun brighten, slopes
    // facing away darken, and dynamic shadows are added.
    let l = frame.sun_dir.xyz;
    let ndl = max(dot(relief_n, l), 0.0);
    let flat_ndl = max(l.y, 0.2);
    // (Capped: slopes facing a sun low in the sky would otherwise glare.)
    // (Under water the light reaching the bed is diffused by the water and the waves above it:
    // its slopes stand out less, the deeper it lies.)
    let relight = min(1.0 + frame.sun_color.a * (ndl - flat_ndl) / flat_ndl * (1.0 - 0.8 * murk), 1.7);
    // (No models stand far beyond the map, but the relief's long shadows run on over the ground
    // there; the clouds' shadows reach the horizon.) Ray traced where the depth prepass saw this
    // very ground (raytrace.rs).
    let rt = rt_visibility(in.clip);
    var shadow = 1.0;
    if (rt.traced) {
        shadow = min(mix(1.0, rt.sun, frame.screen.w), terrain_sunlight(in.world));
        // (The 3D trees and rocks are not traced: their shadows have a map of their own.)
        if (in_beyond < 96.0) {
            shadow = min(shadow, rt_feature_shadow(in.world, n));
        }
    } else if (in_beyond < 96.0) {
        shadow = shadow_factor(in.world, n);
    } else {
        shadow = terrain_sunlight(in.world);
    }
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(in.world);
    }
    // (The deeper the bed, the more the water scatters the light into its shadows.)
    shadow = mix(shadow, 1.0, 0.6 * murk);
    // A face turned away from the light gets none of it directly (what light it keeps is the
    // relight's low end: the sky's, and TA painted it dark): a ridge's or a cloud's shadow, or
    // its own in the shadow map, cannot darken it a second time, which crushed such faces to
    // black. (Highlights keep `shadow`.)
    let shaded = mix(1.0, shadow, smoothstep(0.0, 0.05, dot(n, l)));
    // Nor is shade TA painted near black darkened much more by the relighting (as the
    // occlusion below spares painted shadows).
    var relit = max(relight, 0.35);
    if (relit < 1.0) {
        relit = mix(1.0, relit, mix(0.4, 1.0, smoothstep(0.004, 0.035, luminance(albedo))));
    }
    var sun = relit * mix(shade_fill(), vec3<f32>(1.0), shaded);
    if (wall_k > 0.0) {
        // (The painted ground's light is relative to TA's flat painted lighting; a wall sees
        // the sun as it falls on it, the half of the sky it faces and the ground below it,
        // as light as the map's ground is on average.)
        let fill = shade_fill();
        let sky_view = 0.5 + 0.5 * relief_n.y;
        let ground = frame.ground_bounce.rgb / max(frame.sun_color.rgb * frame.sun_dir.w * max(l.y, 0.0) + frame.ambient.rgb, vec3<f32>(1e-3));
        let wall_light = (vec3<f32>(1.0) - fill) * min(ndl / flat_ndl, 1.7) * shadow + fill * sky_view + ground * (0.5 - 0.5 * relief_n.y) * frame.ground_bounce.w;
        sun = mix(sun, wall_light, wall_k);
    }
    sun *= frame.sun_dir.w;
    // Caustics redistribute sunlight (brighter filaments, dimmer cells; same average): light,
    // not paint, so on a saturated bed (Gods of War's lilac) they are paler than it.
    let caustic_k = caustic * 0.9 * shadow * saturate(l.y * 1.5);
    let albedo_sun = max(albedo + mix(albedo, vec3<f32>(luminance(albedo)), 0.4) * caustic_k, vec3<f32>(0.0));
    // Dynamic point lights add to the painted lighting. The time of day colours the light on
    // the painted ground, and at night the sky adds its own (both nothing by TA's light).
    // (Up close they catch the relief of the ground; with ray tracing what stands in the way
    // of the brightest shadows it.)
    let point = point_lighting(in.world, relief_n) * rt.point;
    let sunlit = sun * frame.daylight.rgb;
    var lit = albedo_sun * sunlit + albedo * (frame.night.rgb + point);
    // Ambient occlusion (perspective camera): the relief's creases, and whatever stands on the
    // ground around. The art has its lighting painted in, so the relief's occlusion acts less
    // on art that is already dark (painted shadows are not darkened twice); what stands on the
    // ground is not in the painting, so its occlusion acts in full.
    var ao = contact_occlusion(in.world.xz) * rt.ao;
    if (frame.ao.x > 0.0) {
        let painted_light = smoothstep(0.012, 0.12, luminance(albedo));
        var relief = terrain_occlusion(in.world.xz, 0.0);
        if (beyond > 0.0) {
            relief = mix(1.0, terrain_occlusion(shown.at, 0.0), detail_k);
        }
        ao *= mix(1.0, relief, frame.ao.x * mix(0.3, 1.0, painted_light) * (1.0 - 0.5 * wall_k));
    }
    if (ao < 1.0) {
        lit = (albedo_sun * sunlit + albedo * frame.night.rgb) * ao + albedo * point * (0.5 + 0.5 * ao);
    }
    // Dry surfaces have material response too: broad metal reflections, a restrained ice
    // sheen, and faint dielectric highlights. Finite-width lobes and filtered relief keep
    // this stable during camera movement; no random bright glitter is added to the terrain.
    // (Reflections are taken about a normal bent just enough to face the eye: a facet of the
    // relief turned away from it shows nothing of the sky, it is hidden behind the bumps in front.
    // Without that, every such facet mirrored the sky at full Fresnel, a field of bright specks.)
    let eye = view_dir(in.world);
    let spec_n = normalize(relief_n + eye * max(0.08 - dot(relief_n, eye), 0.0));
    if (reborn * mat_k > 0.0 && depth <= 0.5) {
        let v = eye;
        let h = normalize(v + frame.sun_dir.xyz);
        // (Relief the pixel no longer resolves roughens the surface instead.)
        let rough = clamp(mat_rough + 0.25 * (1.0 - mat_detail_kept) + 0.1 * abs(mat_d.x), 0.26, 1.0);
        let exponent = mix(72.0, 5.0, rough * rough);
        let f0 = mix(vec3<f32>(0.025), clamp(albedo, vec3<f32>(0.05), vec3<f32>(0.55)), mat_metal);
        // Bounded: a rough surface's Fresnel rises to no more than its smoothness at grazing
        // (Schlick, roughness-aware), masked by its own bumps (Smith-Schlick); the plain's own
        // grazing sheen from its broad normal.
        let nv = max(dot(spec_n, v), 1e-3);
        let kg = rough * rough * 0.5;
        let g1 = nv / (nv * (1.0 - kg) + kg);
        let f_rough = f0 + (max(vec3<f32>(1.0 - rough), f0) - f0) * pow(1.0 - nv, 5.0);
        // (Masked too: rough ground does not glow along its crests, where it turns edge-on.)
        let nvp = max(dot(n, v), 1e-3);
        let f_plain = (f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - nvp, 5.0)) * (nvp / (nvp * (1.0 - kg) + kg));
        let env_f = mix(f_plain, f_rough * g1, saturate(mat_metal / 0.8));
        let environment = wet_sky(reflect(-v, spec_n));
        let lobe = pow(saturate(dot(spec_n, h)), exponent) * (exponent + 2.0) / 30.0 * saturate(dot(spec_n, frame.sun_dir.xyz)) * g1;
        let sheen = env_f * environment * ao * (0.08 + mat_metal * 0.22 + mat_ice * 0.08)
            + f_rough * frame.sun_color.rgb * frame.sun_dir.w * shadow * lobe * (1.0 - rough * 0.5);
        lit = lit * (1.0 - mat_metal * 0.08 * mat_k) + sheen * reborn * mat_k;
    }
    // Ore (perspective camera): metal glints in a deposit's bed (see `ore_k` above), twinkling
    // as the sun and the eye move, and a metallic sheen of the sky over it; none off it.
    if (ore_k > 0.0) {
        let v = view_dir(in.world);
        let g = ore_glints(in.world.xz, v, frame.sun_dir.xyz, footprint, t, ore_k);
        let sun = frame.sun_color.rgb * frame.sun_dir.w * shadow;
        let r = reflect(-v, spec_n);
        let sheen = (0.04 + 0.5 * pow(1.0 - saturate(dot(spec_n, v)), 5.0)) * ore_k * 0.35;
        lit += sun * vec3<f32>(0.9, 0.95, 1.0) * g + wet_sky(r) * sheen * ao;
    }
    // Beyond the map, ground standing out of the water where the water pass draws sea (where the
    // mesh's triangles cut its per-pixel shoreline) is drawn as the water is there: painted as
    // the sea (see `ground_albedo`) and mirroring the sky, as flat as a puddle. (Painted sea alone,
    // seen from low over the sea, was dark wedges sticking out of the banks and dark hairlines
    // across the bright, sky-mirroring water near the horizon.)
    // So too the flats the water leaves dry there under its level (the coast's band, see
    // `skirt_height`), wet sand, as the view grazes them: from low over the sea, seen edge-on as
    // the sea round them mirrors the haze, each far island's flats were a dark line across it.
    // (Not on the far land's: its band lies under the water a vertex spacing or two inland, wide
    // flats that gleamed like the sea round the land standing out of them.)
    var awash = 0.0;
    if (water_on && frame.water_extra.y < 0.5 && beyond > 0.0) {
        let standing = smoothstep(0.5, 2.0, water_depth) * (1.0 - smoothstep(-0.25, 0.25, depth));
        let flats = smoothstep(-0.5, 0.0, -water_depth) * smoothstep(0.0, 0.25, depth) * (1.0 - smoothstep(0.02, 0.06, view_dir(in.world).y))
            * (1.0 - shown.far);
        awash = max(standing, flats);
        puddle = max(puddle, awash);
    }
    // Wet ground gleams with the sky; puddles mirror it, rain rings spreading over them, and
    // catch the sun.
    let gleam = max(max(wet, shore_wet * 0.3), awash);
    if (gleam > 0.0) {
        let v = view_dir(in.world);
        let rip = rain_ripples(in.world.xz, t, frame.rain.y, footprint);
        // (The rings' ripples too fine for the pixels blur the puddle's mirror and its glint
        // instead: a soft sheen, not single white pixels.)
        let alpha2 = 0.00125 + rain_roughness(frame.rain.y, footprint) * puddle;
        let sharpness = 0.5 / alpha2;
        // (A film of water evens out the relief's bumps; the puddles are flat but for the rings.)
        let film = normalize(mix(spec_n, n, 0.75 * smoothstep(0.0, 0.6, gleam)));
        let nn = normalize(mix(film, normalize(vec3<f32>(-rip.x, 1.0, -rip.y)), puddle));
        let r = reflect(-v, nn);
        let f0 = mix(0.02, 0.05, puddle);
        let wet_rough = mix(0.55, 0.0, puddle);
        // (Masked by its own bumps, Smith-Schlick: soaked earth and grass do not mirror the sky at
        // grazing, where a crest turns edge-on, or every crest would glow like a pane of frosted
        // glass; standing water does.)
        let nv = max(dot(nn, v), 1e-3);
        let kg = wet_rough * wet_rough * 0.5;
        let masking = nv / (nv * (1.0 - kg) + kg);
        let fres = (f0 + (max(1.0 - wet_rough, f0) - f0) * pow(1.0 - nv, 5.0)) * masking;
        let gloss = mix(0.45 * gleam, 1.0, puddle);
        // (Under the rain's cloud the sun has no disc for them to mirror: its glint fades while
        // it rains, or, spread by the rings over the whole puddle, it laid a flat pale sheet on
        // it; the puddles mirror the grey sky.)
        let glint = frame.sun_color.rgb * frame.sun_dir.w * pow(saturate(dot(r, frame.sun_dir.xyz)), sharpness) * 5.0 * (sharpness + 1.0) / 401.0 * shadow * puddle * (1.0 - 0.85 * frame.rain.y);
        // Fires, explosions and lamps shine in the wet ground: long, bright reflections on the
        // puddles, broad sheens on the soaked earth.
        let mirrored = point_lights_eval(in.world, nn, v, mix(30.0, min(400.0, 2.0 / alpha2), puddle)).specular * mix(0.6, 3.0, puddle) * gleam;
        // Ray traced: the units standing around mirrored in the puddles (a little wobbled by
        // the rain rings), dimly in the soaked earth.
        var sky = wet_sky(r);
        let traced = rt_reflection(in.clip + vec4<f32>(rip * 6.0 * puddle, 0.0, 0.0));
        sky = mix(sky, traced.rgb, traced.a * mix(0.2, 1.0, puddle));
        lit = mix(lit, sky, saturate(fres * gloss)) + glint + mirrored;
    }
    if (foam.x > 0.0) {
        lit = mix(lit, foam_color(shadow, point, foam.y) * mix(1.0, ao, 0.8), foam.x);
    }
    // Lava worlds: the melt lights up the rock around it (its own surface glows in the water
    // pass, so the painted melt below the surface is left alone). Beyond the map the melt
    // nearby is the mirrored ground's.
    if (water_on && frame.water_extra.y > 0.5 && frame.shore_land.a > 0.0 && depth > -80.0 && depth < 1.0) {
        var near = painted_water(art_sample(shown.px, 5.5, 0.0));
        var around = painted_water(art_sample(shown.px, 7.0, 0.0));
        if (beyond > 0.0) {
            let w = beyond_wet_from(beyond_ground_from(shown.at, in.world.xz, beyond, shown.far, false).y, in.world.xz, beyond);
            near = mix(near, smoothstep(0.2, 0.6, w), beyond_coast_mix(beyond));
            around = mix(around, w, beyond_coast_mix(beyond));
        }
        lit += albedo * lava_glow(near, around * smoothstep(-80.0, -24.0, depth), t) * (1.0 - dry_melt);
    }
    // (Not on walls: melt does not stand on a cliff, whatever the paint stretched up it says.)
    let melt_here = dry_melt * (1.0 - wall);
    if (melt_here > 0.0) {
        // (Its surface at the lava's level on the same line of sight, as the water pass would
        // draw it there: the two meet at the waterline without a seam.)
        let v = view_dir(in.world);
        let at = vec3<f32>(in.world.xz + v.xz * (depth / max(v.y, 0.05)), frame.world.y).xzy;
        // (The art coordinate's change across a pixel: the ground's, (x, z - y/2).)
        let rim = lava_rim(in.art_px, art.dwx.xz - vec2<f32>(0.0, 0.5 * art.dwx.y), art.dwy.xz - vec2<f32>(0.0, 0.5 * art.dwy.y));
        let layer = lava_layer(at, shore_distance(at.xz), melt_here, rim * (1.0 - wall), shadow, footprint, t);
        lit = lit * (1.0 - layer.a) + layer.rgb;
    }

    // Map overlays (a player's map view, the editor's height map): over the lit ground on the
    // map itself; beyond it the ground greys out, so the overlay ends where the map does.
    if (frame.detail.z > 0.5) {
        if (in_beyond <= 0.0) {
            lit = map_overlay_color(lit, in.world, art.footprint);
        } else {
            lit = mix(lit, vec3<f32>(luminance(lit) * 0.4), 0.75);
        }
    }

    // Terrain whose art coordinate falls outside the painted image (the strip TA could never
    // scroll to, and beyond the map edges) fades into darkness, unless the world continues
    // beyond the map (the albedo then continues too).
    var faded = lit;
    if (!beyond_on()) {
        faded = mix(frame.fog.rgb * 0.08, lit, art_edge_fade(in.art_px));
    }
    if (in_beyond > 0.0) {
        let f = beyond_fog(in.world, in_beyond);
        return vec4<f32>(mix(fog_of_war(faded, in.world), fog_color_at(in.world), f), 1.0);
    }
    return vec4<f32>(apply_fog(faded, in.world), 1.0);
}
