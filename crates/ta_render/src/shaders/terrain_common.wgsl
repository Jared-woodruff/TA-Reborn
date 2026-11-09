// Terrain resources shared by the terrain and water shaders (bind group 1): heightmap,
// tile map and tile atlas, plus the functions that sample them.
//
// The map art was painted in TA's oblique projection, so a ground point (x, h, z) shows the
// art pixel at (x, z - h/2). Tiles are stored once each in an atlas; `art_sample` performs
// exact trilinear filtering of the *virtual* full-map image by resolving each texel's tile
// through the tile map. Because tiles are 32 px, box-filtered per-tile mips equal the mips
// of the whole image, and the 1x1 level equals a per-tile average texture.
//
// Beyond the map (perspective camera with the sky on, see `beyond_on`), the extended ground
// ("skirt", see sky.rs) is the map's own mirror image next to it, so that whatever crosses an
// edge carries on without a seam, and further out land and sea of its own, made after the map's
// own ground round each stretch of its edges (see "Beyond the map" below): never copies of it.

struct TerrainParams {
    cells: vec2<u32>,       // heightmap size (16 px cells)
    tiles: vec2<u32>,       // tile map size (32 px tiles)
    atlas_cols: u32,        // tiles per atlas row
    chunk_cells: u32,       // cells per chunk edge
    detail: f32,            // strength of zoomed-in detail enhancement
    _pad: u32,
};

@group(1) @binding(0) var<uniform> params: TerrainParams;
@group(1) @binding(1) var heightmap: texture_2d<f32>;
@group(1) @binding(2) var tile_map: texture_2d<u32>;
@group(1) @binding(3) var tile_atlas: texture_2d<f32>;
@group(1) @binding(4) var tile_avg: texture_2d<f32>;
@group(1) @binding(5) var linear_clamp: sampler;
// The map's plain ground tiles, most used first (PlainTiles in terrain.rs).
struct PlainTiles {
    // xyz = square root of the mean colour (linear)
    tiles: array<vec4<f32>, 16>,
    // x = number of tiles in use
    count: vec4<u32>,
};
@group(1) @binding(9) var<uniform> plain_tiles: PlainTiles;
// What lies round the map (Surroundings in surroundings.rs): the map's own ground near each of
// AROUND points spaced evenly round its edges, clockwise from its north-west corner, for the far
// land beyond it (see `far_ground`).
const AROUND: i32 = 128;
struct Surroundings {
    // x = the fraction of the ground under the sea, y = the land's mean height, z = the sea
    // bed's, w = how much the land's heights vary (their standard deviation).
    shape: array<vec4<f32>, 128>,
    // The dry ground's colours (linear): its lighter half's, its darker half's.
    light: array<vec4<f32>, 128>,
    dark: array<vec4<f32>, 128>,
    // The sea's colour as painted (linear).
    sea: array<vec4<f32>, 128>,
};
@group(1) @binding(6) var<uniform> surroundings: Surroundings;
// Sky visibility of every heightmap vertex (ambient occlusion of the relief, baked at load;
// see `terrain_visibility` in occlusion.rs), and its distance from the waterline (g, see
// `shore_distance`).
@group(1) @binding(11) var terrain_ao: texture_2d<f32>;
// Map overlay colours per heightmap cell (straight alpha; see TerrainGpu::set_overlay).
@group(1) @binding(12) var map_overlay: texture_2d<f32>;

// Sky visibility of the relief at world xz (1 = the whole sky), bilinear between the heightmap
// vertices; beyond the map (`beyond` units out) it eases to 1 over a few cells.
fn terrain_occlusion(xz: vec2<f32>, beyond: f32) -> f32 {
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(params.cells);
    let v = textureSampleLevel(terrain_ao, linear_clamp, uv, 0.0).r;
    return mix(v, 1.0, smoothstep(0.0, 64.0, beyond));
}

// (`FAR_LAND`, defined before this file: whether the shader draws the ground beyond the map, the
// skirt's and the water's, or only the map's; see `terrain::shader_source_for`. Without it the
// lookups beyond the map below are left out: no point the map's own shader looks at lies there.)

// Distance (world units) from world `xz` to the waterline, the sea-level contour of the terrain
// mesh: > 0 under water, < 0 on land, saturating at ±32 (`water::shore_distances`, measured at
// the heightmap's vertices; beyond the map, next to it at the mirror image the ground there is
// drawn from, see `mirror_point`; further out from how much of the far land lies under the sea
// there, whose coasts are drawn per pixel, see `beyond_wet`).
fn shore_distance(world_xz: vec2<f32>) -> f32 {
    if (FAR_LAND && beyond_on()) {
        let d = beyond_distance(world_xz);
        if (d > 0.0) {
            let k = far_mix(world_xz, d);
            if (k <= 0.0) {
                return shore_distance_at(mirror_point(world_xz, d).at);
            }
            let far = clamp((beyond_wet_from(far_ground(world_xz, d, false).y, world_xz, d) - 0.5) * 128.0, -32.0, 32.0);
            if (k >= 1.0) {
                return far;
            }
            return mix(shore_distance_at(mirror_point(world_xz, d).at), far, k);
        }
    }
    return shore_distance_at(world_xz);
}

// `shore_distance` at map position `xz`. Between the vertices a cubic B-spline (four bilinear
// taps, Sigg and Hadwiger): the contours of a bilinear field kink at every cell, of this one
// they curve smoothly (rounding the mesh's sharpest corners by a unit or two).
fn shore_distance_at(xz: vec2<f32>) -> f32 {
    let p = xz / 16.0;
    let i = floor(p);
    let f = p - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let size = vec2<f32>(params.cells);
    let h0 = (i - 0.5 + w1 / g0) / size;
    let h1 = (i + 1.5 + w3 / g1) / size;
    let a = textureSampleLevel(terrain_ao, linear_clamp, h0, 0.0).g;
    let b = textureSampleLevel(terrain_ao, linear_clamp, vec2<f32>(h1.x, h0.y), 0.0).g;
    let c = textureSampleLevel(terrain_ao, linear_clamp, vec2<f32>(h0.x, h1.y), 0.0).g;
    let d = textureSampleLevel(terrain_ao, linear_clamp, h1, 0.0).g;
    let v = (a * g0.x + b * g1.x) * g0.y + (c * g0.x + d * g1.x) * g1.y;
    return (v * 255.0 - 128.0) * 0.25;
}

