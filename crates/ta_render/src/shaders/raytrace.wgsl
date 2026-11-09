enable wgpu_ray_query;

// Ray tracing (see raytrace.rs): shadows, ambient occlusion and reflections of the models,
// traced against the models themselves with hardware ray queries, for every pixel of the depth
// prepass; then a depth-aware blur turns the few rays per pixel into smooth shade.


struct Params {
    inv_view_proj: mat4x4<f32>,
    // xyz = the eye, w = the raster frame's texture-animation time in seconds
    eye: vec4<f32>,
    // xyz = towards the light (the sun, or the moon at night), w = tan of the radius of its
    // disc (how soft shadows grow away from what casts them)
    light: vec4<f32>,
    // xy = size of the targets in pixels
    size: vec4<u32>,
    // x = near, y = far plane (depth to distance), z = reach of the ambient occlusion's rays
    // (world units), w = its strength
    depth: vec4<f32>,
    // The point lights' grid (lights.rs): xy = world xz of its corner, z = 1 / cell size (0: no
    // lights), w = unused
    grid: vec4<f32>,
    // xy = cells per axis, z = light count, w = bit 0 colour-blind palette, bit 1 Reborn
    grid_dims: vec4<u32>,
    // The light on what reflections show: rgb = the sun's (or moon's), and the sky's
    sun_color: vec4<f32>,
    ambient: vec4<f32>,
    // xy = 1 / the world extent of the ground map, z = how wet the ground is (0..1; wet ground
    // is traced for its reflections), w = 1 when models are traced for theirs
    ground: vec4<f32>,
};

// A traced piece (raytrace.rs): where its model's vertices start in the shared vertex buffer and
// its triangles in the index buffer.
struct TracedPiece {
    first_vertex: u32,
    first_index: u32,
};

