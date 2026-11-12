// Shared declarations prepended to every shader module.

struct Frame {
    view_proj: mat4x4<f32>,
    shadow_view_proj: mat4x4<f32>,
    // xyz = eye position, w = 1 for perspective, 0 for classic projection
    camera_pos: vec4<f32>,
    // xyz = unit direction towards the sun, w = sun intensity
    sun_dir: vec4<f32>,
    // rgb = sun colour, a = terrain relight strength
    sun_color: vec4<f32>,
    // rgb = ambient colour, a = unit ambient occlusion strength
    ambient: vec4<f32>,
    // x = time (s), y = sea level, z = map width (px), w = map height (px)
    world: vec4<f32>,
    // x = viewport width, y = viewport height, z = zoom (px per unit), w = shadow strength
    screen: vec4<f32>,
    // rgb = fog colour, a = fog density
    fog: vec4<f32>,
    // World-space vectors for one sprite pixel to the right / up on screen (billboards).
    billboard_right: vec4<f32>,
    billboard_up: vec4<f32>,
    // rgb = colour scattered by deep water (linear), a = absorption per unit of view path
    water_color: vec4<f32>,
    // x = wave strength, y = foam strength, z = max water body opacity, w = 1 when water is drawn
    water: vec4<f32>,
    // x = sea bed caustics / refraction strength, y = 1 on lava worlds, z = 1 when the
    // reflection pass drew the world above the water this frame (see reflection.rs), w = the
    // reflectance added where the water mirrors it (WaterSettings::reflectivity)
    water_extra: vec4<f32>,
    // Painted shoreline reference colours measured at map load (linear):
    // rgb = average painted water, a = 1 / |water - land|²
    shore_water: vec4<f32>,
    // rgb = average painted beach next to the water, a = 1 when the two are distinct enough
    // to locate the painted shoreline (else foam falls back to the heightmap)
    shore_land: vec4<f32>,
    // Point light grid: xy = world xz of the grid origin, z = 1 / cell size, w = unused
    light_grid_origin: vec4<f32>,
    // x, y = grid cells per axis, z = light count, w = unused
    light_grid_dims: vec4<u32>,
    // Fog of war: xy = 1 / world extent of the visibility texture, z = 1 when active;
    // w = distance from the eye where distance haze begins (just beyond the view focus)
    los: vec4<f32>,
    // Sky and the world beyond the map (see sky.rs); only meaningful when `sky.w` is 1.
    // rgb = sky colour straight up (linear), a = cloud cover (0..1)
    sky_zenith: vec4<f32>,
    // x = sun disc angular radius (radians), y = cloud drift speed (world units / s),
    // z = camera far plane distance (everything has faded into the horizon haze there),
    // w = 1 when the sky and the ground beyond the map are drawn (perspective camera only)
    sky: vec4<f32>,
    // x = how far the extended ground reaches beyond the map edges (world units),
    // y = mean fog-of-war brightness over the map (what the ground beyond it fades to),
    // zw = the map's offset of the cloud field (world units)
    ground_beyond: vec4<f32>,
    // The map's painted sea bed (rgb, linear; a = 1 when measured) and beach next to it (rgb;
    // a = 1 when distinct): the extended ground takes these colours below and at sea level.
    beyond_sea: vec4<f32>,
    beyond_shore: vec4<f32>,
    // Inverse of view_proj (the sky pass reconstructs view rays with it).
    inv_view_proj: mat4x4<f32>,
    // Ambient occlusion (perspective camera only, see occlusion.rs): x = strength of the
    // terrain's baked occlusion, y = 1 when the contact occlusion map is drawn, z = strength of
    // the darkening of models and trees near the ground; w = art style (1 Reborn, 0 Enhanced Classic)
    ao: vec4<f32>,
    // The contact occlusion map: xy = world xz of its minimum corner, z = 1 / its world size,
    // w = unused
    ao_map: vec4<f32>,
    // Cloud shadows (perspective camera with the sky, see clouds.rs): x = strength (0 = off),
    // y = scale of the map's compressed mapping, zw = its centre (world xz, plane y = 0)
    clouds: vec4<f32>,
    // x = heat haze strength (perspective camera only, see heat.rs; 0 = off), y = height over
    // which smoke fades out where it meets the ground (soft smoke; 0 = off), z = how strongly
    // a lava world's melt glows on the rock standing over it from afar (`melt_glow`; 0 = not a
    // lava world), w = unused
    heat: vec4<f32>,
    // Light on units from the sky and the ground (perspective camera, 0 = the classic hemisphere
    // ambient): rgb = light bounced off the painted ground, a = how much the sky and ground
    // colours replace the plain ambient colour
    ground_bounce: vec4<f32>,
    // Surface detail quality (both cameras): x = the ground's smooth
    // colour, grain and relief up close, y = models' metal reflections and glowing lights;
    // z = the map overlay on the ground (0 none, 1 the overlay texture's colours, 2 heights),
    // w = 1 for colour-blind-safe player colours and marks (see unit.wgsl)
    detail: vec4<f32>,
    // The time of day (perspective camera, see daylight.rs): rgb = what the painted ground's
    // sunlit term is multiplied by (1 = TA's own light, as painted), a = the warm glow along
    // the horizon on the sun's side at dawn and dusk (0..1)
    daylight: vec4<f32>,
    // rgb = light the sky adds to the painted ground whatever the sun does (night, twilight),
    // a = how dark it is (0..1): the stars show and the disc in the sky is the moon
    night: vec4<f32>,
    // xyz = direction towards the sun itself (with a time of day it may be below the horizon
    // while the moon lights the world), w = how airless the world is (0..1, perspective camera
    // with the sky: a black sky with stars by day, deep shade; see sky::WorldSky)
    celestial: vec4<f32>,
    // The relief's shadows (perspective camera, see terrain_shadow.rs): x = strength (0 = off),
    // yz = 1 / the world extent of the clearance map, w = unused
    terrain_shadow: vec4<f32>,
    // The wind: xy = world xz, its length 0 (calm) to about 1.5 (a gale); z = how much trees
    // sway in it (0 = not at all: the classic view), w = unused
    wind: vec4<f32>,
    // Rain (perspective camera): x = how wet the world is (0..1), y = how hard it rains now,
    // z = how much the ground moves in the wind (0..1, see `ground_life` in terrain.wgsl),
    // w = unused
    rain: vec4<f32>,
    // Soft shadows (perspective camera): x = penumbra width per unit of caster height (0 = the
    // plain 3x3 filter), y = how far to look for casters (texels), z = world units per unit of
    // shadow depth, w = world units per shadow texel
    shadow_soft: vec4<f32>,
    // Ray tracing (see raytrace.rs, `rt_visibility`): x = 1 when this frame's result was traced,
    // y, z = the near and far planes (depth to distance), w = unused
    rt: vec4<f32>,
    // The liquid below sea level (see water.rs): x = 1 for acid (0 for water), y = 1 in space
    // (perspective camera with the sky: the map's void cells drawn as nothing, the sky all
    // round; see sky::WorldSky::space), z = 1 in a battle (perspective camera with the sky, not
    // in space: nothing beyond the map, the sky pass's sea of dark clouds under it; see
    // sky::Beyond::Clouds), w = unused
    liquid: vec4<f32>,
    // The world's colour grade (WorldGrade in terrain_detail.rs, see `world_grade`): x the gain,
    // y the contrast, z the saturation, w the key after the gain; the shade's tint (w how
    // strongly the tints show); the light's tint (w 1 when there is a grade).
    world_grade: array<vec4<f32>, 3>,
    // How far the wind has carried what drifts on it (see `advance_wind_drift` in lib.rs): xy =
    // world xz, in units of the wind's strength times seconds; zw = unused
    drift: vec4<f32>,
};