// Height of heightmap vertex `c` as the ground is drawn (cells are 16 px; clamped at the map
// edges): TA's heights smoothed within a tolerance (terrain_surface.rs), not whole units.
fn height_at(c: vec2<i32>) -> f32 {
    let cc = clamp(c, vec2<i32>(0), vec2<i32>(params.cells) - 1);
    return textureLoad(heightmap, cc, 0).r * 255.0;
}

// Whether a cell with corner heights h00, h10, h01, h11 is split along its (1,0)-(0,1)
// diagonal rather than (0,0)-(1,1): a steep cell (corners 16 units or more apart) is split
// along the diagonal whose ends differ less, so a slope running across the other diagonal
// is drawn as the straight slope it is, not as a step a cell (a cliff's lip running from
// south-west to north-east was a staircase). Gentle ground keeps the one split.
// (`split_flipped` in terrain.rs and mapgen's `Field::sample_tri` are the same rule.)
fn split_flipped(h00: f32, h10: f32, h01: f32, h11: f32) -> bool {
    let range = max(max(h00, h10), max(h01, h11)) - min(min(h00, h10), min(h01, h11));
    return range >= 16.0 && abs(h10 - h01) < abs(h00 - h11);
}

// Ground height exactly as the terrain mesh interpolates it, at fraction `t` of a cell with
// those corner heights (HeightField::sample on the CPU matches this).
fn cell_height(h00: f32, h10: f32, h01: f32, h11: f32, t: vec2<f32>) -> f32 {
    if (split_flipped(h00, h10, h01, h11)) {
        if (t.x + t.y <= 1.0) {
            return h00 + (h10 - h00) * t.x + (h01 - h00) * t.y;
        }
        return h11 + (h01 - h11) * (1.0 - t.x) + (h10 - h11) * (1.0 - t.y);
    }
    if (t.x >= t.y) {
        return h00 + (h10 - h00) * t.x + (h11 - h10) * t.y;
    }
    return h00 + (h11 - h01) * t.x + (h01 - h00) * t.y;
}

// Ground height exactly as the terrain mesh interpolates it.
fn mesh_height(xz: vec2<f32>) -> f32 {
    let f = xz / 16.0;
    let c = floor(f);
    let i = vec2<i32>(c);
    return cell_height(height_at(i), height_at(i + vec2<i32>(1, 0)), height_at(i + vec2<i32>(0, 1)), height_at(i + vec2<i32>(1, 1)), f - c);
}

// Bilinearly filtered ground height: smoother than the mesh (no creases along triangle
// edges), for quantities whose derivatives are visible, such as slopes.
fn smooth_height(xz: vec2<f32>) -> f32 {
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(params.cells);
    return textureSampleLevel(heightmap, linear_clamp, uv, 0.0).r * 255.0;
}

// Horizontal gradient of the smoothed ground height (height units per world unit).
fn smooth_gradient(xz: vec2<f32>) -> vec2<f32> {
    let e = 8.0;
    let gx = smooth_height(xz + vec2<f32>(e, 0.0)) - smooth_height(xz - vec2<f32>(e, 0.0));
    let gz = smooth_height(xz + vec2<f32>(0.0, e)) - smooth_height(xz - vec2<f32>(0.0, e));
    return vec2<f32>(gx, gz) / (2.0 * e);
}

// One texel of the virtual map image at mip `lod` (0..4).
fn art_texel(p: vec2<i32>, lod: u32) -> vec3<f32> {
    let tile_size = 32u >> lod;
    let size = vec2<i32>(params.tiles * tile_size);
    let t = vec2<u32>(clamp(p, vec2<i32>(0), size - 1));
    let tile = textureLoad(tile_map, t / tile_size, 0).r;
    let slot = vec2<u32>(tile % params.atlas_cols, tile / params.atlas_cols);
    return textureLoad(tile_atlas, slot * tile_size + t % tile_size, i32(lod)).rgb;
}

// Atlas tile at `slot` (its tile coordinates in the atlas) repeated with a 32 px period,
// sampled bilinearly at pixel `local` of mip `lod` (0..4).
fn tile_bilinear(slot: vec2<i32>, local: vec2<f32>, lod: u32) -> vec3<f32> {
    let size = i32(32u >> lod);
    let mask = vec2<i32>(size - 1);
    let origin = slot * size;
    let p = local / f32(1u << lod) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let l = i32(lod);
    // Wrap within the tile (sizes are powers of two; & works for negative texels too).
    let a = textureLoad(tile_atlas, origin + (i & mask), l).rgb;
    let b = textureLoad(tile_atlas, origin + ((i + vec2<i32>(1, 0)) & mask), l).rgb;
    let c = textureLoad(tile_atlas, origin + ((i + vec2<i32>(0, 1)) & mask), l).rgb;
    let d = textureLoad(tile_atlas, origin + ((i + vec2<i32>(1, 1)) & mask), l).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// Art pixel `px` reflected into the painted image at its edges (repeating), for sampling the
// art mirrored beyond it.
fn mirror_art(px: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(params.tiles * 32u);
    let f = fract(px / (2.0 * size)) * 2.0;
    return (1.0 - abs(f - 1.0)) * size;
}

// `sharp` in 0..1 steepens the interpolation between texels ("sharp bilinear"), keeping
// magnified pixel art crisp without the aliasing of nearest-neighbour sampling.
fn art_bilinear(px: vec2<f32>, lod: u32, sharp: f32) -> vec3<f32> {
    let p = px / f32(1u << lod) - 0.5;
    let i = vec2<i32>(floor(p));
    let fl = p - floor(p);
    let f = mix(fl, smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), fl), sharp);
    let a = art_texel(i, lod);
    let b = art_texel(i + vec2<i32>(1, 0), lod);
    let c = art_texel(i + vec2<i32>(0, 1), lod);
    let d = art_texel(i + vec2<i32>(1, 1), lod);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn art_sample(px: vec2<f32>, lod: f32, sharp: f32) -> vec3<f32> {
    if (lod >= 5.0) {
        let uv = px / vec2<f32>(params.tiles * 32u);
        return textureSampleLevel(tile_avg, linear_clamp, uv, lod - 5.0).rgb;
    }
    let l0 = u32(floor(lod));
    let t = fract(lod);
    let c0 = art_bilinear(px, l0, select(0.0, sharp, l0 == 0u));
    if (t < 0.01) {
        return c0;
    }
    var c1: vec3<f32>;
    if (l0 + 1u >= 5u) {
        let uv = px / vec2<f32>(params.tiles * 32u);
        c1 = textureSampleLevel(tile_avg, linear_clamp, uv, 0.0).rgb;
    } else {
        c1 = art_bilinear(px, l0 + 1u, 0.0);
    }
    return mix(c0, c1, t);
}