// One point light (lights.rs): xyz = position, w = radius (negative: it casts no shadows);
// rgb = colour, w = 1 / radius².
struct PointLightGpu {
    pos_radius: vec4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var depth_map: texture_depth_2d;
@group(0) @binding(2) var scene: acceleration_structure;
@group(0) @binding(3) var raw: texture_storage_2d<rgba16float, write>;
// The blur passes: what they blur, and where they write.
@group(0) @binding(4) var blur_src: texture_2d<f32>;
@group(0) @binding(5) var blur_mid: texture_storage_2d<rgba16float, write>;
@group(0) @binding(6) var blur_out: texture_storage_2d<rgba32float, write>;
// The point lights and their grid (the frame's own, see lights.rs).
@group(0) @binding(7) var<storage, read> point_lights: array<PointLightGpu>;
@group(0) @binding(8) var<storage, read> light_grid: array<u32>;
// Reflections: what the mirror ray from each pixel meets (rgb, lit), a = 1 where it meets
// something (else the sky the scene's shaders mirror themselves).
@group(0) @binding(9) var reflections: texture_storage_2d<rgba16float, write>;
// What reflections are shaded from: the traced pieces, the models' vertices (13 words each, see
// ModelVertex in model.rs) and indices, their textures, and the ground's heights.
@group(0) @binding(10) var<storage, read> traced_pieces: array<TracedPiece>;
@group(0) @binding(11) var<storage, read> model_vertices: array<u32>;
@group(0) @binding(12) var<storage, read> model_indices: array<u32>;
@group(0) @binding(13) var model_textures: texture_2d_array<f32>;
@group(0) @binding(14) var linear_sampler: sampler;
@group(0) @binding(15) var ground_map: texture_2d<f32>;
// Per top-level instance (the hit's instance index, a piece): its treads' travel, its player's
// colour, and its state: bit 0 its animated textures play, bits 8-15 how far its unit's lights
// are out (raytrace.rs `TracedInstance`).
struct TracedInstance {
    travel: vec2<f32>,
    team: u32,
    state: u32,
}
@group(0) @binding(19) var<storage, read> traced_instances: array<TracedInstance>;
@group(0) @binding(20) var reborn_textures: texture_2d_array<f32>;
@group(0) @binding(21) var reborn_physical: texture_2d_array<f32>;
@group(0) @binding(22) var skin_textures: texture_2d_array<f32>;
@group(0) @binding(23) var enemy_skin_textures: texture_2d_array<f32>;
// The skins' liveries (livery.wgsl, prepended): the own bank's and the enemy bank's.
@group(0) @binding(24) var<uniform> liveries: Liveries;
// The paint's second bank (model.rs `TEXTURE_BANKS`), for the layers past the first's (with one
// bank, every layer is in the first, and these are stand-ins nothing samples).
@group(0) @binding(25) var model_textures_2: texture_2d_array<f32>;
@group(0) @binding(26) var reborn_textures_2: texture_2d_array<f32>;
@group(0) @binding(27) var reborn_physical_2: texture_2d_array<f32>;

// A layer of the paint, the redrawn paint or its finish, from whichever bank holds it, at `mip`.
fn model_paint(uv: vec2<f32>, layer: u32, mip: f32) -> vec4<f32> {
    let first = textureNumLayers(model_textures);
    if (layer >= first) {
        return textureSampleLevel(model_textures_2, linear_sampler, uv, layer - first, mip);
    }
    return textureSampleLevel(model_textures, linear_sampler, uv, layer, mip);
}

fn reborn_paint(uv: vec2<f32>, layer: u32, mip: f32) -> vec4<f32> {
    let first = textureNumLayers(model_textures);
    if (layer >= first) {
        return textureSampleLevel(reborn_textures_2, linear_sampler, uv, layer - first, mip);
    }
    return textureSampleLevel(reborn_textures, linear_sampler, uv, layer, mip);
}

fn reborn_finish(uv: vec2<f32>, layer: u32, mip: f32) -> vec4<f32> {
    let first = textureNumLayers(model_textures);
    if (layer >= first) {
        return textureSampleLevel(reborn_physical_2, linear_sampler, uv, layer - first, mip);
    }
    return textureSampleLevel(reborn_physical, linear_sampler, uv, layer, mip);
}

// Words per model vertex (ModelVertex in model.rs): position (3), normal, uv, layer, colour,
// piece, edge distances (2), edge weights, treads (2). A physical finish keeps its roughness
// in normal.w and its metallic in colour.a.
const VERTEX_WORDS: u32 = 13u;

const TAU: f32 = 6.2831853;
// Rays per pixel towards the light, and over the hemisphere for the ambient occlusion.
const SHADOW_RAYS: i32 = 2;
const AO_RAYS: i32 = 2;
// How far a shadow can reach (world units): past the tallest models from far up the map.
const SHADOW_REACH: f32 = 3000.0;
// How far reflections look (world units).
const REFLECTION_REACH: f32 = 600.0;

fn load_depth(pix: vec2<i32>) -> f32 {
    return textureLoad(depth_map, clamp(pix, vec2<i32>(0), vec2<i32>(p.size.xy) - 1), 0);
}

// World position of pixel `pix` at depth `z`.
fn position(pix: vec2<i32>, z: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pix) + 0.5) / vec2<f32>(p.size.xy);
    let h = p.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0);
    return h.xyz / h.w;
}

// Distance from the eye along the view of depth `z`.
fn linear_depth(z: f32) -> f32 {
    let n = p.depth.x;
    let f = p.depth.y;
    return n * f / max(f - z * (f - n), 1e-6);
}

// Surface normal at pixel `pix` (world position `w`) from the depth around it: of the two
// neighbours along each axis, the one nearer in depth, so that edges do not bend it.
fn normal_at(pix: vec2<i32>, w: vec3<f32>) -> vec3<f32> {
    let r = position(pix + vec2<i32>(1, 0), load_depth(pix + vec2<i32>(1, 0)));
    let l = position(pix - vec2<i32>(1, 0), load_depth(pix - vec2<i32>(1, 0)));
    let d = position(pix + vec2<i32>(0, 1), load_depth(pix + vec2<i32>(0, 1)));
    let u = position(pix - vec2<i32>(0, 1), load_depth(pix - vec2<i32>(0, 1)));
    let dx = select(w - l, r - w, distance(r, w) < distance(l, w));
    let dy = select(w - u, d - w, distance(d, w) < distance(u, w));
    var n = normalize(cross(dy, dx));
    if (dot(n, p.eye.xyz - w) < 0.0) {
        n = -n;
    }
    return n;
}

