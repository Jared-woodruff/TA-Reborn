// Depth of field (photo mode, see post.rs). Three passes: the scene's depth turned into
// distances from the eye; for each tile of 16 × 16 pixels the largest blur in it; then each
// pixel gathers the scene over its circle of confusion. In "lens" mode a point blurs as far as
// it lies from the focus distance, in "tilt-shift" mode as far as it lies from a sharp band
// across the picture (the miniature look). Prepended: fullscreen.wgsl.

struct DofUniform {
    // x = mode (1 lens, 2 tilt-shift, 3 uniform), y = focus (lens: distance from the eye; tilt-shift: the
    // sharp band's height on the picture, 0 top to 1 bottom), z = the largest blur's radius in
    // pixels, w = tilt-shift: the sharp band's half height (0..1)
    params: vec4<f32>,
    // Depth to distance: x, y = the near and far planes (a perspective camera), or with z = 1
    // an offset and a scale (distance = x + y depth: the classic camera). w = how many tiles
    // around its own a pixel's blur can reach from.
    clip: vec4<f32>,
};

@group(0) @binding(0) var<uniform> dof: DofUniform;

// Tile size in pixels (see TILE in post.rs).
const TILE: i32 = 16;

// ---- Distances ------------------------------------------------------------------------------

@group(0) @binding(1) var scene_depth: texture_depth_multisampled_2d;

@fragment
fn fs_distance(in: VsOut) -> @location(0) vec4<f32> {
    let z = textureLoad(scene_depth, vec2<i32>(in.pos.xy), 0);
    if (dof.clip.z > 0.5) {
        return vec4<f32>(dof.clip.x + dof.clip.y * z, 0.0, 0.0, 1.0);
    }
    let n = dof.clip.x;
    let f = dof.clip.y;
    return vec4<f32>(n * f / max(f - z * (f - n), 1e-6), 0.0, 0.0, 1.0);
}

// ---- Blur radii -----------------------------------------------------------------------------

@group(0) @binding(2) var distances: texture_2d<f32>;

// Radius (pixels) of the blur of a point `distance` from the eye, at `uv` on the picture.
fn confusion(distance: f32, uv: vec2<f32>) -> f32 {
    let blur = dof.params.z;
    // (Uniform: all of it alike, as through frosted glass.)
    if (dof.params.x > 2.5) {
        return blur;
    }
    if (dof.params.x > 1.5) {
        let band = dof.params.w;
        return blur * smoothstep(band, band + 0.32, abs(uv.y - dof.params.y));
    }
    let focus = dof.params.y;
    // (Sharp within `band` of the focus either side, a share of the distance; the blur grows
    // over a stretch that lengthens with it.)
    let band = dof.params.w;
    return blur * saturate((abs(distance - focus) / max(distance, 1.0) - band) / (0.4 + 1.5 * band));
}

fn confusion_at(p: vec2<i32>, size: vec2<i32>) -> f32 {
    return confusion(textureLoad(distances, p, 0).r, (vec2<f32>(p) + 0.5) / vec2<f32>(size));
}

// The largest blur radius in each tile.
@fragment
fn fs_tiles(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(distances));
    let origin = vec2<i32>(in.pos.xy) * TILE;
    var largest = 0.0;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            let p = min(origin + vec2<i32>(x, y), size - 1);
            largest = max(largest, confusion_at(p, size));
        }
    }
    return vec4<f32>(largest, 0.0, 0.0, 1.0);
}

// ---- The blur -------------------------------------------------------------------------------

@group(0) @binding(3) var scene_color: texture_2d<f32>;
@group(0) @binding(4) var tiles: texture_2d<f32>;

// Samples over the pixel's own circle, then over the ring out to the largest blur around it
// (what a blurred thing in front can spread over it from), each on a golden-angle spiral.
const OWN_TAPS: u32 = 48u;
const RING_TAPS: u32 = 32u;
const GOLDEN_ANGLE: f32 = 2.39996323;

struct Gather {
    sum: vec3<f32>,
    weight: f32,
};

// Adds the sample `r` pixels out at angle `a` (weighing `w`, the area it stands for). A sample
// reaches this pixel when its blur is at least that far; what lies behind reaches it no
// farther than this pixel's own blur, so a sharp thing in front of a blur stays sharp. A
// sample that does not reach stands in for the colour gathered so far (Gustafsson's
// single-pass gather), so the result stays weighed by area.
fn gather(g: Gather, px: vec2<f32>, size: vec2<i32>, r: f32, a: f32, w: f32, d0: f32, c0: f32) -> Gather {
    let p = clamp(vec2<i32>(px + vec2<f32>(cos(a), sin(a)) * r), vec2<i32>(0), size - 1);
    let c = textureLoad(scene_color, p, 0).rgb;
    let d = textureLoad(distances, p, 0).r;
    let cs = confusion(d, (vec2<f32>(p) + 0.5) / vec2<f32>(size));
    let reach = select(min(cs, c0), cs, d < d0);
    let m = saturate(reach - r + 1.0);
    var out: Gather;
    out.sum = g.sum + mix(g.sum / g.weight, c, m) * w;
    out.weight = g.weight + w;
    return out;
}

@fragment
fn fs_dof(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(scene_color));
    let px = vec2<i32>(in.pos.xy);
    let centre = textureLoad(scene_color, px, 0);
    let d0 = textureLoad(distances, px, 0).r;
    let c0 = confusion(d0, in.uv);
    // The largest blur in the tiles around.
    let tile_count = vec2<i32>(textureDimensions(tiles));
    let tile = px / TILE;
    let k = i32(dof.clip.w);
    var reach = c0;
    for (var y = -k; y <= k; y++) {
        for (var x = -k; x <= k; x++) {
            reach = max(reach, textureLoad(tiles, clamp(tile + vec2<i32>(x, y), vec2<i32>(0), tile_count - 1), 0).r);
        }
    }
    if (reach < 0.5) {
        return centre;
    }
    let at = in.pos.xy;
    var g: Gather;
    g.sum = centre.rgb;
    g.weight = 1.0;
    let own = select(0.0, c0, c0 >= 0.5);
    if (own > 0.0) {
        let own_w = 3.14159265 * own * own / f32(OWN_TAPS);
        for (var i = 0u; i < OWN_TAPS; i++) {
            let r = sqrt((f32(i) + 0.5) / f32(OWN_TAPS)) * own;
            g = gather(g, at, size, r, f32(i) * GOLDEN_ANGLE, own_w, d0, c0);
        }
    }
    if (reach > own + 1.0) {
        let span = reach * reach - own * own;
        let ring_w = 3.14159265 * span / f32(RING_TAPS);
        for (var i = 0u; i < RING_TAPS; i++) {
            let r = sqrt(own * own + (f32(i) + 0.5) / f32(RING_TAPS) * span);
            g = gather(g, at, size, r, f32(i) * GOLDEN_ANGLE + 1.3, ring_w, d0, c0);
        }
    }
    // (The alpha carries the sky mask and the heat haze on to the composite.)
    return vec4<f32>(g.sum / g.weight, centre.a);
}