// 0 outside the painted image, fading to 1 within 40 art pixels of its edges.
fn art_edge_fade(art_px: vec2<f32>) -> f32 {
    let size = vec2<f32>(params.tiles * 32u);
    let edge_dist = min(min(art_px.x, art_px.y), min(size.x - art_px.x, size.y - art_px.y));
    return smoothstep(-2.0, 40.0, edge_dist);
}

// ---- Beyond the map -----------------------------------------------------------------------
//
// Next to the map the ground past its edges is its own mirror image: a point there shows the
// ground at the point reflected into the map across the edges nearest to it (`mirror_point`).
// Whatever crosses an edge carries on (a hill cut by the edge closes as its own mirror image, a
// coast bends back, open sea stays sea, a road that meets the edge goes on) and the seam is
// exact: heights, art, land and sea all meet the map's own. A slow domain warp, nothing at the
// edge and growing with the distance, bends the axes of symmetry.
//
// Further out (`far_mix`: from a few hundred units, all of it a thousand or two out) the land is
// the far land's own (`far_ground`, `far_color`), made after the map's ground round the nearest
// stretch of its edges (`surroundings`): as much of it sea as there, the land as high and as
// rugged, painted in its colours, but its coasts and hills shaped by noise. The mirror image
// further out showed copies of the map all round it (a continent's own coasts round it on every
// side), which no world would have. The far land dissolves into the horizon haze.

fn map_size_px() -> vec2<f32> {
    return vec2<f32>(params.cells) * 16.0;
}

// Horizontal distance from world position `xz` to the map rectangle (0 inside it).
fn beyond_distance(xz: vec2<f32>) -> f32 {
    return distance(xz, clamp(xz, vec2<f32>(0.0), map_size_px()));
}

// Distance fog on the extended ground at `world`, `beyond` units from the map: the usual fog,
// but free of its cap (which keeps the battlefield readable) a few thousand units out, and
// thickening with distance (the haze along a long, low view is as dense as the air near the
// ground), so the land nearest the horizon has nearly dissolved into it and meets the sky
// without a line; and dissolving completely into the horizon haze over the outer half of the
// ground's reach, so its far edge never shows.
fn beyond_fog(world: vec3<f32>, beyond: f32) -> f32 {
    let reach = frame.ground_beyond.x;
    if (!FAR_LAND || !beyond_on() || beyond <= 0.0) {
        return fog_factor(world);
    }
    var f = fog_factor(world);
    if (frame.fog.a > 0.0) {
        let d = distance(world, frame.camera_pos.xyz);
        let thick = 1.0 + smoothstep(4000.0, 20000.0, d);
        let open = 1.0 - exp(-max(d - frame.los.w, 0.0) * frame.fog.a * thick);
        f = max(f, mix(f, open, smoothstep(0.0, 3000.0, beyond)));
    }
    if (reach > 0.0) {
        f = max(f, smoothstep(reach * 0.5, reach * 0.98, beyond));
    }
    return f;
}

// The mirror's domain warp at `xz`, `d` beyond the map (world units): nothing at the edge, so
// the reflection meets the map exactly, then growing with the distance (gently at first, so
// roads and coasts that cross an edge run on nearly straight: a wiggle of up to 192 units,
// and a slow sway that reaches a sixth of the map's size). Its slope stays well below 1, so
// the warped reflection never folds over itself.
fn mirror_warp(xz: vec2<f32>, d: f32) -> vec2<f32> {
    let size = map_size_px();
    let q = xz / 1100.0;
    let wiggle = vec2<f32>(noise(q + vec2<f32>(3.1, 7.7)), noise(q + vec2<f32>(-5.3, 1.9))) * min(d * d / (d + 600.0) * 0.2, 192.0);
    let s = xz / 4800.0;
    let sway = vec2<f32>(noise(s + vec2<f32>(-1.7, 4.3)), noise(s + vec2<f32>(6.2, -2.8))) * min(d * d / (d + 1200.0) * 0.12, min(size.x, size.y) / 6.0);
    return wiggle + sway;
}

// The ground point inside the map that the point `xz`, `d` beyond it shows: `xz` (warped)
// reflected into the map across its edges, as often as it takes (a triangle wave of period
// twice the map's size along each axis).
struct Mirror {
    at: vec2<f32>,
    // Per axis, -1 where the reflection runs the map backwards (slopes and gradients read at
    // `at` turn round along that axis), else 1.
    flip: vec2<f32>,
};

// How far the art shown at `xz` beyond the map is moved along the edges it lies beyond (art
// pixels): nothing at the edge, so the seam stays exact, then a few pixels. The reflection's
// painted speckle no longer meets the map's own as its mirror image, which closed up into a
// column of kaleidoscope chevrons along the edge; shapes larger than that run on as they did.
fn beyond_art_shift(xz: vec2<f32>) -> vec2<f32> {
    let out = max(-xz, xz - map_size_px());
    return vec2<f32>(smoothstep(0.0, 32.0, out.y), smoothstep(0.0, 32.0, out.x)) * 16.0;
}