// Interleaved gradient noise (Jimenez): a well spread number in 0..1 per pixel, `k` a different
// sequence for each ray.
fn ign(pix: vec2<f32>, k: f32) -> f32 {
    let q = pix + k * vec2<f32>(5.588238, 5.588238);
    return fract(52.9829189 * fract(dot(q, vec2<f32>(0.06711056, 0.00583715))));
}

// Two unit vectors square to `axis`.
fn basis(axis: vec3<f32>) -> mat2x3<f32> {
    let t = normalize(cross(axis, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(axis.y) > 0.9)));
    return mat2x3<f32>(t, cross(axis, t));
}

// A direction through the light's disc (radius `spread`, as a tangent), from two numbers in 0..1.
fn towards_disc(axis: vec3<f32>, spread: f32, u1: f32, u2: f32) -> vec3<f32> {
    let b = basis(axis);
    let r = sqrt(u1) * spread;
    let a = u2 * TAU;
    return normalize(axis + (b[0] * cos(a) + b[1] * sin(a)) * r);
}

// A cosine-weighted direction over the hemisphere around `n`, from two numbers in 0..1.
fn over_hemisphere(n: vec3<f32>, u1: f32, u2: f32) -> vec3<f32> {
    let b = basis(n);
    let r = sqrt(u1);
    let a = u2 * TAU;
    return normalize((b[0] * cos(a) + b[1] * sin(a)) * r + n * sqrt(max(1.0 - u1, 0.0)));
}

// Whether anything lies along the ray within `reach`.
fn blocked(origin: vec3<f32>, dir: vec3<f32>, reach: f32) -> bool {
    var rq: ray_query;
    rayQueryInitialize(&rq, scene, RayDesc(RAY_FLAG_TERMINATE_ON_FIRST_HIT | RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, reach, origin, dir));
    loop {
        if (!rayQueryProceed(&rq)) {
            break;
        }
    }
    return rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE;
}

// Distance along the ray to the nearest thing within `reach` (`reach` when there is none).
fn nearest(origin: vec3<f32>, dir: vec3<f32>, reach: f32) -> f32 {
    var rq: ray_query;
    rayQueryInitialize(&rq, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, reach, origin, dir));
    loop {
        if (!rayQueryProceed(&rq)) {
            break;
        }
    }
    let hit = rayQueryGetCommittedIntersection(&rq);
    return select(reach, hit.t, hit.kind != RAY_QUERY_INTERSECTION_NONE);
}

// The point light that lights `w` (normal `n`) the most, if it casts shadows: its index, or -1.
fn strongest_light(w: vec3<f32>, n: vec3<f32>) -> i32 {
    if (p.grid.z <= 0.0) {
        return -1;
    }
    let g = (w.xz - p.grid.xy) * p.grid.z;
    let dims = p.grid_dims.xy;
    if (any(g < vec2<f32>(0.0)) || any(g >= vec2<f32>(dims))) {
        return -1;
    }
    let c = vec2<u32>(g);
    let packed = light_grid[c.y * dims.x + c.x];
    let first = packed & 0xFFFFu;
    var best = -1;
    var most = 0.0;
    var lit_by_unshadowed = 0.0;
    for (var i = first; i < first + (packed >> 16u); i++) {
        let index = light_grid[i];
        let l = point_lights[index];
        let to = l.pos_radius.xyz - w;
        let x2 = dot(to, to) * l.color.w;
        if (x2 >= 1.0) {
            continue;
        }
        let window = saturate(1.0 - x2);
        let reach = window * window / (1.0 + 6.0 * x2) * saturate(dot(n, normalize(to)) * 0.65 + 0.35);
        let amount = dot(l.color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)) * reach;
        if (l.pos_radius.w < 0.0) {
            lit_by_unshadowed = max(lit_by_unshadowed, amount);
        } else if (amount > most) {
            most = amount;
            best = i32(index);
        }
    }
    // (Where a light that casts no shadow outshines it, no shadow at all.)
    return select(best, -1, lit_by_unshadowed > most);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

