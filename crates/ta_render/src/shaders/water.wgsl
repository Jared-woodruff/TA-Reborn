// Water surface: a flat plane at sea level over the map chunks that have terrain below it,
// and (perspective camera with the sky on) over the extended ground around the map, so seas
// and lava fields continue to the horizon. Prepended: common.wgsl, terrain_common.wgsl
// (heightmap and art), noise.wgsl, water_common.wgsl, sky_common.wgsl.
//
// TA's painted map art already shows the sea bed and the water colour, so the surface is a
// translucent layer over it (premultiplied alpha; drawn after opaque geometry with the depth
// test on and depth writes off). Everything is computed per fragment from the world
// position, so chunk borders are seamless and both camera modes work alike:
//
// * depth of the water column from the heightmap (with the mesh's own triangulation, so the
//   surface is fully transparent exactly where it meets the ground),
// * procedural band-limited waves -> normal, plus the slope variance lost to filtering,
// * water body: light scattered inside the water, opacity from the refracted view path to
//   the sea bed (Beer-Lambert) capped by `water.z`, so shallow water shows the painted bed,
// * Fresnel reflection of a sky gradient derived from the fog (horizon) and ambient (zenith),
//   and (perspective camera) of the world above the water, looked up through the waves in the
//   planar reflection rendered from the mirrored camera (reflection.rs),
// * sun glitter (GGX; roughness widened by the filtered wave slopes and by the normal's
//   screen-space variation) and point light sparkle,
// * animated, translucent surf along the shore (shared with the terrain shader, see
//   water_common.wgsl).

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    // Extended ground only: the ground height below, interpolated exactly like the skirt
    // mesh it lies over (so the waterline matches the ground it meets).
    @location(1) ground: f32,
};

@vertex
fn vs_water(@builtin(vertex_index) vi: u32, @location(0) chunk: vec2<u32>) -> VsOut {
    // Two triangles per chunk: (0,0) (1,0) (1,1) / (0,0) (1,1) (0,1).
    let corners = array<vec2<u32>, 6>(
        vec2<u32>(0u, 0u), vec2<u32>(1u, 0u), vec2<u32>(1u, 1u),
        vec2<u32>(0u, 0u), vec2<u32>(1u, 1u), vec2<u32>(0u, 1u),
    );
    // Same vertex grid as the terrain mesh, clamped to the map edge.
    let cell = min((chunk + corners[vi % 6u]) * params.chunk_cells, params.cells);
    let world = vec3<f32>(f32(cell.x) * 16.0, frame.world.y, f32(cell.y) * 16.0);
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.ground = 0.0;
    return out;
}

// Sea level over the extended ground: the skirt mesh (sky.rs) flattened to the water plane.
@vertex
fn vs_water_outer(@location(0) xz: vec2<f32>) -> VsOut {
    let world = vec3<f32>(xz.x, frame.world.y, xz.y);
    var out: VsOut;
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.ground = skirt_vertex_height(xz);
    return out;
}

// Sky seen in a reflection. Classic projection: fog colour at the horizon, a deeper blue
// derived from the ambient colour at the zenith, and a soft glow around the sun, scaled with
// the sun. With the sky drawn: the same sky as the sky pass (without clouds), so the sea
// meets the sky at the horizon.
fn sky_radiance(r: vec3<f32>) -> vec3<f32> {
    if (beyond_on()) {
        return sky_gradient(r);
    }
    let sun_i = frame.sun_dir.w;
    let horizon = frame.fog.rgb * 1.1;
    let zenith = frame.ambient.rgb * vec3<f32>(0.62, 0.85, 1.35);
    var sky = mix(horizon, zenith, sqrt(saturate(r.y))) * sun_i;
    let s = saturate(dot(r, frame.sun_dir.xyz));
    sky += frame.sun_color.rgb * sun_i * (pow(s, 64.0) * 2.0 + pow(s, 8.0) * 0.25);
    return sky;
}

// ---- Reflections of the world above the water (see reflection.rs) --------------------------
// Drawn from the mirrored camera before the scene: premultiplied colour (alpha = where
// something was mirrored; elsewhere the sky is) with its mip chain, and beside it the reach:
// how far beyond the surface the mirrored world lies along the eye's rays, each level the
// nearest of the four below. Read only while `water_extra.z` is 1.
@group(2) @binding(0) var reflection_color: texture_2d<f32>;
@group(2) @binding(1) var reflection_reach: texture_2d<f32>;
@group(2) @binding(2) var reflection_sampler: sampler;

// Farthest (world units) behind the surface a reflection is looked for: where nothing was
// mirrored, the waves may not scatter far-off things across the sky's reflection.
const MIRROR_REACH: f32 = 400.0;
// Share of the wave slopes that bends reflections: TA's waves are steep so they read from high
// above, and mirrored at that slope everything would shatter.
const MIRROR_WAVES: f32 = 0.25;
// Screen pixels the reach is smoothed over, so where it changes (at every silhouette in the
// reflection) the look-up glides instead of jumping between unrelated texels.
const MIRROR_SMOOTH: f32 = 12.0;

// Reflection texture coordinates of the clip-space position `c`.
fn mirror_uv(c: vec4<f32>) -> vec2<f32> {
    return c.xy / c.w * vec2<f32>(0.5, -0.5) + 0.5;
}