fn mirror_point(xz: vec2<f32>, d: f32) -> Mirror {
    let p = xz + mirror_warp(xz, d);
    let size = map_size_px();
    let f = fract(p / (2.0 * size)) * 2.0;
    var m: Mirror;
    m.at = (1.0 - abs(f - 1.0)) * size;
    m.flip = select(vec2<f32>(1.0), vec2<f32>(-1.0), f > vec2<f32>(1.0));
    return m;
}

// How much of the ground at `xz`, `d` beyond the map, is the far land's (`far_ground`) rather than
// the map's mirror image: none within FAR_FROM of the edge, so that whatever crosses an edge
// carries on until the mirror has closed it, all of it from FAR_FULL; nearer or further as it
// goes along the edges, raggedly, so the change never runs along them in a line (where the
// map's land meets the far land's sea, there its coast ends, and a coast is ragged).
const FAR_FROM: f32 = 160.0;
const FAR_FULL: f32 = 700.0;

fn far_mix(xz: vec2<f32>, d: f32) -> f32 {
    // (The wobble lies between 0.45 and 1.55: nearer or further than that, it makes no odds.)
    if (d * 1.55 <= FAR_FROM) {
        return 0.0;
    }
    if (d * 0.45 >= FAR_FULL) {
        return 1.0;
    }
    let wobble = 1.0 + 0.5 * noise(xz / 1300.0 + vec2<f32>(2.3, 5.9)) + 0.25 * noise(xz / 420.0 + vec2<f32>(-7.1, 1.4));
    return smoothstep(FAR_FROM, FAR_FULL, d * wobble);
}

// Where round the map's edges (0..1, clockwise from its north-west corner, as the points of
// `surroundings` go) the line from the map's centre through `xz` leaves it.
fn round_the_edge(xz: vec2<f32>) -> f32 {
    let size = map_size_px();
    let half = size * 0.5;
    let v = xz - half;
    var s: f32;
    if (abs(v.x) * half.y > abs(v.y) * half.x) {
        // Through the east or the west edge.
        let z = half.y + v.y * half.x / abs(v.x);
        s = select(2.0 * (size.x + size.y) - z, size.x + z, v.x > 0.0);
    } else {
        // Through the north or the south edge.
        let x = half.x + v.x * half.y / max(abs(v.y), 1e-3);
        s = select(x, 2.0 * size.x + size.y - x, v.y > 0.0);
    }
    return s / (2.0 * (size.x + size.y));
}

// The map's ground round its edges at `s` (`round_the_edge`), from `field` (0: its shape, 1: its
// lighter colour, 2: its darker, 3: its sea's), averaged over a stretch of the edges about
// `width` world units long: three taps weighted 1, 2, 1, each between the points either side of
// it.
fn around_at(field: i32, s: f32, width: f32) -> vec4<f32> {
    let size = map_size_px();
    let step = 0.25 * width / (2.0 * (size.x + size.y));
    var sum = vec4<f32>(0.0);
    for (var t = -1; t <= 1; t++) {
        let p = (s + f32(t) * step) * f32(AROUND) - 0.5;
        let a = ((i32(floor(p)) % AROUND) + AROUND) % AROUND;
        let b = (a + 1) % AROUND;
        var lo: vec4<f32>;
        var hi: vec4<f32>;
        if (field == 0) {
            lo = surroundings.shape[a];
            hi = surroundings.shape[b];
        } else if (field == 1) {
            lo = surroundings.light[a];
            hi = surroundings.light[b];
        } else if (field == 2) {
            lo = surroundings.dark[a];
            hi = surroundings.dark[b];
        } else {
            lo = surroundings.sea[a];
            hi = surroundings.sea[b];
        }
        sum += mix(lo, hi, p - floor(p)) * f32(2 - abs(t));
    }
    return sum * 0.25;
}

// The lighter and the darker colours of the map's dry ground round its edges at `s`, as
// `around_at` reads them (both in one pass).
fn around_tones(s: f32, width: f32) -> array<vec3<f32>, 2> {
    let size = map_size_px();
    let step = 0.25 * width / (2.0 * (size.x + size.y));
    var light = vec3<f32>(0.0);
    var dark = vec3<f32>(0.0);
    for (var t = -1; t <= 1; t++) {
        let p = (s + f32(t) * step) * f32(AROUND) - 0.5;
        let a = ((i32(floor(p)) % AROUND) + AROUND) % AROUND;
        let b = (a + 1) % AROUND;
        let f = p - floor(p);
        let weight = f32(2 - abs(t));
        light += mix(surroundings.light[a].rgb, surroundings.light[b].rgb, f) * weight;
        dark += mix(surroundings.dark[a].rgb, surroundings.dark[b].rgb, f) * weight;
    }
    return array<vec3<f32>, 2>(light * 0.25, dark * 0.25);
}