// Ground height at world `xz` (the ground map, see terrain_shadow.rs).
fn ground_height(xz: vec2<f32>) -> f32 {
    return textureSampleLevel(ground_map, linear_sampler, xz * p.ground.xy, 0.0).r * 255.0;
}

// The ground's normal at world `xz`, from the heights a texel of the ground map round it.
fn ground_normal(xz: vec2<f32>) -> vec3<f32> {
    let e = 8.0;
    let dx = ground_height(xz + vec2<f32>(e, 0.0)) - ground_height(xz - vec2<f32>(e, 0.0));
    let dz = ground_height(xz + vec2<f32>(0.0, e)) - ground_height(xz - vec2<f32>(0.0, e));
    return normalize(vec3<f32>(-dx, 2.0 * e, -dz));
}

// The colour of what a mirror ray from `origin` along `dir` meets: its paint (the texture, in
// the player's colour, or the face's colour), lit by the sun and the sky; a = 0 when it meets
// nothing within reach.
fn model_position(vertex: u32) -> vec3<f32> {
    return vec3<f32>(bitcast<f32>(model_vertices[vertex]), bitcast<f32>(model_vertices[vertex + 1u]), bitcast<f32>(model_vertices[vertex + 2u]));
}

fn reflected_normal(n: vec3<f32>, e1: vec3<f32>, e2: vec3<f32>, uv1: vec2<f32>, uv2: vec2<f32>, texel: vec3<f32>) -> vec3<f32> {
    let det = uv1.x * uv2.y - uv1.y * uv2.x;
    if (abs(det) < 1e-10) { return n; }
    let raw_t = (e1 * uv2.y - e2 * uv1.y) / det;
    let raw_b = (e2 * uv1.x - e1 * uv2.x) / det;
    let projected_t = raw_t - n * dot(n, raw_t);
    if (dot(projected_t, projected_t) < 1e-12) { return n; }
    let t = normalize(projected_t);
    let b = cross(n, t) * select(-1.0, 1.0, dot(cross(n, t), raw_b) >= 0.0);
    let mapped = texel * 2.0 - 1.0;
    return normalize(t * mapped.x + b * mapped.y + n * mapped.z);
}

// Keep the unwrapped arithmetic identical to the raster vertex shader. Wrap after
// interpolation; a stationary face retains its original edge coordinates.
fn tread_uv(uv: vec2<f32>, tread: vec2<f32>, travel: vec2<f32>) -> vec2<f32> {
    if (tread.x == 0.0) { return uv; }
    return vec2<f32>(fract(uv.x + tread.x * (travel.x - tread.y * travel.y)), uv.y);
}

// Both authored paint and legacy team frames use the raster shader's palettes.
// flags bit 0 selects accessible colours; bit 1 selects art style independently.
fn reflected_team_colour(team: u32, flags: u32) -> vec3<f32> {
    let colours = array<vec3<f32>, 10>(
        vec3<f32>(48.0, 92.0, 220.0), vec3<f32>(210.0, 40.0, 30.0),
        vec3<f32>(230.0), vec3<f32>(40.0, 170.0, 60.0),
        vec3<f32>(20.0, 40.0, 140.0), vec3<f32>(130.0, 70.0, 180.0),
        vec3<f32>(240.0, 220.0, 40.0), vec3<f32>(40.0),
        vec3<f32>(120.0, 190.0, 240.0), vec3<f32>(180.0, 170.0, 120.0)
    );
    let accessible = array<vec3<f32>, 10>(
        vec3<f32>(0.0, 114.0, 178.0), vec3<f32>(230.0, 159.0, 0.0),
        vec3<f32>(240.0), vec3<f32>(0.0, 158.0, 115.0),
        vec3<f32>(86.0, 180.0, 233.0), vec3<f32>(204.0, 121.0, 167.0),
        vec3<f32>(240.0, 228.0, 66.0), vec3<f32>(35.0),
        vec3<f32>(213.0, 94.0, 0.0), vec3<f32>(150.0)
    );
    return srgb_to_linear(select(colours[min(team, 9u)], accessible[min(team, 9u)], (flags & 1u) != 0u) / 255.0);
}