// One dynamic point light (see lights.rs).
struct PointLightGpu {
    // xyz = position, w = radius
    pos_radius: vec4<f32>,
    // rgb = linear HDR colour, w = 1 / radius²
    color: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var shadow_map: texture_depth_2d;
@group(0) @binding(2) var shadow_sampler: sampler_comparison;
@group(0) @binding(3) var<storage, read> point_lights: array<PointLightGpu>;
// Per grid cell: first index | count << 16 into this same array, then the light index lists.
@group(0) @binding(4) var<storage, read> light_grid: array<u32>;
// The viewer's sight over the map, a texel to a vision cell (see `fog_state`): r = in sight, g =
// explored, b = in radar or sonar range, each 0..1, eased over time by the game (fog.rs).
@group(0) @binding(5) var los_tex: texture_2d<f32>;
// Linear, clamped: also samples the maps below.
@group(0) @binding(6) var los_sampler: sampler;
// Contact occlusion of the ground around the view (1 = open ground; see `contact_occlusion`).
@group(0) @binding(7) var occlusion_map: texture_2d<f32>;
// Cloud layer coverage along the sun over the ground (see `cloud_light`).
@group(0) @binding(8) var cloud_map: texture_2d<f32>;
// The ground (see terrain_shadow.rs): r = height / 255, g = how far above the ground the light
// clears the relief / 255 (see `terrain_sunlight`); texels `frame.terrain_shadow.yz` apart
// (1x1 black before a map is loaded).
@group(0) @binding(9) var ground_map: texture_2d<f32>;

// Ground height (bilinear) at world xz; clamped at the map edges.
fn ground_height(xz: vec2<f32>) -> f32 {
    return textureSampleLevel(ground_map, los_sampler, xz * frame.terrain_shadow.yz, 0.0).r * 255.0;
}

// The reflection pass (see reflection.rs): whether the water can show the camera the point `p`
// mirrored, i.e. `p` is above the water and the ray reflected from it towards the eye meets the
// sea where there is water, not ground (in this pass `camera_pos` is the eye mirrored below the
// water, so the ray runs from `p` towards it). Everything else could only be seen through the
// ground: skipping it saves the shading and keeps the ground from showing through near coasts
// when the waves shift the lookup. Beyond the map, where coasts are drawn per pixel, the sea is
// assumed.
fn seen_in_water(p: vec3<f32>) -> bool {
    let sea = frame.world.y;
    if (p.y < sea) {
        return false;
    }
    let eye = frame.camera_pos.xyz;
    let meets = mix(p.xz, eye.xz, (p.y - sea) / max(p.y - eye.y, 1e-3));
    if (any(meets < vec2<f32>(0.0)) || any(meets > frame.world.zw)) {
        return true;
    }
    // A little ground is let through: the waterline between heightmap vertices is not exact.
    return ground_height(meets) < sea + 1.5;
}

// Contact occlusion at world xz: how much of the sky the units, buildings, wrecks and trees
// standing around leave open (1 = all of it). 1 when the map is off or outside it (fading out
// towards its border).
fn contact_occlusion(xz: vec2<f32>) -> f32 {
    if (frame.ao.y <= 0.0) {
        return 1.0;
    }
    let uv = (xz - frame.ao_map.xy) * frame.ao_map.z;
    let border = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
    if (border <= 0.0) {
        return 1.0;
    }
    let v = textureSampleLevel(occlusion_map, los_sampler, uv, 0.0).r;
    return mix(1.0, v, smoothstep(0.0, 0.02, border));
}

// Occlusion of a model or tree surface at `world` from what stands on the ground around it
// (its own footprint included): full near the ground, gone `reach` units above it. `above` =
// the surface's height above the ground.
fn near_ground_occlusion(world: vec3<f32>, above: f32, reach: f32) -> f32 {
    if (frame.ao.z <= 0.0 || frame.ao.y <= 0.0) {
        return 1.0;
    }
    let near = 1.0 - smoothstep(0.0, reach, above);
    if (near <= 0.0) {
        return 1.0;
    }
    return mix(1.0, contact_occlusion(world.xz), near * frame.ao.z);
}

// Direct sunlight left by the clouds' shadows at `world` (1 = none in the way). The map is
// indexed by where the sun ray through the point crosses the plane y = 0.
fn cloud_light(world: vec3<f32>) -> f32 {
    if (frame.clouds.x <= 0.0) {
        return 1.0;
    }
    let sun = frame.sun_dir.xyz;
    let g = world.xz - sun.xz / max(sun.y, 0.1) * world.y;
    let d = g - frame.clouds.zw;
    let uv = 0.5 + 0.5 * d / (abs(d) + frame.clouds.y);
    return 1.0 - frame.clouds.x * textureSampleLevel(cloud_map, los_sampler, uv, 0.0).r;
}

// ---- Fog of war ----
//
// The viewer's sight, drawn in three states: in sight (as lit), explored (seen once: greyed and
// dimmed) and never seen (dark), and where a radar or sonar reaches a faint scan over what is out
// of sight. The game eases the states over time (a place coming into sight lights up over a
// third of a second, one going out of it dims over a second and a half) and spreads each over the
// cells round it, so an edge between two is a slope a cell or two wide; here it is shaped into
// a soft edge and pushed about by a slow noise that drifts with the wind (more where a cell is
// between its states, a place wholly in or out of sight stays where it is), so the edges read
// as mist rather than as a polygon of cells.

// How bright a place is by TA's own reckoning: in sight 1, explored a half, never seen a fifth.
const FOG_EXPLORED: f32 = 0.5;
const FOG_UNEXPLORED: f32 = 0.2;

fn fog_texel(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(los_tex, los_sampler, uv, 0.0).rgb;
}

// The brightness TA's fog gives a state (what beams, particles and the classic view use).
fn fog_brightness(s: vec3<f32>) -> f32 {
    return mix(mix(FOG_UNEXPLORED, FOG_EXPLORED, max(s.y, s.x)), 1.0, s.x);
}

// The state beyond the map's edge: what the mean brightness of the map comes to, as a state.
fn fog_mean_state() -> vec3<f32> {
    let m = frame.ground_beyond.y;
    return vec3<f32>(saturate((m - FOG_EXPLORED) / (1.0 - FOG_EXPLORED)), saturate((m - FOG_UNEXPLORED) / (FOG_EXPLORED - FOG_UNEXPLORED)), 0.0);
}

// The state at a world position (x = in sight, y = explored, z = in radar range, each 0..1),
// beyond the map (with the world drawn there, see `beyond_on`) fading within a few hundred units
// into the map's mean instead of stretching out to the horizon. `soft`: with the soft edges (a
// sample or two more), or the plain sample (what only needs a brightness).
fn fog_state(world_pos: vec3<f32>, soft: bool) -> vec3<f32> {
    let uv = world_pos.xz * frame.los.xy;
    var s = fog_texel(uv);
    if (soft) {
        let between = 4.0 * max(max(s.x * (1.0 - s.x), s.y * (1.0 - s.y)), s.z * (1.0 - s.z));
        if (between > 0.02) {
            let q = world_pos.xz / 170.0 - frame.drift.xy * 0.03;
            let push = vec2<f32>(gust_noise(q) - 0.5, gust_noise(q + vec2<f32>(7.3, 3.9)) - 0.5) * 44.0 * between;
            s = fog_texel(uv + push * frame.los.xy);
        }
        s = smoothstep(vec3<f32>(0.06), vec3<f32>(0.94), s);
    }
    if (frame.sky.w > 0.5) {
        let outside = distance(uv, clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0))) / max(max(frame.los.x, frame.los.y), 1e-9);
        s = mix(s, fog_mean_state(), smoothstep(0.0, 320.0, outside));
    }
    return s;
}