// The far land at `xz`, `d` beyond the map: x = its height, y = how much of it lies under the sea
// (as `height_mip` gives them for the mirror image). It is made after the map's own ground round
// the nearest stretch of its edges (`around_at`, over a stretch about as long as the land is
// far), but its coasts and hills are its own: where that ground is part land and part sea, a
// coast wanders through the far land (noise), land and sea in about the shares they have there
// (where it is all land or all sea, no coast is made up); the land stands as high as the map's
// does there, rising and falling as much (and more, far out), rising from its shores, and the
// sea bed lies as deep, falling away from them. Its hills only with `relief` (the extended
// ground's vertices carry them; what lies under the water, and where the shore is, never needs
// them).
fn far_ground(xz: vec2<f32>, d: f32, relief: bool) -> vec2<f32> {
    let a = around_at(0, round_the_edge(xz), max(d, 800.0) * 1.5);
    let sea = frame.world.y;
    var land = a.y;
    if (relief) {
        let hills = noise(xz / 1800.0 + vec2<f32>(4.2, -1.3)) + 0.5 * noise(xz / 760.0 - vec2<f32>(2.7, 6.1));
        // (Never below the lowest ground TA has.)
        land = max(a.y + hills * max(clamp(a.w * 1.5, 6.0, 80.0), 50.0 * smoothstep(1500.0, 8000.0, d)), 0.0);
    }
    if (sea <= 0.0) {
        return vec2<f32>(land, 0.0);
    }
    let mixed = smoothstep(0.0, 0.1, a.x) * smoothstep(0.0, 0.1, 1.0 - a.x);
    var raw = a.x;
    // (How fast `raw` changes, per world unit.)
    var slope = 0.0;
    if (mixed > 0.0) {
        if (relief) {
            let n1 = noise_d(xz / 5200.0 + vec2<f32>(1.3, -4.1));
            let n2 = noise_d(xz / 2300.0 + vec2<f32>(-6.2, 2.9));
            let n3 = noise_d(xz / 1000.0 + vec2<f32>(3.7, 8.1));
            raw += (n1.x + 0.5 * n2.x + 0.25 * n3.x) * 1.6 * mixed;
            slope = length(n1.yz / 5200.0 + 0.5 * n2.yz / 2300.0 + 0.25 * n3.yz / 1000.0) * 1.6 * mixed;
        } else {
            let n = noise(xz / 5200.0 + vec2<f32>(1.3, -4.1)) + 0.5 * noise(xz / 2300.0 + vec2<f32>(-6.2, 2.9)) + 0.25 * noise(xz / 1000.0 + vec2<f32>(3.7, 8.1));
            raw += n * 1.6 * mixed;
        }
    }
    // (Its shores sharp where the shore is drawn, per pixel: the land soon above the water,
    // the bed soon deep, no shallows for miles. For the extended ground's vertices (`relief`),
    // by the distance from the shore (`raw` over its slope), in vertex spacings: a coastal band
    // lying just under the water from two spacings out to one and a half inland, so the band
    // holds the shore between the vertices and the water pass draws it, never the triangles'
    // edges (see `skirt_height`); the land rising gently from it over the next two and a half.)
    var w = smoothstep(0.47, 0.53, raw);
    if (relief) {
        let spacing = max(16.0, 0.2 * d);
        let inland = (0.5 - raw) / max(slope, 1e-7) / spacing;
        if (inland >= 0.0) {
            w = max(max(0.5 - 0.25 * inland / 1.5, 0.25 - 0.15 * (inland - 1.5) / 2.5), 0.0);
        } else {
            w = min(0.5 - 0.2 * inland, 1.0);
        }
    }
    land = mix(sea + 2.0, max(land, sea + 2.0), smoothstep(0.5, 0.3, raw));
    let bed = mix(sea - 4.0, min(a.z, sea - 4.0), smoothstep(0.5, 0.7, raw));
    return vec2<f32>(mix(land, bed, w), w);
}

// The colour of the far land's dry ground at `xz`, `d` beyond the map, seen `footprint` world
// units a pixel: the map's own round the nearest stretch of its edges, its lighter and its
// darker grounds lying on each other in patches of every size from a couple of thousand units
// down (bent by a warp, so they never read as blobs), their edges soft but not blurred, each
// patch shaded unevenly, and up close a speckle as the paint has; the patches `contrast` as far
// apart as they are (a surface of its own shading the ground instead: see `far_surface`).
fn far_color(xz: vec2<f32>, d: f32, footprint: f32, contrast: f32) -> vec3<f32> {
    let s = round_the_edge(xz);
    let width = max(d, 800.0) * 1.5;
    let tones = around_tones(s, width);
    let lighter = tones[0];
    let darker = tones[1];
    let middle = 0.5 * (lighter + darker);
    // (Its lighter and darker grounds no further apart in brightness than half again, and
    // closer still where they are grey: on a metal world or the moon the darker half is the
    // plating's or the rocks' shadows and the towers', which as patches of their own lay over the
    // far land like smoke.)
    let spread = 0.5 * (lighter - darker);
    let rec709 = vec3<f32>(0.2126, 0.7152, 0.0722);
    let top = max(max(middle.r, middle.g), middle.b);
    let grey = 1.0 - smoothstep(0.08, 0.25, (top - min(min(middle.r, middle.g), middle.b)) / max(top, 1e-4));
    let room = min(mix(0.2, 0.07, grey) * dot(middle, rec709) / max(dot(abs(spread), rec709), 1e-4), 1.0) * contrast;
    let light = middle + spread * room;
    let dark = middle - spread * room;
    let q = xz + vec2<f32>(0.8, 0.6) * noise(xz / 900.0 + vec2<f32>(1.7, 9.2)) * 500.0;
    // (Each octave fades out before a pixel covers as much as a quarter of its size, and is not
    // worked out at all from then: most of the far land lies far off, a pixel covering much.)
    var p = noise(q / 2400.0 + vec2<f32>(-3.3, 7.4));
    let mid = 1.0 - smoothstep(130.0, 250.0, footprint);
    if (mid > 0.0) {
        p += 0.55 * noise(q / 1000.0 + vec2<f32>(5.1, -2.2)) * mid;
    }
    let small = 1.0 - smoothstep(55.0, 105.0, footprint);
    if (small > 0.0) {
        p += 0.3 * noise(q / 420.0 + vec2<f32>(-8.3, 1.6)) * small;
    }
    // (Softer than a pixel's change in the pattern, so they never shimmer.)
    let soft = max(0.12, 0.0012 * footprint);
    let c = mix(mix(middle, dark, 0.55), mix(middle, light, 0.55), smoothstep(-soft, soft, p));
    // (Each patch shaded unevenly: by the pattern itself, and up close finer.)
    var shade = 0.25 * (p - clamp(p, -soft, soft));
    if (footprint < 90.0) {
        shade += 0.07 * noise(q / 110.0 + vec2<f32>(-2.4, 8.8)) * (1.0 - smoothstep(30.0, 90.0, footprint));
    }
    if (footprint < 14.0) {
        shade += 0.08 * noise(xz / 26.0 + vec2<f32>(5.5, -3.3)) * (1.0 - smoothstep(4.0, 14.0, footprint));
    }
    return max(c * (1.0 + shade), vec3<f32>(0.0));
}