fn reflected_team_layer(layer_word: u32, team: u32, flags: u32) -> u32 {
    let layer = layer_word & 0xFFFFu;
    if (layer != 0xFFFFu && ((layer_word >> 16u) & 0xFFu) == 10u) {
        // Accessible paint comes from the white player's frame, just as in vs_main.
        return layer + select(min(team, 9u), 2u, (flags & 1u) != 0u);
    }
    return layer;
}

fn reflected_texture_layer(layer_word: u32, team: u32, flags: u32, time: f32, plays: bool) -> u32 {
    let layer = reflected_team_layer(layer_word, team, flags);
    let frames = (layer_word >> 16u) & 0xFFu;
    if (layer != 0xFFFFu && frames > 1u && frames != 10u && plays) {
        // Match vs_main exactly: the ten-frame palette is stationary team paint;
        // every other multi-frame GAF advances from the same 8 fps frame clock where the
        // piece's script lets it play, and holds its first frame elsewhere.
        return layer + u32(time * 8.0) % frames;
    }
    return layer;
}

// Whether a face a livery paints shows TA's own paint under it (Heritage's), as unit.wgsl draws it.
fn reflected_original_paint(layer_word: u32) -> bool {
    return (layer_word & (1u << 30u)) != 0u && livery_original(liveries.banks[select(0u, 1u, (layer_word & (1u << 31u)) != 0u)]);
}

fn reflected_team_tint(albedo: vec3<f32>, layer_word: u32, team: u32, flags: u32) -> vec3<f32> {
    if ((layer_word & 0xFFFFu) != 0xFFFFu && ((layer_word >> 16u) & 0xFFu) == 10u && (flags & 1u) != 0u) {
        // The white frame's brightest shade is approximately 0.79 in linear space.
        return albedo / 0.79 * reflected_team_colour(team, flags);
    }
    return albedo;
}