// Fog of war brightness at `world_pos`: 1 in the viewer's line of sight, dimmer outside it.
fn los_light(world_pos: vec3<f32>) -> f32 {
    if (frame.los.z <= 0.0) {
        return 1.0;
    }
    return fog_brightness(fog_state(world_pos, false));
}

// Light (sun or moon) that gets past the relief to `world_pos`: 0 in the shadow of a ridge, 1 in
// the open (perspective camera; see terrain_shadow.rs). A point is in the shadow when it stands
// lower above the ground than the height at which the light clears the hills there. (Points on
// the ground count as on it: the mesh and the smooth heightmap differ by a unit or two.)
//
// Beyond the map (the extended ground, a sea past the edge) the clearance map has no texels, but
// the light's path from the point runs back over the map where the light comes from that side:
// where it crosses the map's edge, at the height it has risen to there, the clearance says
// whether a ridge stands in its way (the path beyond that point is the edge point's own). So a
// long low-sun shadow runs on past the edge as the ridge casts it, instead of stopping dead
// along the map's outline. (`MIN_TAN` in terrain_shadow.rs bounds the rise as the march does.)
fn terrain_sunlight(world_pos: vec3<f32>) -> f32 {
    let strength = frame.terrain_shadow.x;
    if (strength <= 0.0) {
        return 1.0;
    }
    var uv = world_pos.xz * frame.terrain_shadow.yz;
    var height = world_pos.y;
    var outside = any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0));
    if (outside) {
        let light = frame.sun_dir.xyz;
        let across = length(light.xz);
        if (across < 1e-3) {
            return 1.0;
        }
        let dir = light.xz / across;
        let extent = 1.0 / frame.terrain_shadow.yz;
        // Where the path towards the light enters the map rectangle (a ray through a box).
        let inv = 1.0 / select(dir, vec2<f32>(1e-6), abs(dir) < vec2<f32>(1e-6));
        let t0 = -world_pos.xz * inv;
        let t1 = (extent - world_pos.xz) * inv;
        let enter = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), 0.0);
        let leave = min(max(t0.x, t1.x), max(t0.y, t1.y));
        if (leave <= enter) {
            return 1.0;
        }
        // (A few units in: the edge texels are the map's own.)
        let t = min(enter + 4.0, leave);
        uv = clamp((world_pos.xz + dir * t) * frame.terrain_shadow.yz, vec2<f32>(0.0), vec2<f32>(1.0));
        height += t * max(light.y / across, 0.03);
    }
    let g = textureSampleLevel(ground_map, los_sampler, uv, 0.0);
    var above = height - g.r * 255.0;
    // (Beyond the map, a path that reaches the edge below the ground there is in its shadow.)
    if (!outside) {
        above = max(above, 0.0);
    }
    let clear = g.g * 255.0;
    return mix(1.0, smoothstep(-2.5, -0.5, above - clear), strength);
}