// The colour of the far land's sea bed at `xz`, `d` beyond the map: the map's own sea as painted
// round the nearest stretch of its edges, so that the sea runs on from the map's in one colour.
fn far_sea_color(xz: vec2<f32>, d: f32) -> vec3<f32> {
    let c = around_at(3, round_the_edge(xz), max(d, 800.0) * 1.5).rgb;
    // (No sea round the map at all: the sea beyond it as the map's is painted.)
    return select(c, frame.beyond_sea.rgb, dot(c, vec3<f32>(1.0)) <= 0.0);
}

// The ground at `xz`, `d` beyond the map (x = its mean height, y = how much of it lies under the
// sea, as coarse as the extended ground's vertices are there): next to the map the mirror
// image's (read at `at`, its point, see `mirror_point`), further out the far land's. (As the far
// land takes over, the mirror image is read ever more coarsely: where the far land is sea, the
// map's land crossing the edge shrinks back from its own coasts towards its heart, as land
// sinks, rather than being cut off along a line where the two meet.)
fn beyond_ground_with(at: vec2<f32>, xz: vec2<f32>, d: f32) -> vec2<f32> {
    return beyond_ground_from(at, xz, d, far_mix(xz, d), false);
}

// `beyond_ground_with`, the far land's share there (`far_mix`) known; the far land's hills with
// `relief` (see `far_ground`).
fn beyond_ground_from(at: vec2<f32>, xz: vec2<f32>, d: f32, k: f32, relief: bool) -> vec2<f32> {
    if (k >= 1.0) {
        return far_ground(xz, d, relief);
    }
    let g = height_mip(at, mix(beyond_lod(d), 5.0, k));
    if (k <= 0.0) {
        return g;
    }
    return mix(g, far_ground(xz, d, relief), k);
}

// `beyond_ground_with`, the mirror image's point found here.
fn beyond_ground(xz: vec2<f32>, d: f32) -> vec2<f32> {
    return beyond_ground_relief(xz, d, false);
}

// `beyond_ground`, the far land's hills with `relief`.
fn beyond_ground_relief(xz: vec2<f32>, d: f32, relief: bool) -> vec2<f32> {
    let k = far_mix(xz, d);
    if (k >= 1.0) {
        return far_ground(xz, d, relief);
    }
    return beyond_ground_from(mirror_point(xz, d).at, xz, d, k, relief);
}

// Mip level at which the ground `d` beyond the map reads the heightmap: texels as far apart
// as the skirt's rings are there (sky.rs), so the vertices carry the reflected relief's shape
// (the mesh is fixed in the world: nothing shimmers); exact next to the map.
fn beyond_lod(d: f32) -> f32 {
    return clamp(log2(max(d * 0.15 / 16.0, 1.0)), 0.0, 9.0);
}

// The heightmap's mip `lod` around map position `at`: x = the mean height there, y = the
// fraction of the ground there under sea level.
fn height_mip(at: vec2<f32>, lod: f32) -> vec2<f32> {
    let s = textureSampleLevel(heightmap, linear_clamp, (at / 16.0 + 0.5) / vec2<f32>(params.cells), lod);
    return vec2<f32>(s.r * 255.0, s.g);
}

// How far the extended ground at distance `d` from the map draws its coasts per pixel (the
// water pass's shoreline where `beyond_wet` crosses 0.5; the vertices along the coast lie
// just under the water) rather than between its vertices, as on the map: exact at the edge,
// where the vertices are as close as the heightmap's.
fn beyond_coast_mix(d: f32) -> f32 {
    return smoothstep(8.0, 128.0, d);
}

// Fraction of the ground under sea level around world `xz`: the heightmap's box-filtered mips,
// averaged over about 2^`level` cells.
fn sea_fraction(xz: vec2<f32>, level: f32) -> f32 {
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(params.cells);
    return textureSampleLevel(heightmap, linear_clamp, uv, min(level, f32(textureNumLevels(heightmap) - 1u))).g;
}

// Heightmap (x = height, y = 1 under sea level) at world position `q`, bilinear.
fn height_sample(q: vec2<f32>) -> vec2<f32> {
    let s = textureSampleLevel(heightmap, linear_clamp, (q / 16.0 + 0.5) / vec2<f32>(params.cells), 0.0);
    return vec2<f32>(s.r * 255.0, s.g);
}

// Horizontal gradient of the heightmap's mip `lod` around map position `at` (height units per
// world unit): `smooth_gradient` at the mip's own scale.
fn mip_gradient(at: vec2<f32>, lod: f32) -> vec2<f32> {
    let e = 8.0 * exp2(lod);
    let gx = height_mip(at + vec2<f32>(e, 0.0), lod).x - height_mip(at - vec2<f32>(e, 0.0), lod).x;
    let gz = height_mip(at + vec2<f32>(0.0, e), lod).x - height_mip(at - vec2<f32>(0.0, e), lod).x;
    return vec2<f32>(gx, gz) / (2.0 * e);
}

// Fraction of the ground under sea level at `xz`, `d` beyond the map: the ground's there
// (`beyond_ground`: the reflected ground's next to the map, as coarse as the vertices there, so
// a coast never falls where no vertex can carry it; further out the far land's), with a little
// noise further out so shorelines meander at scales the mips no longer hold. Heights, the
// per-pixel water depth and the ground colour all decide land and sea by this one value, so
// they agree on where the shore is. 0.5 is the shoreline.
fn beyond_wet(xz: vec2<f32>, d: f32) -> f32 {
    return beyond_wet_from(beyond_ground(xz, d).y, xz, d);
}

// `beyond_wet` from the ground's own wet fraction there `w` (`beyond_ground`'s y).
fn beyond_wet_from(w: f32, xz: vec2<f32>, d: f32) -> f32 {
    let n = noise(xz / 190.0) + 0.5 * noise(xz / 73.0 + vec2<f32>(5.3, 1.7));
    return w + n * 0.12 * beyond_coast_mix(d);
}