fn reflected(origin: vec3<f32>, dir: vec3<f32>) -> vec4<f32> {
    var rq: ray_query;
    rayQueryInitialize(&rq, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, REFLECTION_REACH, origin, dir));
    loop {
        if (!rayQueryProceed(&rq)) {
            break;
        }
    }
    let hit = rayQueryGetCommittedIntersection(&rq);
    if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
        return vec4<f32>(0.0);
    }
    // (The custom data is the piece's entry in the table: all of its 24 bits.)
    let piece = traced_pieces[hit.instance_custom_data];
    let instance = traced_instances[hit.instance_index];
    let team = instance.team;
    let tri = piece.first_index + hit.primitive_index * 3u;
    let v0 = (model_indices[tri] + piece.first_vertex) * VERTEX_WORDS;
    let v1 = (model_indices[tri + 1u] + piece.first_vertex) * VERTEX_WORDS;
    let v2 = (model_indices[tri + 2u] + piece.first_vertex) * VERTEX_WORDS;
    let b = vec3<f32>(1.0 - hit.barycentrics.x - hit.barycentrics.y, hit.barycentrics.x, hit.barycentrics.y);
    let local_normal = unpack4x8snorm(model_vertices[v0 + 3u]).xyz * b.x
        + unpack4x8snorm(model_vertices[v1 + 3u]).xyz * b.y
        + unpack4x8snorm(model_vertices[v2 + 3u]).xyz * b.z;
    var normal = normalize((hit.object_to_world * vec4<f32>(local_normal, 0.0)).xyz);
    let layer_word = model_vertices[v0 + 5u];
    var material_factors = vec2<f32>(unpack4x8snorm(model_vertices[v0 + 3u]).w, unpack4x8unorm(model_vertices[v0 + 6u]).a);
    var texture_ao = 1.0;
    var albedo = srgb_to_linear(unpack4x8unorm(model_vertices[v0 + 6u]).rgb) * b.x
        + srgb_to_linear(unpack4x8unorm(model_vertices[v1 + 6u]).rgb) * b.y
        + srgb_to_linear(unpack4x8unorm(model_vertices[v2 + 6u]).rgb) * b.z;
    if ((layer_word & (1u << 26u)) != 0u) {
        albedo *= reflected_team_colour(team, p.grid_dims.w);
    }
    let layer = reflected_texture_layer(layer_word, team, p.grid_dims.w, p.eye.w, (instance.state & 1u) != 0u);
    if (layer != 0xFFFFu) {
        let uv0 = unpack2x16unorm(model_vertices[v0 + 4u]);
        let uv1 = unpack2x16unorm(model_vertices[v1 + 4u]);
        let uv2 = unpack2x16unorm(model_vertices[v2 + 4u]);
        let uv = uv0 * b.x + uv1 * b.y + uv2 * b.z;
        let tread = vec2<f32>(bitcast<f32>(model_vertices[v0 + 11u]), bitcast<f32>(model_vertices[v0 + 12u]));
        let sampled_uv = tread_uv(uv, tread, instance.travel);
        if ((layer_word & (1u << 29u)) != 0u) {
            if ((layer_word & (1u << 31u)) != 0u) {
                albedo = textureSampleLevel(enemy_skin_textures, linear_sampler, sampled_uv, layer, max(1.0, log2(f32(textureDimensions(enemy_skin_textures).x) / 128.0) + 1.0)).rgb;
            } else {
                albedo = textureSampleLevel(skin_textures, linear_sampler, sampled_uv, layer, max(1.0, log2(f32(textureDimensions(skin_textures).x) / 128.0) + 1.0)).rgb;
            }
        } else if ((p.grid_dims.w & 2u) != 0u && !reflected_original_paint(layer_word)) {
            let colour = reborn_paint(sampled_uv, layer, 1.0);
            let finish = reborn_finish(sampled_uv, layer, 1.0);
            albedo = colour.rgb;
            texture_ao = colour.a;
            material_factors = finish.ba;
            let e1 = (hit.object_to_world * vec4<f32>(model_position(v1) - model_position(v0), 0.0)).xyz;
            let e2 = (hit.object_to_world * vec4<f32>(model_position(v2) - model_position(v0), 0.0)).xyz;
            let xy = clamp((finish.rg * 255.0 - 128.0) / 127.0, vec2<f32>(-1.0), vec2<f32>(1.0));
            let nz = sqrt(max(1.0 - dot(xy, xy), 0.001));
            let normal_texel = normalize(vec3<f32>(xy, nz)) * 0.5 + 0.5;
            normal = reflected_normal(normal, e1, e2, uv1 - uv0, uv2 - uv0, normal_texel);
        } else {
            albedo = model_paint(sampled_uv, layer, 1.0).rgb;
        }
        albedo = reflected_team_tint(albedo, layer_word, team, p.grid_dims.w);
    }
    // A skin's livery, as the raster draws it (unit.wgsl) but for the edges' trim, wear and dirt:
    // a reflection is too small and soft to show them.
    if ((layer_word & (1u << 30u)) != 0u) {
        let l = liveries.banks[select(0u, 1u, (layer_word & (1u << 31u)) != 0u)];
        let piece_number = f32(model_vertices[v0 + 7u] & 0xFFFFu);
        let local = model_position(v0) * b.x + model_position(v1) * b.y + model_position(v2) * b.z
            + vec3<f32>(31.4 + piece_number * 13.7, 0.0, 17.2 + piece_number * 7.3);
        var mean = albedo;
        if (layer != 0xFFFFu) {
            if ((p.grid_dims.w & 2u) != 0u && !reflected_original_paint(layer_word)) {
                mean = reborn_paint(vec2<f32>(0.5), layer, f32(textureNumLevels(reborn_textures) - 1u)).rgb;
            } else {
                mean = model_paint(vec2<f32>(0.5), layer, f32(textureNumLevels(model_textures) - 1u)).rgb;
            }
        }
        albedo = livery_paint(l, albedo, mean, local, local_normal, 0.6, 0.0);
        material_factors = livery_material(l, material_factors);
    }
    // (Faces are lit on the side the ray meets.)
    let n = select(-normal, normal, dot(normal, dir) < 0.0);
    var ao = texture_ao;
    var emission = vec3<f32>(0.0);
    if ((layer_word & (1u << 24u)) != 0u) {
        // (A light shines while its unit works, as vs_main's flags have it.)
        let luminance = dot(albedo, vec3<f32>(0.2126, 0.7152, 0.0722));
        let lit = 1.0 - f32((instance.state >> 8u) & 0xFFu) / 255.0;
        emission += albedo * smoothstep(0.08, 0.45, luminance) * select(0.9, 1.25, (p.grid_dims.w & 2u) != 0u) * lit;
    }
    let light = p.ambient.rgb * (0.75 + 0.25 * n.y) * ao + p.sun_color.rgb * max(dot(n, p.light.xyz), 0.0);
    var shaded = albedo * light;
    if ((layer_word & (1u << 28u)) != 0u || ((p.grid_dims.w & 2u) != 0u && layer != 0xFFFFu)) {
        let metal = material_factors.y;
        let rough = clamp(material_factors.x, 0.08, 1.0);
        let ndl = max(dot(n, p.light.xyz), 0.0);
        let v = -dir;
        let h = normalize(p.light.xyz + v);
        let nv = max(dot(n, v), 1e-3);
        let nh = max(dot(n, h), 0.0);
        let vh = max(dot(v, h), 0.0);
        let a = rough * rough;
        let a2 = a * a;
        let dd = nh * nh * (a2 - 1.0) + 1.0;
        let ggx = a2 / (3.14159265 * dd * dd);
        let k = a * 0.5;
        let visibility = 0.25 / ((nv * (1.0 - k) + k) * (ndl * (1.0 - k) + k));
        let f0 = mix(vec3<f32>(0.04), albedo, metal);
        let fresnel = f0 + (1.0 - f0) * pow(1.0 - vh, 5.0);
        let physical = albedo * (1.0 - 0.72 * metal) * light + p.sun_color.rgb * min(ggx * visibility, 60.0) * fresnel * ndl;
        shaded = mix(shaded, physical, select(0.3, 1.0, (p.grid_dims.w & 2u) != 0u));
    }
    return vec4<f32>(shaded + emission, 1.0);
}