// Lava worlds (perspective camera): the glow of the melt below what stands near it (units,
// trees and rocks, painted features, smoke), as the ground by the shore glows (`lava_glow`): as
// much as the sea lies around (the ground map's alpha, see `lava_nearness` in
// terrain_shadow.rs), fading with the height above the melt. Light from below: a face looking
// down or across catches the most (`lava_on`), and metal mirrors it.
fn lava_light(world: vec3<f32>) -> vec3<f32> {
    if (frame.water_extra.y < 0.5 || frame.camera_pos.w < 0.5 || frame.terrain_shadow.y <= 0.0) {
        return vec3<f32>(0.0);
    }
    let near = textureSampleLevel(ground_map, los_sampler, world.xz * frame.terrain_shadow.yz, 0.0).a;
    if (near <= 0.004) {
        return vec3<f32>(0.0);
    }
    let above = max(world.y - frame.world.y, 0.0) / 96.0;
    let pulse = 0.92 + 0.08 * sin(frame.world.x * 1.7);
    return vec3<f32>(1.25, 0.3, 0.045) * (near * sqrt(near) * pulse / (1.0 + above * above));
}

// The melt's light (`lava_light`) on a surface facing `n`: most looking down at it, a little
// looking up (what the smoke and haze scatter back).
fn lava_on(lava: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return lava * mix(0.2, 1.0, saturate(0.5 - 0.5 * n.y));
}

// Lava worlds (perspective camera): the melt's glow on rock standing over it from afar (a basalt
// spire's shade, which the sky alone left a black cut-out): the melt across the map lights the
// ground and the pall of smoke over it, which light the rock warm from below and round the
// horizon, not from above, the most near the ground (`up`: how high above its foot; a tall
// spire's top sees more sky) (`frame.heat.z`: how much melt the map has); and more on the side
// facing the melt nearby (the sea's nearness on a ring round `world`), the less the higher
// above it.
fn melt_glow(world: vec3<f32>, n: vec3<f32>, up: f32) -> vec3<f32> {
    if (frame.heat.z <= 0.0 || frame.camera_pos.w < 0.5 || frame.terrain_shadow.y <= 0.0) {
        return vec3<f32>(0.0);
    }
    var toward = vec2<f32>(0.0);
    for (var k = 0; k < 6; k++) {
        let a = f32(k) * 1.0472;
        let d = vec2<f32>(cos(a), sin(a));
        toward += d * textureSampleLevel(ground_map, los_sampler, (world.xz + d * 160.0) * frame.terrain_shadow.yz, 0.0).a;
    }
    let near = length(toward) / 3.0;
    // (By night the pall's glow is the less: the melt nearby keeps its own.)
    let low = mix(0.45, 1.0, 1.0 / (1.0 + up * up / 3600.0)) * mix(0.5, 1.0, saturate(frame.sun_dir.w));
    let below = saturate(0.55 - 0.45 * n.y) * low;
    let facing = saturate(dot(n.xz, toward / max(length(toward), 1e-4)) * 0.7 + 0.3) * saturate(0.8 - 0.6 * n.y);
    let above = max(world.y - frame.world.y, 0.0) / 160.0;
    return vec3<f32>(1.0, 0.42, 0.16) * frame.heat.z * (below + 1.5 * near * facing / (1.0 + above * above));
}

// The light shade keeps, as a share of the direct light (the sky's fill): 55% grey in the
// classic view, the light TA's art was painted for; under the perspective camera's sky a little
// less and in the sky's own colour, so sunlit ground and shade stand apart as they do outdoors.
fn shade_fill() -> vec3<f32> {
    if (!beyond_on()) {
        return vec3<f32>(0.55);
    }
    let sky = frame.sky_zenith.rgb;
    let l = dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722));
    // (Without air, little: the shade is the ground's own bounce.)
    let fill = 0.46 * (1.0 - 0.5 * frame.celestial.w);
    if (l <= 1e-4) {
        return vec3<f32>(fill);
    }
    return fill * mix(vec3<f32>(1.0), clamp(sky / l, vec3<f32>(0.55), vec3<f32>(1.7)), 0.4);
}

// Ambient light on a surface facing `n` from the sky above and the painted ground below
// (perspective camera, `Lighting::sky_light`): the sky's own colours at the brightness of the
// plain ambient, and the light the map's ground bounces up, its brightness drawn towards that
// of the plain ground term `plain_ground` (by a factor kept within 0.6..1.4), so units on dark
// and bright worlds stay readable while taking the ground's hue.
fn sky_and_ground_light(n: vec3<f32>, plain_ground: vec3<f32>) -> vec3<f32> {
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    var sky = frame.ambient.rgb;
    if (frame.sky.w > 0.5) {
        let dome = mix(frame.fog.rgb, frame.sky_zenith.rgb, 0.5);
        sky = dome * (dot(frame.ambient.rgb, lum) / max(dot(dome, lum), 1e-3));
    }
    let bounce = frame.ground_bounce.rgb;
    let ground = bounce * clamp(dot(plain_ground, lum) / max(dot(bounce, lum), 1e-3), 0.6, 1.4);
    return mix(ground, sky, n.y * 0.5 + 0.5);
}

// Gusts: patches of stronger wind about this many units across, carried along by the wind this
// many times as fast as its strength (`frame.drift`): about 200 units a second in a strong one.
const GUST_SIZE: f32 = 420.0;
const GUST_CARRY: f32 = 200.0;

// Smooth value noise, 0..1, over a lattice of whole numbers (the gusts'; the other shaders' noise
// is noise.wgsl's).
fn gust_noise(p: vec2<f32>) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    // (The four corners' `gust_hash`es in one pass, lane by lane: the same bits in a quarter of
    // the code, which every call inlines; see noise.wgsl's `noise_d`.)
    let x = bitcast<vec4<u32>>(vec4<i32>(i.x, i.x + 1, i.x, i.x + 1));
    let y = bitcast<vec4<u32>>(vec4<i32>(i.y, i.y, i.y + 1, i.y + 1));
    var h = (x * 73856093u) ^ (y * 19349663u);
    h = (h ^ (h >> vec4<u32>(16u))) * 0x45d9f3bu;
    h = h ^ (h >> vec4<u32>(16u));
    let v = vec4<f32>(h & vec4<u32>(0xffffu)) / 65535.0;
    return mix(mix(v.x, v.y, u.x), mix(v.z, v.w, u.x), u.y);
}