// How far beyond the surface the mirrored world seen at `uv` lies (MIRROR_REACH where nothing
// is), smoothed (nearest wins) over the reach's level `level`.
fn mirror_reach(uv: vec2<f32>, level: f32) -> f32 {
    return textureSampleLevel(reflection_reach, reflection_sampler, uv, level).r;
}

// The mirror image (below the water) of what the reflection shows at `uv`, `reach` beyond the
// water along the eye's ray through it.
fn mirror_image(uv: vec2<f32>, reach: f32) -> vec3<f32> {
    let h = frame.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 1.0, 1.0);
    let eye = frame.camera_pos.xyz;
    let ray = normalize(h.xyz / h.w - eye);
    return eye + ray * ((frame.world.y - eye.y) / min(ray.y, -1e-4) + reach);
}

// The ray reflected towards the eye `v` (unit) by water of slope `bend`, mirrored below the
// surface (for flat water it continues the view ray), in clip space per unit of distance.
fn bent_ray(v: vec3<f32>, bend: vec2<f32>) -> vec4<f32> {
    let n = normalize(vec3<f32>(-bend.x * MIRROR_WAVES, 1.0, -bend.y * MIRROR_WAVES));
    let r = reflect(-v, n);
    return frame.view_proj * vec4<f32>(r.x, -r.y, r.z, 0.0);
}

// Where to look the reflection up for surface point `p`, seen along `v` (unit, to the eye)
// through waves of slope `bend`: xy = the reflection's coordinates, zw = how far they move when
// the waves too fine for the pixels tilt the surface by `spread` towards the eye (the reflection
// smears along that).
//
// The wave normal bends the reflected ray; what it shows lies where the bent ray meets the
// mirrored world. How far away that is comes from the reach: at this pixel, checked at the
// pixel that distance leads to (the nearer of the two wins, so the waves do not scatter things
// into the sky around them). So reflections stay attached where things meet the water and
// waver the more the higher they reach above it.
fn mirror_lookup(p: vec3<f32>, v: vec3<f32>, bend: vec2<f32>, spread: f32) -> vec4<f32> {
    let clip = frame.view_proj * vec4<f32>(p, 1.0);
    let ray = bent_ray(v, bend);
    // (The reach's texels are twice the reflection's, whose are 1 or 2 screen pixels.)
    let texels = f32(textureDimensions(reflection_reach).x) * 2.0 / frame.screen.x;
    let level = clamp(log2(MIRROR_SMOOTH * texels) - 1.0, 0.0, 4.0);
    let first = mirror_reach(mirror_uv(clip), level);
    let guess = mirror_uv(clip + ray * first);
    let second = distance(mirror_image(guess, mirror_reach(guess, level)), p);
    let t = min(first, second);
    let uv = mirror_uv(clip + ray * t);
    let toward = v.xz / max(length(v.xz), 1e-4);
    let axis = mirror_uv(clip + bent_ray(v, bend + toward * spread) * t) - uv;
    return vec4<f32>(uv, axis);
}

// The world above the water mirrored at reflection coordinates `uv`: premultiplied colour,
// a = coverage (0: only the sky there), filtered over the screen pixel's footprint in the
// reflection (`dx`, `dy`: the coordinates' change across it) widened along `axis` (see
// `mirror_lookup`) and to a texel and a bit (the reflection pass has no antialiasing). Fades
// out where the look-up leaves the screen.
fn mirrored_world(uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, axis: vec2<f32>) -> vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(reflection_color));
    let least = 1.25 * texel.x;
    var gx = dx;
    var gy = dy;
    // The spread goes to whichever of the two runs more along it.
    if (abs(dot(gx, axis)) > abs(dot(gy, axis))) {
        gx += axis * sign(dot(gx, axis) + 1e-9);
    } else {
        gy += axis * sign(dot(gy, axis) + 1e-9);
    }
    gx = select(gx * (least / max(length(gx), 1e-9)), gx, length(gx) >= least);
    gy = select(gy * (least / max(length(gy), 1e-9)), gy, length(gy) >= least);
    if (length(dx) < 1e-9) {
        gx = vec2<f32>(least, 0.0);
    }
    if (length(dy) < 1e-9) {
        gy = vec2<f32>(0.0, least);
    }
    let outside = max(max(-uv.x, uv.x - 1.0), max(-uv.y, uv.y - 1.0));
    let fade = 1.0 - smoothstep(0.0, 0.03, outside);
    return textureSampleGrad(reflection_color, reflection_sampler, uv, gx, gy) * fade;
}

// ---- The scene under the water (see `WaterScene` in water.rs) ------------------------------
// The opaque scene is finished before the water is drawn: its depth, every sample of it, and a
// copy of its colour resolved. So the water knows what lies how far under it at each pixel (the
// bed, a hull, a pontoon, a wading unit's legs), and shows it bent by the waves.
@group(3) @binding(0) var scene_depth: texture_depth_multisampled_2d;
@group(3) @binding(1) var scene_color: texture_2d<f32>;
// The wakes of ships and hovercraft (see wakes.rs; half the scene's size): rg = the slope of
// the waves their hulls raised, b = the slick their churn left, a = foam.
@group(3) @binding(2) var wake_map: texture_2d<f32>;

// Share of the waves' slope by which what lies under the water appears shifted per unit of its
// depth (about 1 - 1/1.33 of the tilt, exaggerated a little so it reads at RTS distances), and
// the most it shifts (world units).
const REFRACT: f32 = 0.35;
const REFRACT_MAX: f32 = 5.0;
// How far (world units) the collar of foam reaches out from where something stands in the water.
const COLLAR: f32 = 2.2;
// How high (world units) the surf breaking on a shore stands off the water, seen from low down.
const SURF_STAND: f32 = 1.2;