// Traces a pixel: r = how much of the light's disc it sees, g = how open the hemisphere
// above it is (1 = nothing near), b = whether the point light that lights it most reaches it;
// and what its mirror ray meets (models, and wet ground while it rains).
@compute @workgroup_size(8, 8)
fn trace(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let pix = vec2<i32>(id.xy);
    let z = textureLoad(depth_map, pix, 0);
    if (z >= 1.0) {
        textureStore(raw, pix, vec4<f32>(1.0, 1.0, 0.0, 1.0));
        textureStore(reflections, pix, vec4<f32>(0.0));
        return;
    }
    let w = position(pix, z);
    let n = normal_at(pix, w);
    // Off the surface by a little more than the depth buffer's error there.
    let origin = w + n * (0.15 + distance(w, p.eye.xyz) * 0.0006);
    let fp = vec2<f32>(pix);
    var lit = 0.0;
    for (var k = 0; k < SHADOW_RAYS; k++) {
        let dir = towards_disc(p.light.xyz, p.light.w, ign(fp, f32(2 * k)), ign(fp, f32(2 * k + 1)));
        if (!blocked(origin, dir, SHADOW_REACH)) {
            lit += 1.0;
        }
    }
    var open = 0.0;
    let reach = p.depth.z;
    for (var k = 0; k < AO_RAYS; k++) {
        let dir = over_hemisphere(n, ign(fp, f32(2 * k + 11)), ign(fp, f32(2 * k + 12)));
        // Nearer things hide more of the sky.
        open += nearest(origin, dir, reach) / reach;
    }
    let ao = mix(1.0, open / f32(AO_RAYS), p.depth.w);
    // Fires, explosions, muzzle flashes and lasers throw shadows too: towards the brightest of
    // them here, a point in a small ball around it (soft shadows); what lies within a little
    // of the light itself (the wreck a fire burns on, the barrel a flash leaves) does not count.
    var reached = 1.0;
    let index = strongest_light(w, n);
    if (index >= 0) {
        let l = point_lights[index];
        let ball = over_hemisphere(normalize(w - l.pos_radius.xyz), ign(fp, 21.0), ign(fp, 22.0)) * 3.0;
        let to = l.pos_radius.xyz + ball - origin;
        let d = length(to);
        reached = select(1.0, 0.0, blocked(origin, to / max(d, 1e-3), d - min(6.0, d * 0.3)));
    }
    textureStore(raw, pix, vec4<f32>(lit / f32(SHADOW_RAYS), ao, reached, 1.0));
    // Reflections: models (their metal) always, the ground while it is wet.
    let on_model = w.y - ground_height(w.xz) > 1.0;
    var mirror = vec4<f32>(0.0);
    if ((on_model && p.ground.w > 0.5) || (!on_model && p.ground.z > 0.01)) {
        let v = normalize(p.eye.xyz - w);
        // (Wet ground mirrors about the ground's own normal: the one from the depth around is
        // stepped where the eye grazes the ground, which tore what it mirrored into slices.)
        let m = select(n, ground_normal(w.xz), !on_model);
        mirror = reflected(origin, reflect(-v, m));
    }
    textureStore(reflections, pix, mirror);
}