fn gust_hash(c: vec2<i32>) -> f32 {
    var h = (bitcast<u32>(c.x) * 73856093u) ^ (bitcast<u32>(c.y) * 19349663u);
    h = (h ^ (h >> 16u)) * 0x45d9f3bu;
    h = h ^ (h >> 16u);
    return f32(h & 0xffffu) / 65535.0;
}

// How far the wind moves a point of a tree `top` units tall rooted at `root`, `up` units above
// the root: the crown most, the foot not at all. It leans downwind as far as the wind is
// strong, gusts sweep through the forest downwind, and each tree flutters on its own.
fn wind_bend(root: vec3<f32>, top: f32, up: f32) -> vec3<f32> {
    let h = clamp(up / top, 0.0, 1.2);
    let w = frame.wind.xy;
    let strength = length(w);
    let dir = w / max(strength, 1e-4);
    let t = frame.world.x;
    let phase = fract(sin(dot(root.xz, vec2<f32>(12.9898, 78.233))) * 43758.5453) * 6.2832;
    // (Carried by how far the wind has blown, not laid out along its heading: bands along it
    // swung through a forest far from the map's corner as fast as the wind turned, and every
    // tree leaned anew each frame.)
    let gust = smoothstep(0.2, 0.8, gust_noise((root.xz - frame.drift.xy * GUST_CARRY) / GUST_SIZE));
    let flutter = sin(t * (1.9 + phase * 0.08) + phase) * 0.6 + sin(t * 4.7 + phase * 2.3) * 0.25;
    let lean = strength * (0.35 + 0.75 * gust);
    let shake = flutter * (0.2 + 0.5 * strength);
    let across = vec2<f32>(-dir.y, dir.x) * sin(t * 1.3 + phase * 1.7) * 0.25 * shake;
    let d = (dir * (lean + shake) + across) * top * 0.06 * h * h;
    return vec3<f32>(d.x, -0.3 * dot(d, d) / top, d.y);
}

// A 4x4 ordered dither at a pixel (`position` as the fragment sees it): 0..1, each step once.
// Where a painted sprite hands over to its 3D model in the water's reflection (one sample a
// pixel), the model keeps the pixels whose dither is under how far it has taken over and the
// sprite the rest, so between them every pixel is drawn once.
fn handover_dither(px: vec2<f32>) -> f32 {
    let bayer = array<f32, 16>(0.0, 8.0, 2.0, 10.0, 12.0, 4.0, 14.0, 6.0, 3.0, 11.0, 1.0, 9.0, 15.0, 7.0, 13.0, 5.0);
    let i = (u32(px.x) & 3u) + (u32(px.y) & 3u) * 4u;
    return (bayer[i] + 0.5) / 16.0;
}

// MSAA samples of the scene pass, for the pipelines whose fragments pick their samples
// (`handover_mask`, and the water reading the scene: set per pipeline, see foliage.rs,
// sprites.rs, model.rs, water.rs; 1 elsewhere).
override SCENE_SAMPLES: u32 = 1u;

// A scene fragment and the samples of its pixel it covers.
struct Covered {
    @location(0) color: vec4<f32>,
    @builtin(sample_mask) mask: u32,
};

// The samples (bits, of `n`) a fragment drawn `share` (0..1) of the way takes: that share of them,
// the fraction of a sample dithered over 4 x 4 pixels (`handover_dither`) and the run of them
// turned pixel by pixel. What one drawn `share` takes and the complement (all but those) are
// disjoint and cover every sample: a 3D tree taking over from its painting takes these, the
// painting the rest, so the pixel is the two blended in proportion, never partly empty (alpha to
// coverage gives a lower alpha a subset of a higher one's samples, so two fading together left
// holes). Multisampled, it resolves to a smooth blend; with one sample, an ordered dither.
fn handover_mask(px: vec2<f32>, share: f32, n: u32) -> u32 {
    let all = (1u << n) - 1u;
    let k = min(u32(floor(saturate(share) * f32(n) + handover_dither(px))), n);
    let first = (1u << k) - 1u;
    let r = (u32(px.x) * 3u + u32(px.y) * 5u) % n;
    return ((first << r) | (first >> (n - r))) & all;
}

// Points of a disc (radius 1) for the soft shadows' samples: sixteen on a golden-angle spiral,
// spread evenly over it.
const SHADOW_DISC = array<vec2<f32>, 16>(
    vec2<f32>(0.177, 0.000), vec2<f32>(-0.226, 0.207), vec2<f32>(0.035, -0.394), vec2<f32>(0.285, 0.371),
    vec2<f32>(-0.522, -0.092), vec2<f32>(0.495, -0.315), vec2<f32>(-0.165, 0.616), vec2<f32>(-0.316, -0.608),
    vec2<f32>(0.685, 0.250), vec2<f32>(-0.712, 0.294), vec2<f32>(0.343, -0.734), vec2<f32>(0.254, 0.809),
    vec2<f32>(-0.765, -0.443), vec2<f32>(0.897, -0.197), vec2<f32>(-0.548, 0.779), vec2<f32>(-0.126, -0.976),
);