// The opaque scene's depth at pixel `px` under the water drawn there at depth `own`: at a
// silhouette the first sample may be what stands in front of the water, which is drawn over the
// others; the farthest of those is what lies under it.
fn scene_depth_at(px: vec2<i32>, own: f32) -> f32 {
    var z = textureLoad(scene_depth, px, 0);
    if (z < own) {
        for (var i = 1u; i < SCENE_SAMPLES; i++) {
            z = max(z, textureLoad(scene_depth, px, i));
        }
    }
    return z;
}

// The world position the scene shows at screen position `px` (pixels), depth `z`.
fn scene_point(px: vec2<f32>, z: f32) -> vec3<f32> {
    let ndc = vec2<f32>(px.x / frame.screen.x * 2.0 - 1.0, 1.0 - px.y / frame.screen.y * 2.0);
    let h = frame.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return h.xyz / h.w;
}

// How what lies `below` the surface at `under` (seen at pixel `px`) looks through waves of slope
// `slope`, as a change to the scene drawn there: shifted along the slope, the more the deeper it
// lies. Never to what stands above the water: a hull's side is not seen in the sea beside it.
fn refracted(px: vec2<f32>, under: vec3<f32>, below: f32, slope: vec2<f32>) -> vec3<f32> {
    let here = textureLoad(scene_color, vec2<i32>(px), 0).rgb;
    let shift = clamp(slope * (below * REFRACT), vec2<f32>(-REFRACT_MAX), vec2<f32>(REFRACT_MAX)) * (smoothstep(0.0, 10.0, below) * frame.water_extra.x);
    let c = frame.view_proj * vec4<f32>(under.x + shift.x, under.y, under.z + shift.y, 1.0);
    let uv = c.xy / c.w * vec2<f32>(0.5, -0.5) + 0.5;
    let q = clamp(vec2<i32>(uv * frame.screen.xy), vec2<i32>(0), vec2<i32>(frame.screen.xy) - 1);
    let there = scene_point(vec2<f32>(q) + 0.5, textureLoad(scene_depth, q, 0));
    if (there.y > frame.world.y - 0.05) {
        return vec3<f32>(0.0);
    }
    return textureSampleLevel(scene_color, linear_clamp, uv, 0.0).rgb - here;
}

// GGX normal distribution and height-correlated Smith visibility.
fn ggx_specular(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, alpha2: f32) -> f32 {
    let h = normalize(l + v);
    let nh = saturate(dot(n, h));
    let nl = saturate(dot(n, l));
    let nv = max(dot(n, v), 1e-3);
    let dd = nh * nh * (alpha2 - 1.0) + 1.0;
    let d = alpha2 / (3.14159265 * dd * dd);
    let gv = nl * sqrt(nv * nv * (1.0 - alpha2) + alpha2);
    let gl = nv * sqrt(nl * nl * (1.0 - alpha2) + alpha2);
    let vis = 0.5 / max(gv + gl, 1e-4);
    let f = 0.02 + 0.98 * pow(1.0 - saturate(dot(v, h)), 5.0);
    return d * vis * f * nl;
}

// Fades the surface out where the sea bed seen through it has no painted art (and at the
// map edges), matching the terrain's fade of the painted image. The bed point is found
// along the view ray `v`: in the classic projection its art coordinate equals the surface
// point's own, in perspective it is the bed right below (so the north border keeps water).
// No fade when the world continues beyond the map.
fn map_edge_fade(p: vec3<f32>, v: vec3<f32>, depth: f32) -> f32 {
    if (beyond_on()) {
        return 1.0;
    }
    let size = frame.world.zw;
    let d = min(min(p.x, p.z), min(size.x - p.x, size.y - p.z));
    let bed = p.xz - v.xz * (depth / max(v.y, 0.2));
    let bed_h = p.y - depth;
    return smoothstep(0.0, 40.0, d) * art_edge_fade(vec2<f32>(bed.x, bed.y - bed_h * 0.5));
}

// Lava worlds: crust, glowing cracks and hot spots over the painted lava, an incandescent
// rim along the painted shore (on the map and next to it: `far` < 1); no reflections or foam.
fn lava_fragment(p: vec3<f32>, depth: f32, dxz: vec2<f32>, dyz: vec2<f32>, t: f32, far: f32) -> vec4<f32> {
    let footprint = max(length(dxz), length(dyz));
    // (The distance from the bank: measured on the map, from the depth out in the far field.)
    var shore = depth * 5.0;
    if (far < 1.0) {
        shore = mix(shore_distance(p.xz), shore, far);
    }
    var rim = 0.0;
    // Where the melt is: a little in from the bank, and near the waterline where TA painted it
    // (`lava_melt`; the painted rock it covered shows, the terrain draws the melt the
    // heightmap left above the level); beyond the map, where the art is a reflection, the
    // heightmap's.
    var melt = smoothstep(0.0, 2.0, depth);
    if (frame.shore_land.a > 0.0 && far < 1.0) {
        let art_px = surface_art_px(p.xz, p.y);
        rim = lava_rim(art_px, dxz, dyz) * (1.0 - far);
        melt = mix(lava_melt(art_px, depth, footprint), melt, max(far, smoothstep(0.0, 16.0, beyond_distance(p.xz))));
    }
    var shadow = shadow_factor(p, vec3<f32>(0.0, 1.0, 0.0));
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(p);
    }
    let layer = lava_layer(p, shore, melt, rim, shadow, footprint, t);
    var rgb = layer.rgb;
    var a = layer.a;
    let fade = map_edge_fade(p, view_dir(p), depth);
    rgb *= fade;
    a *= fade;
    rgb = mix(fog_of_war(rgb, p), fog_color_at(p) * a, water_fog(p));
    return vec4<f32>(rgb, a);
}