// ---- Blur ---------------------------------------------------------------------------------
// Two passes (across, then down) of a Gaussian that only mixes pixels at about the same depth,
// narrow for the shadows (their edges stay crisp), wide for the ambient occlusion.

const BLUR_RADIUS: i32 = 6;

fn blur(pix: vec2<i32>, step: vec2<i32>) -> vec3<f32> {
    let z0 = linear_depth(load_depth(pix));
    let tolerance = 0.4 + z0 * 0.004;
    var sum = vec3<f32>(0.0);
    var weight = vec3<f32>(0.0);
    for (var i = -BLUR_RADIUS; i <= BLUR_RADIUS; i++) {
        let q = pix + step * i;
        if (any(q < vec2<i32>(0)) || any(q >= vec2<i32>(p.size.xy))) {
            continue;
        }
        let zq = load_depth(q);
        if (zq >= 1.0) {
            continue;
        }
        let same = exp(-abs(linear_depth(zq) - z0) / tolerance);
        let x = f32(i);
        let narrow = exp(-x * x / 4.5);
        let g = vec3<f32>(narrow, exp(-x * x / 18.0), narrow) * same;
        sum += textureLoad(blur_src, q, 0).rgb * g;
        weight += g;
    }
    return sum / max(weight, vec3<f32>(1e-4));
}

@compute @workgroup_size(8, 8)
fn blur_across(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let pix = vec2<i32>(id.xy);
    if (load_depth(pix) >= 1.0) {
        textureStore(blur_mid, pix, vec4<f32>(1.0, 1.0, 1.0, 1.0));
        return;
    }
    textureStore(blur_mid, pix, vec4<f32>(blur(pix, vec2<i32>(1, 0)), 1.0));
}

// The result: r = the sun's (or moon's) light, g = ambient occlusion, b = the brightest point
// light's, a = the pixel's depth (the scene's shaders use the result only where their own
// fragment lies at that depth).
@compute @workgroup_size(8, 8)
fn blur_down(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= p.size.x || id.y >= p.size.y) {
        return;
    }
    let pix = vec2<i32>(id.xy);
    let z = load_depth(pix);
    if (z >= 1.0) {
        textureStore(blur_out, pix, vec4<f32>(1.0, 1.0, 1.0, z));
        return;
    }
    textureStore(blur_out, pix, vec4<f32>(blur(pix, vec2<i32>(0, 1)), z));
}