// Contact-hardening soft shadow at shadow map `uv`, depth `depth` (1 = lit): the casters
// around are found and averaged, and the filter widens as far as they stand above the
// point, as the sun's disc blurs a shadow in the world (and a ray-traced one on screen). The
// search (every other point of the disc) reaches as far as the widest filter. Where it finds
// no caster, or nothing but, the point's own sample is the answer; where it finds few, the
// result leans on that sample: a thin caster slipping between the search's samples keeps its
// shadow, its penumbra fading, rather than switching it all on and off (it was taken for lit).
fn soft_shadow(uv: vec2<f32>, depth: f32) -> f32 {
    let dims = vec2<f32>(textureDimensions(shadow_map));
    let texel = 1.0 / dims;
    let compare = depth - shadow_bias();
    let here = textureSampleCompareLevel(shadow_map, shadow_sampler, uv, compare);
    var blockers = 0.0;
    var count = 0.0;
    for (var i = 0; i < 16; i += 2) {
        let c = clamp(vec2<i32>((uv + SHADOW_DISC[i] * frame.shadow_soft.y * texel) * dims), vec2<i32>(0), vec2<i32>(dims) - 1);
        let d = textureLoad(shadow_map, c, 0);
        if (d < compare) {
            blockers += d;
            count += 1.0;
        }
    }
    if (count == 0.0 || count == 8.0) {
        return here;
    }
    let above = (depth - blockers / count) * frame.shadow_soft.z;
    let radius = clamp(above * frame.shadow_soft.x / max(frame.shadow_soft.w, 1e-3), 1.0, frame.shadow_soft.y);
    var lit = 0.0;
    for (var i = 0; i < 16; i++) {
        lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + SHADOW_DISC[i] * radius * texel, compare);
    }
    return mix(here, lit / 16.0, saturate(count / 2.0));
}

// Shadow of models (soft or 3x3 PCF over the shadow map) and of the relief. Returns 1 when lit,
// 0 when fully shadowed.
fn shadow_factor(world_pos: vec3<f32>, normal: vec3<f32>) -> f32 {
    let relief = terrain_sunlight(world_pos);
    if (frame.screen.w <= 0.0) {
        return relief;
    }
    let biased = world_pos + normal * 1.5;
    let clip = frame.shadow_view_proj * vec4<f32>(biased, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    if (any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) || ndc.z > 1.0) {
        return relief;
    }
    // Faded out over the map's outer tenth (the view is fitted inside the rest, see `shadow_fit`
    // in lib.rs): where the map ends, beyond the ground near the camera, no line.
    let strength = frame.screen.w * shadow_edge_keep(ndc.xy);
    if (frame.shadow_soft.x > 0.0) {
        return min(mix(1.0, soft_shadow(uv, ndc.z), strength), relief);
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + vec2<f32>(f32(x), f32(y)) * texel, ndc.z - shadow_bias());
        }
    }
    return min(mix(1.0, sum / 9.0, strength), relief);
}

// How far (shadow map depth, 0..1) a surface stands behind the caster it is compared with before
// it counts as shadowed: about ten world units, however deep the map reaches this frame
// (`frame.shadow_soft.z`, its depth in world units; never more than at the least depth).
fn shadow_bias() -> f32 {
    return 9.6 / max(frame.shadow_soft.z, 6400.0);
}

// How much of a shadow is kept at shadow map position `ndc` (-1..1): all of it but over the
// map's outer tenth, where it fades out (`SHADOW_FADE_FROM` in lib.rs).
fn shadow_edge_keep(ndc: vec2<f32>) -> f32 {
    return 1.0 - smoothstep(0.9, 1.0, max(abs(ndc.x), abs(ndc.y)));
}

// Unit vector from a world position towards the viewer. The classic projection is
// orthographic along -(0, 2, 1), so its view direction is the same everywhere.
fn view_dir(world_pos: vec3<f32>) -> vec3<f32> {
    if (frame.camera_pos.w < 0.5) {
        return vec3<f32>(0.0, 0.894427, 0.447214);
    }
    return normalize(frame.camera_pos.xyz - world_pos);
}

// True when the sky and the ground beyond the map are drawn (perspective camera, sky enabled).
// Every change they bring to the other passes is behind this flag, so the classic projection
// renders exactly as it always did.
fn beyond_on() -> bool {
    return frame.sky.w > 0.5;
}

// The sun's glow in the haze and the sky around it (forward scattering): a halo a few degrees
// wide, an aureole and a faint broad brightening. Kept well below white so the HDR sun disc
// still stands out (bloom adds the glare around it). `dir` = unit view direction (from the eye).
fn sun_glow(dir: vec3<f32>) -> vec3<f32> {
    let s = saturate(dot(dir, frame.sun_dir.xyz));
    let glow = pow(s, 300.0) * 0.45 + pow(s, 24.0) * 0.12 + pow(s, 4.0) * 0.04;
    return frame.sun_color.rgb * frame.sun_dir.w * glow;
}

// Dawn and dusk (see daylight.rs): a warm band low over the horizon, brightest on the sun's
// side and reaching round a good part of the sky, still glowing a while after sunset.
fn dusk_glow(dir: vec3<f32>) -> vec3<f32> {
    let d = frame.daylight.a;
    if (d <= 0.0) {
        return vec3<f32>(0.0);
    }
    let sun_flat = frame.celestial.xz / max(length(frame.celestial.xz), 1e-4);
    let flat = dir.xz / max(length(dir.xz), 1e-4);
    let side = 0.5 + 0.5 * dot(flat, sun_flat);
    let low = exp(-max(dir.y, 0.0) * 6.0);
    return vec3<f32>(1.0, 0.42, 0.18) * (d * low * (0.08 + 0.9 * side * side * side));
}

// Light scattered towards the eye by the haze along the view direction `dir`: the fog colour
// plus the sun's glow (and at dawn and dusk the glowing horizon). The sky is exactly this
// colour at the horizon (and below it), and distance fog tends towards it, so far ground
// dissolves seamlessly into the sky.
fn haze_color(dir: vec3<f32>) -> vec3<f32> {
    return frame.fog.rgb + sun_glow(dir) + dusk_glow(dir);
}

// Fraction of fog in front of a world position (perspective camera only).
fn fog_factor(world_pos: vec3<f32>) -> f32 {
    if (frame.camera_pos.w < 0.5) {
        return 0.0;
    }
    // Haze only what lies beyond the point being looked at, so zooming out doesn't wash
    // out the whole battlefield.
    let d = distance(world_pos, frame.camera_pos.xyz);
    var f = 0.0;
    if (frame.fog.a > 0.0) {
        f = clamp(1.0 - exp(-max(d - frame.los.w, 0.0) * frame.fog.a), 0.0, 0.85);
    }
    if (beyond_on()) {
        // Everything has dissolved into the horizon haze before the far plane clips it; the
        // sky pass fills the clipped background with that same haze.
        f = max(f, smoothstep(frame.sky.z * 0.45, frame.sky.z * 0.97, d));
    }
    return f;
}