// Depth of water at `xz`, `d` beyond the map, where the shoreline is drawn per pixel (see
// `beyond_wet`): deepening quickly seawards, so the water soon hides the coastal band's bed
// (just under the surface, see `skirt_height`); out on the open sea, no deeper than the
// reflected sea bed `skirt_height` draws there, so a sea whose bed shows through it on the map
// shows it beyond the edge too, instead of darkening in a band where the map ends. Negative on
// land.
fn beyond_water_depth(xz: vec2<f32>, d: f32) -> f32 {
    let g = beyond_ground(xz, d);
    let w = beyond_wet_from(g.y, xz, d);
    let ramp = (w - 0.5) * 200.0;
    return mix(ramp, min(ramp, max(frame.world.y - g.x, 6.0)), smoothstep(0.9, 0.98, w));
}

// Ground height beyond the map, `d` from it: the ground's there (`beyond_ground`: the
// reflected ground's next to the map, exact at its edge; further out the far land's, its own
// hills and all).
//
// The mips average land and sea bed, so near a coast land and sea are kept apart by
// `beyond_wet`: land stays above the water and sea bed below it. Next to the map the step
// between them falls between vertices, as on the map; further out the whole coastal band (wet
// fraction 0.25 to 0.9) lies just under the water, so the sparser vertices never cut the
// shoreline: the water pass draws it per pixel where the wet fraction crosses 0.5, and the
// ground shader colours land and sea bed per pixel alike.
fn skirt_height(xz: vec2<f32>, d: f32) -> f32 {
    let sea = frame.world.y;
    let g = beyond_ground_relief(xz, d, true);
    if (sea <= 0.0) {
        return g.x;
    }
    let k = beyond_coast_mix(d);
    let w = beyond_wet_from(g.y, xz, d);
    let land = max(g.x, sea + 0.3 + 0.7 * k);
    let bed = min(g.x, sea - 0.5 - 5.5 * k);
    let near = mix(land, bed, smoothstep(0.3, 0.7, w));
    let coast = sea - 0.4;
    var far = coast;
    if (w < 0.25) {
        far = mix(land, coast, smoothstep(0.1, 0.25, w));
    } else if (w > 0.9) {
        far = mix(coast, bed, smoothstep(0.9, 0.98, w));
    }
    return mix(near, far, k);
}

// Smoothed ground height anywhere: the bilinear heightmap inside the map, the extended ground
// beyond it (when drawn; otherwise the heightmap clamped at the edges).
fn ground_smooth(xz: vec2<f32>) -> f32 {
    if (FAR_LAND && beyond_on()) {
        let d = beyond_distance(xz);
        if (d > 0.0) {
            return skirt_height(xz, d);
        }
    }
    return smooth_height(xz);
}

// Horizontal gradient of `ground_smooth` (height units per world unit).
fn ground_gradient(xz: vec2<f32>) -> vec2<f32> {
    if (!(FAR_LAND && beyond_on())) {
        return smooth_gradient(xz);
    }
    let e = 8.0;
    let gx = ground_smooth(xz + vec2<f32>(e, 0.0)) - ground_smooth(xz - vec2<f32>(e, 0.0));
    let gz = ground_smooth(xz + vec2<f32>(0.0, e)) - ground_smooth(xz - vec2<f32>(0.0, e));
    return vec2<f32>(gx, gz) / (2.0 * e);
}

// Height of heightmap vertex `c`, the map continued as its mirror image when the ground beyond
// it is drawn (the slope across the west and north edges is then level, as the reflection
// makes it; past the east and south edges the clamped last vertex already repeats), else
// clamped at the edges as TA's ground always was.
fn height_across(c: vec2<i32>) -> f32 {
    if (beyond_on()) {
        return height_at(abs(c));
    }
    return height_at(c);
}

// Normal of heightmap vertex `cell` (central differences; see `height_across`).
fn vertex_normal(cell: vec2<i32>) -> vec3<f32> {
    let hx = height_across(cell + vec2<i32>(1, 0)) - height_across(cell - vec2<i32>(1, 0));
    let hz = height_across(cell + vec2<i32>(0, 1)) - height_across(cell - vec2<i32>(0, 1));
    return normalize(vec3<f32>(-hx, 32.0, -hz));
}

// Height and normal of an extended-ground vertex at `xz`. Vertices on the map edge use the
// terrain's own height and normal, so the seam is watertight and lit identically.
struct SkirtVertex {
    height: f32,
    normal: vec3<f32>,
    beyond: f32,
};

fn skirt_vertex(xz: vec2<f32>) -> SkirtVertex {
    var v: SkirtVertex;
    let d = beyond_distance(xz);
    v.beyond = d;
    if (d < 0.5) {
        let cell = vec2<i32>(round(xz / 16.0));
        v.height = height_at(cell);
        v.normal = vertex_normal(cell);
        return v;
    }
    v.height = skirt_height(xz, d);
    // Central differences over about a sixteenth of the distance (the local vertex spacing
    // is about a sixth of it), never finer than a heightmap cell.
    let e = max(16.0, d * 0.06);
    let hx = skirt_height(xz + vec2<f32>(e, 0.0), beyond_distance(xz + vec2<f32>(e, 0.0)))
        - skirt_height(xz - vec2<f32>(e, 0.0), beyond_distance(xz - vec2<f32>(e, 0.0)));
    let hz = skirt_height(xz + vec2<f32>(0.0, e), beyond_distance(xz + vec2<f32>(0.0, e)))
        - skirt_height(xz - vec2<f32>(0.0, e), beyond_distance(xz - vec2<f32>(0.0, e)));
    v.normal = normalize(vec3<f32>(-hx, 2.0 * e, -hz));
    return v;
}

// Height of an extended-ground vertex at `xz` (as `skirt_vertex`, without the normal).
fn skirt_vertex_height(xz: vec2<f32>) -> f32 {
    let d = beyond_distance(xz);
    if (d < 0.5) {
        return height_at(vec2<i32>(round(xz / 16.0)));
    }
    return skirt_height(xz, d);
}

