// The ground's heights and the shadows the relief casts (see terrain_shadow.rs): for every
// texel of the map, r = the ground height / 255, g = how far above the ground (world units /
// 255) the light clears the ground between it and the light, b = the metal deposit's richness
// there, a = how much of the ground around lies under the sea (`lava_nearness`). A point `y` units above the ground is in the shadow of the relief when `y` is less
// than the clearance.

struct Params {
    // Texels of the map per axis.
    size: vec2<u32>,
    // Heightmap vertices per axis (16 world units apart).
    cells: vec2<u32>,
    // Unit direction (world xz) towards the light.
    dir: vec2<f32>,
    // Rise of the light ray per world unit it travels across the map.
    tan_elevation: f32,
    // World units per texel.
    texel: f32,
    // The highest ground on the map: the march ends once the ray is above it.
    max_height: f32,
    pad0: f32,
    // The first texel filled (a part of the map filled again, where its heights changed).
    origin: vec2<u32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var heightmap: texture_2d<f32>;
@group(0) @binding(2) var height_sampler: sampler;
@group(0) @binding(3) var clearance: texture_storage_2d<rgba8unorm, write>;
// Ore per heightmap cell (0..1), and how much sea lies around each.
@group(0) @binding(4) var ore_map: texture_2d<f32>;
@group(0) @binding(5) var sea_near: texture_2d<f32>;

// Ground height at world xz (bilinear between the heightmap vertices).
fn height(xz: vec2<f32>) -> f32 {
    let uv = (xz / 16.0 + 0.5) / vec2<f32>(p.cells);
    return textureSampleLevel(heightmap, height_sampler, uv, 0.0).r * 255.0;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let id = gid.xy + p.origin;
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let xz = (vec2<f32>(id) + 0.5) * p.texel;
    let extent = vec2<f32>(p.cells) * 16.0;
    let ground = height(xz);
    var clear = 0.0;
    // Steps grow with the distance: nearby ridges are found exactly, far ones (which only
    // matter under a low sun) more coarsely.
    var s = p.texel * 0.5;
    for (var i = 0; i < 200; i++) {
        let rise = s * p.tan_elevation;
        if (ground + rise >= p.max_height) {
            break;
        }
        let q = xz + p.dir * s;
        if (any(q < vec2<f32>(0.0)) || any(q > extent)) {
            break;
        }
        clear = max(clear, height(q) - ground - rise);
        s += max(p.texel * 0.5, s * 0.045);
    }
    let cells = xz / (vec2<f32>(p.cells) * 16.0);
    let ore = textureSampleLevel(ore_map, height_sampler, cells, 0.0).r;
    let sea = textureSampleLevel(sea_near, height_sampler, cells, 0.0).r;
    textureStore(clearance, vec2<i32>(id), vec4<f32>(ground / 255.0, clear / 255.0, ore, sea));
}