// Colour that distance fog tends towards at a world position: the plain fog colour, or with
// the sky drawn, the haze seen in that direction (including the sun's glow).
fn fog_color_at(world_pos: vec3<f32>) -> vec3<f32> {
    if (beyond_on()) {
        return haze_color(normalize(world_pos - frame.camera_pos.xyz));
    }
    return frame.fog.rgb;
}

// Fog of war on a lit colour at `world_pos` (see `fog_state`). The classic view darkens as TA
// did (with the soft edges and the fades). With the perspective camera, what lies out of sight
// loses more of its colour than of its light: explored ground greyer and a little dimmer (a cool
// grey, as the moon would leave it), ground never seen dim and cooler, both under a slow mist
// that drifts with the wind; and where a radar or sonar reaches, the shroud lifts a little (no
// tint: what is out of sight is never coloured), a faint scan moves over it and a thin pale line
// marks the edge of its range. What is in sight is the plain lit map, whatever the fog does to the
// rest of the world, which stays readable.
fn fog_of_war(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    if (frame.los.z <= 0.0) {
        return color;
    }
    let s = fog_state(world_pos, true);
    if (s.x >= 0.999) {
        return color;
    }
    if (frame.camera_pos.w < 0.5) {
        return color * fog_brightness(s);
    }
    let known = max(s.y, s.x);
    let radar = s.z;
    // The mist: broad patches and finer ones drifting at a few units a second.
    let mist = gust_noise(world_pos.xz / 260.0 - frame.drift.xy * 0.05) * 0.65
        + gust_noise(world_pos.xz / 83.0 + frame.drift.xy * 0.08 + vec2<f32>(11.7, 2.3)) * 0.35 - 0.5;
    let lum = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    // Out of sight the colours drain towards a cool grey: more of them where never seen.
    let drained = mix(color, vec3<f32>(lum) * vec3<f32>(0.92, 0.98, 1.1), mix(0.9, 0.72, known));
    var shrouded = drained * mix(0.27, 0.6, known) * (1.0 + mist * mix(0.34, 0.14, known));
    // Radar: the shroud lifts a little, without a tint, a scan climbs it (a line a few units
    // wide every twenty, moving slowly) and a thin pale line marks where its reach ends.
    let sweep = 0.5 + 0.5 * sin(world_pos.z * 0.31 + world_pos.x * 0.06 - frame.world.x * 2.0);
    let rim = 1.0 - smoothstep(0.0, 0.2, abs(radar - 0.5));
    shrouded = shrouded * mix(1.0, 1.1 + 0.06 * sweep, radar * 0.5);
    shrouded += vec3<f32>(0.5) * rim * 0.05 * (0.4 + lum);
    return mix(shrouded, color, s.x);
}

// Fog of war, then distance haze.
fn apply_fog(color: vec3<f32>, world_pos: vec3<f32>) -> vec3<f32> {
    return mix(fog_of_war(color, world_pos), fog_color_at(world_pos), fog_factor(world_pos));
}

// ---- Ray tracing (see raytrace.rs; `rt_visibility` comes from rt_on.wgsl or rt_off.wgsl) ----

struct RtLight {
    // The fragment is the surface the result was traced for.
    traced: bool,
    // How much of the sun (or moon) reaches it, how open the sky above it is, and whether the
    // point light that lights it most reaches it.
    sun: f32,
    ao: f32,
    point: f32,
};

// ---- The world's colour grade ---------------------------------------------------------------

// The world's colour grade over the ground's colour `c` (Reborn; `WorldGrade` in
// terrain_detail.rs): its value drawn towards the world's key and spread about the map's in
// stops (the spread easing off a couple of stops out, and a lift easing in under a roof, so the
// art's near-white markings are not pushed into glare), dull colours made richer (not by
// moonlight, which fades them), and the shade and the light tinted apart.
fn world_grade(c: vec3<f32>) -> vec3<f32> {
    let lum = vec3<f32>(0.2126, 0.7152, 0.0722);
    let g = frame.world_grade[0];
    let l = max(dot(c, lum), 1e-6);
    let stops = log2(l * g.x / g.w);
    let spread = stops + (g.y - 1.0) * 2.0 * tanh(stops * 0.5);
    let graded = g.w * exp2(spread);
    var v = graded;
    if (graded > l) {
        let room = max(0.8 - l, 0.0);
        v = l + room * (1.0 - exp(-(graded - l) / max(room, 1e-4)));
    }
    var o = c * (v / l);
    let mx = max(max(o.r, o.g), o.b);
    let sat = (mx - min(min(o.r, o.g), o.b)) / max(mx, 1e-5);
    o = max(mix(vec3<f32>(dot(o, lum)), o, 1.0 + (g.z - 1.0) * (1.0 - sat) * (1.0 - frame.night.a)), vec3<f32>(0.0));
    let tint = mix(frame.world_grade[1].rgb, frame.world_grade[2].rgb, smoothstep(-2.0, 2.0, stops));
    return o * mix(vec3<f32>(1.0), tint, frame.world_grade[1].w);
}

// A colour TA painted on the map's world (its ground, the features standing on it and their 3D
// stand-ins) in the world's colour grade, as the art style shows it (Reborn; as painted in
// Enhanced Classic, and on a world with no grade or with `TA_WORLD_GRADE=0`): all of it in the
// same light.
fn world_graded(c: vec3<f32>) -> vec3<f32> {
    let reborn = saturate(frame.ao.w);
    if (reborn <= 0.0 || frame.world_grade[2].w <= 0.5) {
        return c;
    }
    return mix(c, world_grade(c), reborn);
}

// ---- Ore ------------------------------------------------------------------------------------