// The art the ground at `xz` shows where its art pixel `a` lies outside the painted image (the
// strip along the north edge TA never painted, and everything beyond the map, `beyond` > 0):
// beyond the map, the art pixel the mirrored ground shows (`mirror_point`); on the strip,
// which has no ground to mirror, the art reflected across the image's edge. The strip is as
// deep as half the ground's height, and the rows reflected into it may show something else
// than the ground there (often the dark face of a hill that hid it from TA's camera, or its
// crest): the reflection keeps a subdued copy of its texture, its marks held to a faint shading,
// on the light and colour of the image's first rows (see `shown_color`). Those are taken some
// 16 px down and a hundred along them: taken 16 px along them, the same all the way down the
// strip, they drew every blotch along them as a streak over it (twice as long again where the
// ground beyond the map mirrors it); taken 64 px down, a dark mark further in spread over the
// strip as a dark band and blob. Inside the image on the map, `a` itself.
struct ShownArt {
    // The art pixel shown, inside the painted image.
    px: vec2<f32>,
    // On the strip: the colour of the image's first rows there (`base`) and of the reflected
    // art around `px` (`own`, about 16 px wide), and how much of the art's contrast against
    // that is kept (`keep`); 1, 1 and 1 elsewhere.
    base: vec3<f32>,
    own: vec3<f32>,
    keep: f32,
    // Per axis, -1 where the art shown runs backwards (its gradients turn round), else 1.
    flip: vec2<f32>,
    // The ground point inside the map whose art it is (`xz` on the map), and per axis -1 where
    // the ground is that point's mirror image (`Mirror::flip`; 1 on the map).
    at: vec2<f32>,
    mirrored: vec2<f32>,
    // How much the art blurs with the distance from the map (mip levels).
    blur: f32,
    // How much of the ground is the far land's (see `far_mix`), in its own colours
    // (`far_color`) rather than the mirror image's art.
    far: f32,
};

// How deep (art pixels) into the unpainted strip the reflected art takes on the light and
// colour of the image's first rows (and how many of those rows it takes them from).
const STRIP_TONE: f32 = 8.0;

fn shown_art(a: vec2<f32>, xz: vec2<f32>, beyond: f32) -> ShownArt {
    var s: ShownArt;
    s.px = a;
    s.base = vec3<f32>(1.0);
    s.own = vec3<f32>(1.0);
    s.keep = 1.0;
    s.flip = vec2<f32>(1.0);
    s.at = xz;
    s.mirrored = vec2<f32>(1.0);
    s.blur = 0.0;
    s.far = 0.0;
    if (!beyond_on()) {
        return s;
    }
    var art = a;
    if (FAR_LAND && beyond > 0.0) {
        s.far = far_mix(xz, beyond);
        // (All the far land's: no art of the map's shown, only its point's, inside the image.)
        if (s.far >= 1.0) {
            s.at = clamp(xz, vec2<f32>(0.0), map_size_px());
            s.px = s.at;
            s.blur = 4.5;
            return s;
        }
        let m = mirror_point(xz, beyond);
        s.at = m.at;
        s.mirrored = m.flip;
        s.flip = m.flip;
        art = vec2<f32>(m.at.x, m.at.y - smooth_height(m.at) * 0.5) + beyond_art_shift(xz);
        // (Near a corner the shift can carry the art past the map's far side: there it is
        // reflected back as the mirror is, not clamped into streaks of the edge's last texels.)
        let size = map_size_px();
        if (art.x > size.x) {
            art.x = 2.0 * size.x - art.x;
            s.flip.x = -s.flip.x;
        }
        if (art.y > size.y) {
            art.y = 2.0 * size.y - art.y;
            s.flip.y = -s.flip.y;
        }
        s.blur = clamp(log2(1.0 + beyond / 1200.0), 0.0, 4.5);
    }
    // (Only the north strip lies outside the image: the mirror brings every point beyond the
    // map back inside it, and ground only ever shows art north of itself.)
    s.px = vec2<f32>(art.x, abs(art.y));
    if (art.y < 0.0) {
        s.flip.y = -s.flip.y;
        let k = smoothstep(0.0, STRIP_TONE, -art.y);
        s.own = max(art_sample(s.px, 4.0, 0.0), vec3<f32>(1e-3));
        var first = vec3<f32>(0.0);
        for (var i = -2; i <= 2; i++) {
            first += art_sample(vec2<f32>(art.x + f32(i) * 24.0, min(-art.y, STRIP_TONE)), 4.0, 0.0) * 0.2;
        }
        s.base = mix(s.own, first, k);
        s.keep = mix(1.0, 0.5, k);
    }
    return s;
}

// The colour `c` of the art shown at `s.px` (or around it) as the ground shows it: itself, but
// on the unpainted strip with its contrast against the reflected art around it subdued, on
// the colour of the image's first rows.
fn shown_color(c: vec3<f32>, s: ShownArt) -> vec3<f32> {
    if (s.keep >= 1.0) {
        return c * (s.base / s.own);
    }
    // (Its contrast held within a fifth or so either way, in full from the first rows' depth.)
    let lo = mix(0.25, 0.8, 2.0 * (1.0 - s.keep));
    return s.base * pow(clamp(max(c, vec3<f32>(1e-4)) / s.own, vec3<f32>(lo), vec3<f32>(1.0 / lo)), vec3<f32>(s.keep));
}

// Art pixel of a surface `height` high over world `xz` (the water's, at sea level), as
// `shown_art` finds it: beyond the map that of the mirrored point.
fn surface_art_px(xz: vec2<f32>, height: f32) -> vec2<f32> {
    if (!beyond_on()) {
        return vec2<f32>(xz.x, xz.y - height * 0.5);
    }
    var at = xz;
    var shift = vec2<f32>(0.0);
    let d = beyond_distance(xz);
    if (FAR_LAND && d > 0.0) {
        at = mirror_point(xz, d).at;
        shift = beyond_art_shift(xz);
    }
    return mirror_art(vec2<f32>(at.x, at.y - height * 0.5) + shift);
}