// Distance fog on the water surface, including the extended ground's fade into the haze.
fn water_fog(p: vec3<f32>) -> f32 {
    if (!beyond_on()) {
        return fog_factor(p);
    }
    return beyond_fog(p, beyond_distance(p.xz));
}

// Water over the map: the depth of the water column exactly as the terrain mesh
// interpolates the ground (the surface is fully transparent where it meets the ground).
@fragment
fn fs_water(in: VsOut) -> @location(0) vec4<f32> {
    return water_fragment(in.world, frame.world.y - mesh_height(in.world.xz), 0.0, in.clip.xyz);
}

// Water over the extended ground beyond the map: near the map, the depth over the ground mesh
// below; further out, the per-pixel shoreline and depth of the filtered land/sea layout (the
// ground mesh lies just under the water along coasts there, see `skirt_height`).
@fragment
fn fs_water_outer(in: VsOut) -> @location(0) vec4<f32> {
    let d = beyond_distance(in.world.xz);
    let far = beyond_coast_mix(d);
    var depth = frame.world.y - in.ground;
    if (far > 0.0) {
        depth = mix(depth, beyond_water_depth(in.world.xz, d), far);
    }
    return water_fragment(in.world, depth, far, in.clip.xyz);
}

// Shades the water surface at `p` over a water column `depth` deep (<= 0: no water here).
// `far` (0..1) = how much the depth is the extended ground's per-pixel depth rather than the
// ground mesh's (the water column and the distance to the shore then follow it alone). `frag` =
// the fragment's pixel position and depth.
fn water_fragment(p: vec3<f32>, depth: f32, far: f32, frag: vec3<f32>) -> vec4<f32> {
    let t = frame.world.x;
    let sea = frame.world.y;
    // World units per pixel, for band-limiting the waves and foam (across and down the screen,
    // and the longer of the two).
    let dxz = dpdx(p.xz);
    let dyz = dpdy(p.xz);
    let footprint = max(length(dxz), length(dyz));

    // Lava worlds, and the horizontal distance to the waterline (on the map and next to it
    // measured at load, see `shore_distance`; out in the far field estimated from the depth).
    if (frame.water_extra.y > 0.5) {
        if (depth <= 0.0) {
            discard;
        }
        return lava_fragment(p, depth, dxz, dyz, t, far);
    }
    var shore_dist = max(depth, 0.0) * 5.0;
    var waterline = shore_dist;
    if (far < 1.0) {
        waterline = shore_distance(p.xz);
        shore_dist = mix(max(waterline, 0.0), shore_dist, far);
    }

    // ---- Waves -> normal --------------------------------------------------------------------
    let strength = frame.water.x;
    let waves = water_waves(p.xz, liquid_time(t), footprint);
    // The thin sheet of water running up the beach is calmer.
    let calm = mix(0.35, 1.0, smoothstep(0.0, 14.0, shore_dist));
    // What the ships and hovercraft that passed here left (see wakes.rs): foam, the waves their
    // hulls raised, and the slick in which their churn smoothed the wind's waves away (`still`);
    // none up the beach.
    let wake = textureSampleLevel(wake_map, linear_clamp, frag.xy / frame.screen.xy, 0.0) * smoothstep(0.5, 4.0, depth);
    let still = 1.0 - saturate(wake.z);
    // (Rain: rings spreading from the drops all over the water.)
    let swell_slope = waves.slope * (strength * calm * still) + wake.xy;
    let slope = swell_slope + rain_ripples(p.xz, t, frame.rain.y, footprint);
    let n = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    let v = view_dir(p);

    // ---- What lies under the water at this pixel (see `WaterScene`) ---------------------------
    let under = scene_point(frag.xy, scene_depth_at(vec2<i32>(frag.xy), frag.z));
    let below = max(sea - under.y, 0.0);
    // Whether it stands in the water above the bed (a hull, a pontoon, a wading unit's legs, a
    // rock), on the map, while it spans a few pixels.
    var object = 0.0;
    if (all(under.xz >= vec2<f32>(0.0)) && all(under.xz <= frame.world.zw)) {
        object = smoothstep(0.75, 1.5, under.y - mesh_height(under.xz)) * (1.0 - smoothstep(1.5, 3.0, footprint));
    }
    // How far from where it meets the surface (world units): its depth here over how fast that
    // grows across the surface. Beside a hull that is the view's slope down past its side; a
    // deck awash, as deep all over, is far from any; across the outline of what lies under the
    // water the depth jumps, and there is no edge. (Only near the surface: the sides of a unit
    // deep under the water would otherwise seem to rise to it.)
    let grow = max(abs(dpdx(below)) / max(length(dxz), 1e-4), abs(dpdy(below)) / max(length(dyz), 1e-4));
    let to_contact = select(1e9, below / max(grow, 0.05), grow < 12.0);
    // Specular anti-aliasing: widen the lobe by the normal's variation across the pixel
    // (derivatives taken here, before any non-uniform branch or discard).
    let dn = dpdx(n) * dpdx(n) + dpdy(n) * dpdy(n);
    let pixel_var = min(0.5 * (dn.x + dn.y + dn.z), 0.1);
    // Where the reflection is looked up (see `mirror_lookup`), and how fast that moves across
    // the pixel (taken here too). The waves bend it; the ripples too fine for the pixels and the
    // rain's rings (each ring far finer than the waves) only smear it.
    var mirror = vec4<f32>(0.0);
    var mirror_dx = vec2<f32>(0.0);
    var mirror_dy = vec2<f32>(0.0);
    if (frame.water_extra.z > 0.0) {
        let spread = sqrt(waves.lost_var * strength * strength * calm * calm * still * still + 0.008 * frame.rain.y) * 0.7;
        mirror = mirror_lookup(p, v, clamp(swell_slope, vec2<f32>(-1.4), vec2<f32>(1.4)), spread);
        mirror_dx = dpdx(mirror.xy);
        mirror_dy = dpdy(mirror.xy);
    }
    if (depth <= 0.0) {
        discard;
    }
    // Base roughness (ripples finer than the capillary octave; where that is filtered out it is
    // in the lost variance, so the base far off stays as it was) + slope variance filtered out
    // of the waves + normal variation within the pixel + the rain's rings too fine to show.
    // (A wake's slick is glassy: the ripples are smoothed out of it too.)
    let alpha2 = ((0.0012 - CAPILLARY_VAR) + waves.lost_var * strength * strength * still * still) * mix(1.0, 0.45, saturate(wake.z)) + pixel_var + rain_roughness(frame.rain.y, footprint);

    let sun_dir = frame.sun_dir.xyz;
    let sun_rad = frame.sun_color.rgb * frame.sun_dir.w;
    // (Far beyond the map only the relief's long shadows reach the sea.)
    var shadow = terrain_sunlight(p);
    if (far < 1.0) {
        shadow = shadow_factor(p, vec3<f32>(0.0, 1.0, 0.0));
    }
    // The clouds' shadows reach the horizon.
    if (frame.clouds.x > 0.0) {
        shadow *= cloud_light(p);
    }
    let lights = point_lights_eval(p, n, v, 180.0);

    // ---- Water body ---------------------------------------------------------------------------
    // Follow the refracted view ray down to the sea bed (damped fixed-point iteration on the
    // smoothed heightmap) to get the length of the light path through the water.
    let rd = refract(-v, vec3<f32>(0.0, 1.0, 0.0), 0.75);
    let down = max(-rd.y, 0.3);
    var travel = depth / down;
    if (far < 0.5) {
        for (var k = 0; k < 3; k++) {
            let bed = ground_smooth(p.xz + rd.xz * travel);
            travel = mix(travel, max(sea - bed, 0.0) / down, 0.6);
        }
    }
    // A hull just under the surface is seen through the water before it, not the bed's depth:
    // a deep sea's colour does not fog it (deeper down, what stands on the bed keeps the bed's).
    travel = mix(travel, min(travel, distance(under, p)), object * (1.0 - smoothstep(4.0, 10.0, below)));
    var body_alpha = (1.0 - exp(-travel * frame.water_color.a)) * frame.water.z;
    // Light scattered back out of the water. Wave faces turned towards the sun scatter a
    // little more, which keeps the waves visible from TA's steep viewing angle.
    // (By the wind's waves even where a wake's slick smooths them: it calms the glitter and
    // the ripples, not the colour of the water.)
    let shade_n = select(n, normalize(vec3<f32>(-waves.slope.x * strength * calm, 1.0, -waves.slope.y * strength * calm)), wake.z > 0.0);
    let wave_shade = saturate(1.0 + 3.0 * (dot(shade_n, sun_dir) - sun_dir.y));
    let scatter = frame.ambient.rgb * 0.8 + sun_rad * saturate(sun_dir.y) * mix(0.45, 1.0, shadow) * wave_shade;
    var body = frame.water_color.rgb * (scatter + lights.diffuse * 0.6);
    // Acid: its own colour tints what it mirrors (the film on it), a little less of it (the
    // scum's skin), and a murky, turbid body that shows as the view grazes it.
    let acid = liquid_acid();
    let hue = liquid_hue();
    let ndv = saturate(dot(n, v));
    // (Seen in perspective, a little more than Schlick's fifth power, so that the sea brightens
    // with the sky towards the horizon as the eye expects.)
    let fresnel = (0.02 + 0.98 * pow(1.0 - ndv, select(5.0, 4.0, beyond_on()))) * mix(1.0, 0.7, acid);
    if (acid > 0.0) {
        body += hue * (scatter * (0.05 * min(fresnel * 4.0, 1.0)) + 0.004) * acid;
    }
    // Modern camera: shallows over a light bed glow in the liquid's own hue, a blue sea's
    // turned turquoise (sunlight scattered back up from the sand; on acid a sickly
    // yellow-green), the deep stays dark. By the smoothed ground's depth: the mesh's would
    // show its triangles as glassy facets.
    if (beyond_on()) {
        let smooth_depth = mix(max(sea - ground_smooth(p.xz), 0.0), depth, far);
        let shallow = (1.0 - smoothstep(1.5, 26.0, smooth_depth)) * (1.0 - far);
        let cyan = smoothstep(0.0, 0.3, hue.g - hue.r);
        let own = mix(hue, vec3<f32>(0.21, 1.17, 1.17), 0.6 * cyan) * 0.47;
        let tint = mix(own, mix(hue * 0.42, vec3<f32>(0.24, 0.5, 0.1), 0.4), acid);
        let lagoon = tint * (frame.ambient.rgb * 0.6 + sun_rad * saturate(sun_dir.y) * mix(0.5, 1.0, shadow));
        body = mix(body, lagoon, shallow * 0.65);
        // Out over the deep the water is a richer, darker shade of the painted sea.
        let deep = smoothstep(16.0, 70.0, smooth_depth) * (1.0 - acid);
        let luma = dot(body, vec3<f32>(0.2126, 0.7152, 0.0722));
        body = mix(body, max(mix(vec3<f32>(luma), body, 1.35) * 0.9, vec3<f32>(0.0)), 0.6 * deep);
    }
    // Water a hull churned is full of bubbles: paler, milkier and more opaque, long after its
    // foam has gone.
    let bubbles = saturate(wake.a * 1.4) * 0.6;
    if (bubbles > 0.0) {
        body = mix(body, mix(body * 2.2, foam_color(shadow, lights.diffuse, 0.0) * 0.4, 0.35), bubbles);
        body_alpha = mix(body_alpha, max(body_alpha, 0.8), bubbles);
    }

    // ---- Surface: Fresnel reflection, sun glitter, point light sparkle -----------------------
    let sky_plain = sky_radiance(reflect(-v, n));
    let sky = mix(sky_plain, vec3<f32>(dot(sky_plain, vec3<f32>(0.2126, 0.7152, 0.0722))) * hue, 0.45 * acid);
    // Sun glitter: GGX times the sun's irradiance (a factor of π would be exact; 1.8 keeps it
    // sparkling rather than washing out the art), capped before bloom turns it into a sheet.
    let glitter = sun_rad * min(ggx_specular(n, v, sun_dir, alpha2) * 1.8 * shadow, 6.0);
    let sparkle = lights.specular * 0.35;

    // ---- Surf along the shore ----------------------------------------------------------------
    // (on the map and next to it: the far field's shores are not painted)
    var foam = 0.0;
    var foam_thick = 0.0;
    if (frame.water.y > 0.0 && far < 1.0 && waterline < SURF_REACH) {
        // (The distance from the waterline is the same field as the terrain's beyond it, rising
        // by a unit per unit seawards, down the bed's slope.)
        let bed_slope = ground_gradient(p.xz);
        let rise = max(length(bed_slope), 0.03);
        let seaward = -bed_slope / rise;
        let band = length(vec2<f32>(dot(seaward, dxz), dot(seaward, dyz)));
        let art_px = surface_art_px(p.xz, sea);
        let above = foam_from_above(view_dir(p).y);
        let f = shore_foam(art_px, p.xz, waterline, rise, t, FoamPixel(dxz, dyz, max(band, 0.25 * footprint), above));
        // Up a beach (and seen from above, up a bank) the ground carries the surf on past the
        // water's edge; elsewhere nothing does, so it fades out at the edge instead of being cut
        // off by it.
        let carried = mix(smoothstep(0.0, 1.5, depth / rise), 1.0, foam_runs_up(rise, above));
        foam = f.x * carried * frame.water.y * (1.0 - far);
        foam_thick = f.y;
    }

    // ---- Surf standing on a shore, seen from low down ------------------------------------------
    // From low over the sea the band of surf lying on the water is foreshortened to a sliver; but
    // the surf stands up off it (the wavelets breaking on the shore, their spray), and standing,
    // it is not foreshortened: a shore seen from low down is lined with broken white water. It is
    // drawn as a wall at the water's edge on the water in front of it (as much of the water as the
    // wall would hide, its top at the edge), as high as the breakers there, but never less than a
    // few pixels (far off they would be lost; nearer, the band on the water shows the surf): white
    // and ragged on top, a little darker below where the breaking face is (`surf_face`), in clumps
    // with gaps between them, swelling and falling as sets of waves come in. (The distance field
    // it is measured in reaches 32 units out to sea: from very low down, the wall is no higher
    // than that hides.)
    var surf_face = 0.0;
    let low = 1.0 - smoothstep(0.12, 0.3, v.y);
    if (beyond_on() && frame.water.y > 0.0 && far < 1.0 && low > 0.0 && waterline > 0.0 && waterline < 30.0) {
        // (The distance's gradient: seawards, about a unit long within its reach.)
        // (The two in a loop, `shore_distance` written once: see `noise_sum`.)
        var ahead = vec2<f32>(0.0);
        for (var j = 0u; j < 2u; j++) {
            ahead = select(ahead, vec2<f32>(shore_distance(p.xz + select(vec2<f32>(2.0, 0.0), vec2<f32>(0.0, 2.0), j == 1u))), vec2<u32>(0u, 1u) == vec2<u32>(j));
        }
        let g = ahead * 0.5 - waterline * 0.5;
        let toward = v.xz / max(length(v.xz), 1e-4);
        // (How the shore faces the eye: a wall seen edge-on, at an island's ends, thins away.)
        let facing = dot(g, toward) / max(length(g), 1e-3) * smoothstep(0.3, 0.6, length(g));
        if (facing > 0.05) {
            // Where along the shore (the edge's nearest point): the breakers there.
            let at = p.xz - g / max(dot(g, g), 1e-3) * waterline;
            let sets = 0.7 + 0.3 * sin(t * 0.9 + dot(at, vec2<f32>(0.031, 0.047)));
            let clumps = smoothstep(-0.25, 0.25, noise(at / 13.0 + vec2<f32>(t * 0.04, 2.7)) + 0.6 * noise(at / 5.1 + vec2<f32>(-t * 0.07, 8.1)));
            let ragged = 0.7 + 0.6 * noise(at / 1.9 + vec2<f32>(t * 0.3, 4.4));
            // (World units a pixel spans upright there.)
            let pixel = min(length(dxz), length(dyz));
            let height = max(SURF_STAND, 5.0 * pixel) * sets * clumps * ragged;
            // How far in front of the edge the wall's top hides the water, and where in its
            // height this point is (1 = its top, at the edge).
            let tan_e = v.y / max(length(v.xz), 1e-3);
            let hide = max(height / max(tan_e, 0.01), 1e-3);
            let h = 1.0 - waterline / facing / hide;
            // (Its edges never sharper than a pixel: how much `h` changes across one.)
            let aa = length(vec2<f32>(dot(g, dxz), dot(g, dyz))) / facing / hide;
            let crest = smoothstep(0.3 - aa, 0.3 + aa, h);
            let face = smoothstep(-aa, aa, h) * (1.0 - crest);
            let wall = low * min(frame.water.y, 1.0) * smoothstep(0.05, 0.2, facing) * (1.0 - far);
            foam = max(foam, crest * wall * 0.95);
            foam_thick = max(foam_thick, crest * wall);
            surf_face = face * wall;
        }
    }

    // ---- Foam round what stands in the water ------------------------------------------------
    // A collar of foam where a hull, a pontoon or a wading unit's legs meet the water, thinning
    // out a couple of units from it and breathing with the swell (the model's own waterline is
    // in unit.wgsl).
    if (frame.water.y > 0.0 && object > 0.0 && to_contact < COLLAR) {
        let froth = foam_noise(p.xz * 1.5, t, footprint * 1.5);
        let reach = COLLAR * (0.8 + 0.2 * sin(t * 1.3 + dot(p.xz, vec2<f32>(0.11, 0.07))));
        let cover = (1.0 - smoothstep(0.0, reach, to_contact)) * (1.0 - smoothstep(2.0, 4.0, below)) * object;
        // (Broken into patches along the hull, as the water slaps against it here and there.)
        let patches = smoothstep(-0.25, 0.3, noise(p.xz / 7.0 + vec2<f32>(t * 0.07, -t * 0.05)));
        let f = foam_cover(froth, 1.3 - 2.3 * cover * mix(0.55, 1.0, patches), FoamPixel(dxz * 1.5, dyz * 1.5, 0.0, 0.0));
        let collar = f.x * smoothstep(0.0, 0.35, cover) * mix(0.5, 0.85, f.y) * min(frame.water.y, 1.0);
        foam = max(foam, collar);
        foam_thick = max(foam_thick, f.y * cover);
    }

    // ---- Wakes -------------------------------------------------------------------------------
    // The foam ships and hovercraft leave (see `wake` above): the denser, the more of the foam's
    // texture shows, broken into patches where it thins out, and only a fleck here and there in
    // the pale band of bubbles left down a wake's middle.
    if (frame.water.y > 0.0 && wake.a > 0.04) {
        let n = foam_noise(p.xz * 1.3, t, footprint * 1.3);
        let f = foam_cover(n, 2.2 - 3.6 * saturate(wake.a), FoamPixel(dxz * 1.3, dyz * 1.3, 0.0, 0.0));
        let trail = f.x * smoothstep(0.0, 0.25, wake.a) * mix(0.55, 0.92, f.y) * min(frame.water.y, 1.0);
        foam = max(foam, trail);
        foam_thick = max(foam_thick, f.y * saturate(wake.a));
    }

    // ---- Whitecaps out at sea (modern camera) ---------------------------------------------
    // Where the wind blows hard the crests break (see `whitecaps`): on open, deep water away from
    // the shore's surf (few on lakes and in narrow bays, where the wind has little fetch), and
    // only while they span a good few pixels: from high up they would be specks.
    if (beyond_on() && frame.water.y > 0.0) {
        let near = 1.0 - smoothstep(0.6, 1.4, footprint);
        let open = mix(smoothstep(0.6, 0.92, sea_fraction(p.xz, 5.0)), 1.0, far);
        let offshore = smoothstep(8.0, 30.0, depth) * mix(smoothstep(16.0, 31.0, waterline), 1.0, far);
        let k = near * open * offshore * smoothstep(0.2, 0.8, strength) * still;
        if (k > 0.0) {
            let caps = turning_whitecaps(p.xz, t, FoamPixel(dxz, dyz, 0.0, 0.0), frame.wind.xy, waves.crest, waves.gust) * k;
            foam_thick = max(foam_thick, caps);
            foam = max(foam, caps * 0.75);
        }
    }

    // ---- Compose (premultiplied alpha) ------------------------------------------------------
    // Bottom to top: sea bed (already in the target) -> body -> reflection -> foam, plus
    // additive highlights. The body, reflection and highlights fade in from the waterline
    // (hiding the heightmap's 16 px contour); the foam carries on from the terrain's.
    // Where something meets the surface (a hull at its waterline, a fin or a pad awash) the water
    // thins out over its last half unit, so what lies flush with it is not cut out of it pixel by
    // pixel by the depth test.
    let contact = 1.0 - object * (1.0 - smoothstep(0.0, 0.6, below));
    let edge = smoothstep(0.0, 10.0, shore_dist) * contact;
    var rgb = body * body_alpha;
    var a = body_alpha;
    if (frame.water_extra.z > 0.0) {
        // Where the world above the water is mirrored (see `mirrored_world`), it is reflected
        // more strongly than the Fresnel term alone (`water_extra.w`): from TA's steep views
        // the physical reflectance (2-5%) would hide it. Elsewhere the sky is.
        var world = mirrored_world(mirror.xy, mirror_dx, mirror_dy, mirror.zw);
        world = vec4<f32>(mix(world.rgb, vec3<f32>(dot(world.rgb, vec3<f32>(0.2126, 0.7152, 0.0722))) * hue, 0.21 * acid), world.a);
        let world_reflectance = fresnel + (1.0 - fresnel) * frame.water_extra.w;
        let reflectance = mix(fresnel, world_reflectance, world.a);
        rgb = rgb * (1.0 - reflectance) + sky * (fresnel * (1.0 - world.a)) + world.rgb * world_reflectance;
        a = a + (1.0 - a) * reflectance;
    } else {
        rgb = rgb * (1.0 - fresnel) + sky * fresnel;
        a = a + (1.0 - a) * fresnel;
    }
    // Scum drifting on acid (modern camera, like the whitecaps): a dull olive film, thicker and
    // paler where it piles up along a raft's edge and round its holes, mottled and wrinkled up
    // close (lit over the waves it rides, with a faint oily sheen); it dulls the glitter.
    var scum = 0.0;
    if (acid > 0.0 && beyond_on()) {
        let film = acid_scum(p.xz, t, FoamPixel(dxz, dyz, 0.0, 0.0));
        let tex = scum_texture(p.xz, t, footprint);
        let thick = saturate(0.5 + 0.45 * tex.x + 0.35 * film.y);
        // (Fainter seen from high up, where the film's wrinkles and holes are too fine to show:
        // flat olive at full strength read as printed on the sea.)
        scum = film.x * mix(0.4, 0.62, thick) * acid * mix(1.0, 0.7, smoothstep(0.8, 2.5, footprint));
        let ns = normalize(vec3<f32>(-slope.x - tex.y * 0.5, 1.0, -slope.y - tex.z * 0.5));
        let sun = sun_rad * saturate(dot(ns, sun_dir)) * mix(shade_fill(), vec3<f32>(1.0), shadow);
        let albedo = mix(hue * 0.12, vec3<f32>(0.2, 0.22, 0.05), 0.5) * mix(0.7, 1.3, thick);
        let sheen = sky * (0.04 + 0.5 * pow(1.0 - saturate(dot(ns, v)), 5.0)) * 0.5;
        let scum_color = albedo * (frame.ambient.rgb * 0.85 + sun + lights.diffuse) + sheen;
        rgb = rgb * (1.0 - scum) + scum_color * scum;
        a = a + (1.0 - a) * scum;
    }
    rgb = (rgb + (glitter + sparkle) * (1.0 - scum)) * edge;
    a *= edge;
    // (The face of the surf breaking on a shore, seen from low down, a little darker and not
    // mirroring what stands on the shore.)
    rgb = mix(rgb, body, 0.45 * surf_face);
    a = mix(a, 1.0, 0.45 * surf_face);
    rgb = rgb * (1.0 - foam) + foam_color(shadow, lights.diffuse, foam_thick) * foam;
    a = a + (1.0 - a) * foam;

    // Fade out at the map edges like the terrain does.
    let fade = map_edge_fade(p, v, depth);
    rgb *= fade;
    a *= fade;

    // Fog, premultiplied: the colour tends towards alpha * fog colour.
    rgb = mix(fog_of_war(rgb, p), fog_color_at(p) * a, water_fog(p));
    // What lies under the water (drawn, fogged, below it), bent by the waves where it shows
    // through.
    if (frame.water_extra.x > 0.0) {
        rgb += refracted(frag.xy, under, below, slope) * ((1.0 - a) * fade);
    }
    return vec4<f32>(rgb, a);
}

// ---- Heat haze over lava (see heat.rs) -----------------------------------------------------
// The lava surface drawn again, adding a thin layer of hot air to the scene target's alpha
// (colour untouched): from a little way out from the bank, fading with distance from the
// camera since the distortion is measured in screen pixels.

fn lava_heat(p: vec3<f32>, depth: f32) -> vec4<f32> {
    if (depth <= 0.0) {
        discard;
    }
    let dist = distance(frame.camera_pos.xyz, p);
    let amount = frame.heat.x * 0.6 * smoothstep(0.0, 8.0, depth) * min(1.0, 800.0 / max(dist, 1.0));
    return vec4<f32>(0.0, 0.0, 0.0, amount);
}

@fragment
fn fs_heat_water(in: VsOut) -> @location(0) vec4<f32> {
    return lava_heat(in.world, frame.world.y - mesh_height(in.world.xz));
}

@fragment
fn fs_heat_outer(in: VsOut) -> @location(0) vec4<f32> {
    let d = beyond_distance(in.world.xz);
    let far = beyond_coast_mix(d);
    var depth = frame.world.y - in.ground;
    if (far > 0.0) {
        depth = mix(depth, beyond_water_depth(in.world.xz, d), far);
    }
    return lava_heat(in.world, depth);
}