// Metal crystals in the rock of a deposit `ore` rich (0..1, none where it is 0) at world `xz`:
// in some of the cells a few units across, a crystal whose facet, tilted at random (and swaying
// a little over time), flashes when it mirrors the light `l` into the eye `v`. A glint is a
// couple of pixels across at the least (`footprint`: world units per pixel), the same light
// spread wider and dimmer, not a single pixel clipped to white that pops as the eye moves; it
// is taken from its own cell and the three nearest round the point, so none is cut off at a
// cell's edge. Too small to see from further away: a faint, even glitter then. Returns the
// flash's strength (times the sun's light).
fn ore_glints(xz: vec2<f32>, v: vec3<f32>, l: vec3<f32>, footprint: f32, t: f32, ore: f32) -> f32 {
    let cell = 3.0;
    let p = xz / cell;
    let half = normalize(l + v);
    // From far away: the mean of the flashes, a faint glitter.
    let even = pow(saturate(half.y), 6.0) * 0.12 * ore;
    let fine = 1.0 - smoothstep(0.27, 0.4, footprint / cell);
    if (fine <= 0.0) {
        return even;
    }
    let r = max(0.12, 1.75 * footprint / cell);
    let side = select(vec2<i32>(1), vec2<i32>(-1), fract(p) < vec2<f32>(0.5));
    let base = vec2<i32>(floor(p));
    var flashes = 0.0;
    for (var k = 0; k < 4; k++) {
        let c = base + vec2<i32>(k & 1, k >> 1u) * side;
        var h = bitcast<u32>(c.x) * 73856093u ^ bitcast<u32>(c.y) * 19349663u ^ 0x9e3779b9u;
        h = (h ^ (h >> 16u)) * 0x7feb352du;
        h = (h ^ (h >> 15u)) * 0x846ca68bu;
        h = h ^ (h >> 16u);
        let r1 = f32(h & 255u) / 255.0;
        let r2 = f32((h >> 8u) & 255u) / 255.0;
        if (fract(r1 * 7.13 + r2 * 3.7) > ore * 0.5) {
            continue;
        }
        let r3 = f32((h >> 16u) & 255u) / 255.0;
        let r4 = f32((h >> 24u) & 255u) / 255.0;
        let d = length(p - (vec2<f32>(c) + 0.3 + vec2<f32>(r1, r2) * 0.4)) / r;
        let q = saturate(1.0 - d * d);
        let spot = q * q * (0.12 / r) * (0.12 / r);
        let tilt = vec2<f32>(r3, r4) * 2.0 - 1.0 + 0.25 * vec2<f32>(sin(t * 0.7 + r1 * 6.3), cos(t * 0.9 + r2 * 6.3));
        let facet = normalize(vec3<f32>(tilt.x * 0.6, 1.0, tilt.y * 0.6));
        flashes += spot * pow(saturate(dot(facet, half)), 50.0) * 10.0;
    }
    return mix(even, flashes, fine);
}

// ---- Dynamic point lights ------------------------------------------------------------------
//
// Lights are binned on the CPU into a coarse world-space (xz) grid covering only the union of
// the visible lights' footprints. A fragment looks up its cell and loops over the lights
// listed there; outside the grid it returns immediately.

// Range (first, count) of light indices in `light_grid` for a world position.
fn light_cell(world_pos: vec3<f32>) -> vec2<u32> {
    let dims = frame.light_grid_dims.xy;
    if (frame.light_grid_dims.z == 0u) {
        return vec2<u32>(0u, 0u);
    }
    let g = (world_pos.xz - frame.light_grid_origin.xy) * frame.light_grid_origin.z;
    if (any(g < vec2<f32>(0.0)) || any(g >= vec2<f32>(dims))) {
        return vec2<u32>(0u, 0u);
    }
    let c = vec2<u32>(g);
    let packed = light_grid[c.y * dims.x + c.x];
    return vec2<u32>(packed & 0xFFFFu, packed >> 16u);
}

// Smooth falloff: 1 at the light, a soft inverse-square-like core, exactly 0 at the radius
// with zero slope (no visible edge). `x2` = (distance / radius)².
fn light_falloff(x2: f32) -> f32 {
    let window = saturate(1.0 - x2);
    return window * window / (1.0 + 6.0 * x2);
}

struct PointLit {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
};

// Sums every point light reaching `world_pos`.
// * `normal` = zero vector: omnidirectional (billboards, smoke, foam): no N·L term.
//   Otherwise a wrapped N·L, so ground at the height of a low light still receives some.
// * `shininess` > 0 adds a Blinn-Phong highlight towards `view` (unit vector to the eye).
fn point_lights_eval(world_pos: vec3<f32>, normal: vec3<f32>, view: vec3<f32>, shininess: f32) -> PointLit {
    var out: PointLit;
    out.diffuse = vec3<f32>(0.0);
    out.specular = vec3<f32>(0.0);
    let range = light_cell(world_pos);
    let directional = dot(normal, normal) > 0.25;
    for (var i = range.x; i < range.x + range.y; i++) {
        let light = point_lights[light_grid[i]];
        let to_light = light.pos_radius.xyz - world_pos;
        let d2 = dot(to_light, to_light);
        let x2 = d2 * light.color.w;
        if (x2 >= 1.0) {
            continue;
        }
        let radiance = light.color.rgb * light_falloff(x2);
        let l = to_light * inverseSqrt(max(d2, 1e-4));
        var ndl = 1.0;
        if (directional) {
            ndl = saturate(dot(normal, l) * 0.65 + 0.35);
        }
        out.diffuse += radiance * ndl;
        if (shininess > 0.0) {
            let h = normalize(l + view);
            let facing = saturate(dot(normal, l) * 4.0);
            out.specular += radiance * pow(saturate(dot(normal, h)), shininess) * facing;
        }
    }
    return out;
}

// Diffuse irradiance from all point lights at a surface point. Multiply by the surface
// albedo and add to its lighting.
fn point_lighting(world_pos: vec3<f32>, normal: vec3<f32>) -> vec3<f32> {
    return point_lights_eval(world_pos, normal, vec3<f32>(0.0, 1.0, 0.0), 0.0).diffuse;
}

// Point light irradiance without a surface orientation (sprites, smoke, foam).
fn point_lighting_omni(world_pos: vec3<f32>) -> vec3<f32> {
    return point_lights_eval(world_pos, vec3<f32>(0.0), vec3<f32>(0.0, 1.0, 0.0), 0.0).diffuse;
}
