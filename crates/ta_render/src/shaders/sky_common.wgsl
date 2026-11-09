// Sky model shared by the sky pass and the water's reflections (perspective camera). Needs
// noise.wgsl and clouds.wgsl (the cloud layer's shape, shared with the cloud shadows) prepended.
//
// * Gradient: the horizon haze (`haze_color`, the colour distance fog tends to, including the
//   sun's glow) rising to the zenith colour; below the horizon the haze continues, so ground
//   that dissolves into it, or is clipped by the far plane, meets the sky without a seam.
// * Sun: a limb-darkened HDR disc (it blooms) in the sun direction.
// * Clouds: fractal gradient noise on a layer high above the ground, drifting with the wind,
//   self-shadowed along the sun direction, fading into the haze with distance, thinning
//   towards the horizon and the zenith. The layer is a shell over a curved planet, so it
//   stays at a finite distance at the horizon instead of stretching into streaks. Octaves are
//   band-limited by the pixel footprint on the layer, so distant clouds do not shimmer.

// Clear-sky colour along the view direction `dir` (no clouds, no sun disc).
fn sky_gradient(dir: vec3<f32>) -> vec3<f32> {
    if (frame.liquid.y > 0.5) {
        return space_backdrop(dir) + sun_glow(dir);
    }
    let up = 1.0 - exp(-4.0 * max(dir.y, 0.0));
    return mix(frame.fog.rgb, frame.sky_zenith.rgb, up) + sun_glow(dir) + dusk_glow(dir);
}

// Value noise in 3D (-1..1 about): eight hashed lattice values blended smoothly; and three
// octaves of it.
fn value3(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    var corners: array<f32, 8>;
    for (var k = 0; k < 8; k++) {
        let o = vec3<i32>(k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let c = vec3<u32>(i + o + vec3<i32>(65536));
        corners[k] = f32(hash_u32(c.x ^ hash_u32(c.y ^ hash_u32(c.z + 0x9E37u))) >> 8u) / 16777215.0;
    }
    let x00 = mix(corners[0], corners[1], u.x);
    let x10 = mix(corners[2], corners[3], u.x);
    let x01 = mix(corners[4], corners[5], u.x);
    let x11 = mix(corners[6], corners[7], u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z) * 2.0 - 1.0;
}

fn fbm3(p: vec3<f32>) -> f32 {
    return value3(p) * 0.55 + value3(p * 2.03 + vec3<f32>(1.7, -2.3, 0.4)) * 0.3 + value3(p * 4.1 - vec3<f32>(0.8, 1.1, -2.9)) * 0.15;
}

// In space (`frame.liquid.y`): black all round, the band of the galaxy across it, faint
// nebulae, and far below the map a planet, its night side dark, its day side banded, its
// atmosphere a thin bright rim.
fn space_backdrop(dir: vec3<f32>) -> vec3<f32> {
    var c = vec3<f32>(0.002, 0.002, 0.004);
    // The galaxy's band: a great circle tilted across the sky, brighter towards its core.
    let pole = normalize(vec3<f32>(0.35, 0.55, 0.76));
    let band = exp(-pow(dot(dir, pole) / 0.22, 2.0));
    let core = pow(max(dot(dir, normalize(vec3<f32>(-0.6, 0.3, 0.5))), 0.0), 3.0);
    let cloud = fbm3(dir * 3.2) * 0.5 + 0.5;
    c += vec3<f32>(0.05, 0.045, 0.06) * band * (0.4 + 0.8 * cloud) * (0.6 + 1.2 * core);
    // Nebulae: a violet and a teal veil.
    let n1 = smoothstep(0.55, 0.85, fbm3(dir * 2.1 + vec3<f32>(3.1, 0.7, -1.3)) * 0.5 + 0.5);
    let n2 = smoothstep(0.6, 0.9, fbm3(dir * 2.7 + vec3<f32>(-2.4, 1.9, 0.6)) * 0.5 + 0.5);
    c += vec3<f32>(0.045, 0.012, 0.06) * n1 + vec3<f32>(0.008, 0.035, 0.04) * n2;
    // The planet below: a disc about a fifth of the way round, straight down.
    let below = -dir.y;
    let r = 0.62;
    if (below > r) {
        let d = sqrt(max(1.0 - below * below, 0.0)) / sqrt(1.0 - r * r);
        let lit = saturate(dot(normalize(vec3<f32>(dir.x, 0.0, dir.z) + vec3<f32>(1e-4)), normalize(vec3<f32>(frame.celestial.x, 0.0, frame.celestial.z))) * 0.5 + 0.5);
        let bands = 0.5 + 0.5 * sin(dir.z * 38.0 + fbm3(dir * 6.0) * 2.5);
        var p = mix(vec3<f32>(0.16, 0.11, 0.07), vec3<f32>(0.28, 0.22, 0.15), bands);
        p *= 0.15 + 0.85 * smoothstep(0.2, 0.8, lit);
        let rim = smoothstep(0.75, 1.0, d) * (0.4 + 0.6 * lit);
        c = mix(c, p, smoothstep(0.0, 0.03, below - r)) + vec3<f32>(0.1, 0.16, 0.26) * rim * 0.6;
    }
    return c;
}

// Stars on one cube face. Include neighboring cells whose dots overlap this ray: looking up
// only floor(p) cuts off a star at the cell border and makes it pop as the camera moves.
fn star_grid(uv: vec2<f32>, face: u32, pixel: f32, time: f32) -> vec3<f32> {
    let cells = 260.0;
    let p = uv * cells;
    let footprint = max(pixel, 1e-5) * cells / 1.57;
    let radius = 1.4 * footprint;
    // At extreme minification many stars fit in a pixel. Fade that unresolved field smoothly,
    // keeping the support within the 3 x 3 neighborhood (centres stay 0.25 inside each cell).
    let visibility = 1.0 - smoothstep(1.0, 1.25, radius);
    if (visibility <= 0.0) {
        return vec3<f32>(0.0);
    }
    // Only cells whose possible centres touch the support: usually one or two per axis, not
    // nine hashes at every sky pixel. Most candidates have no star or lie outside the disc.
    let lo = vec2<i32>(ceil(p - radius - 0.75));
    let hi = vec2<i32>(floor(p + radius - 0.25));
    var color = vec3<f32>(0.0);
    for (var y = lo.y; y <= hi.y; y++) {
        for (var x = lo.x; x <= hi.x; x++) {
            let c = vec2<i32>(x, y);
            // A face owns centres only inside its edges. This also keeps other faces from
            // inventing an extra row of stars beyond their shared boundary.
            if (any(c < vec2<i32>(-260)) || any(c >= vec2<i32>(260))) {
                continue;
            }
            let h = hash_u32(u32(c.x + 4096) * 73856093u ^ u32(c.y + 4096) * 19349663u ^ face * 83492791u);
            if ((h & 7u) > 1u) {
                continue;
            }
            let h2 = hash_u32(h);
            let centre = vec2<f32>(c) + 0.25 + 0.5 * vec2<f32>(f32(h2 & 255u), f32((h2 >> 8u) & 255u)) / 255.0;
            let px = length(p - centre) / footprint;
            if (px >= 1.4) {
                continue;
            }
            let dot_size = 1.0 - smoothstep(0.3, 1.4, px);
            let bright = pow(f32((h2 >> 16u) & 255u) / 255.0, 6.0) * 5.0 + 0.25;
            let twinkle = 0.75 + 0.25 * sin(time * (1.5 + f32(h2 >> 28u)) + f32(h2 & 1023u));
            let tint = mix(vec3<f32>(0.75, 0.82, 1.0), vec3<f32>(1.0, 0.9, 0.78), f32((h2 >> 24u) & 15u) / 15.0);
            color += tint * (dot_size * bright * twinkle);
        }
    }
    return color * visibility;
}

// The night sky's stars (with a time of day, see daylight.rs): one in some of the cells of
// a fine grid over the sky (on the faces of a cube, so they do not bunch at the zenith), a
// few of them bright, a little tinted, twinkling; they fade into the haze low over the
// horizon. About a pixel across whatever the zoom (`pixel` = angular size of a pixel).
fn star_field(dir: vec3<f32>, pixel: f32) -> vec3<f32> {
    // (By day too where there is no air.)
    let n = max(frame.night.a, frame.celestial.w);
    // (In space there is no horizon: stars below as above, but not over the planet below.)
    let space = frame.liquid.y > 0.5;
    if (n <= 0.0 || (dir.y <= 0.0 && !space) || (space && -dir.y > 0.62)) {
        return vec3<f32>(0.0);
    }
    let a = abs(dir);
    let dominant = max(max(a.x, a.y), a.z);
    // Usually one face; right at a cube edge include the adjacent face's own star centres.
    // Otherwise the same clipped support would remain visible along the six face boundaries.
    let edge = dominant / (1.0 + 1.25 / 260.0);
    var color = vec3<f32>(0.0);
    if (a.x >= edge) {
        color += star_grid(dir.yz / a.x, select(0u, 1u, dir.x > 0.0), pixel, frame.world.x);
    }
    if (a.y >= edge) {
        color += star_grid(dir.xz / a.y, select(2u, 3u, dir.y > 0.0), pixel, frame.world.x);
    }
    if (a.z >= edge) {
        color += star_grid(dir.xy / a.z, select(4u, 5u, dir.z > 0.0), pixel, frame.world.x);
    }
    let horizon = select(smoothstep(0.02, 0.25, dir.y), 1.0, space);
    return color * (horizon * n);
}

// Where a view ray meets the cloud layer: world xz (drifting with the wind) and the distance
// along the ray. Rays at or below the horizon are clamped to it.
fn cloud_plane(dir: vec3<f32>, t: f32) -> vec3<f32> {
    let s = max(dir.y, 0.0);
    let r = CLOUD_PLANET;
    let dist = -r * s + sqrt(r * r * s * s + CLOUD_HEIGHT * (2.0 * r + CLOUD_HEIGHT));
    let flat_dir = dir.xz / max(length(dir.xz), 1e-4) * sqrt(max(1.0 - s * s, 0.0));
    // (Offset by the map's own seed: maps seen from the same place do not share a skyline.)
    let p = frame.camera_pos.xz + flat_dir * dist - CLOUD_WIND * (frame.sky.y * t) + frame.ground_beyond.zw;
    return vec3<f32>(p, dist);
}

// The cloud layer along `dir`: rgb = premultiplied colour, a = coverage. `plane` comes from
// `cloud_plane`, `footprint` is the pixel size on the layer (world units).
fn sky_clouds(dir: vec3<f32>, plane: vec3<f32>, footprint: f32) -> vec4<f32> {
    let cover = frame.sky_zenith.a;
    if (cover <= 0.0 || dir.y <= 0.0) {
        return vec4<f32>(0.0);
    }
    let p = plane.xy;
    // A slow domain warp gives the billows some swirl.
    let warp = cloud_warp(p);
    let density = cloud_fbm(p + warp, footprint, 5);
    // Coverage 0..1: the threshold falls as the cover rises; soft, wispy edges.
    let threshold = cloud_threshold(cover);
    var coverage = cloud_coverage(density, threshold);
    // Thin out towards the horizon (hazy and foreshortened) and a little at the zenith.
    coverage *= smoothstep(0.0, 0.1, dir.y) * (1.0 - 0.4 * smoothstep(0.6, 1.0, dir.y));
    if (coverage <= 0.002) {
        return vec4<f32>(0.0);
    }
    // Self-shadowing: denser towards the sun means this side faces away from it.
    let sun_flat = frame.sun_dir.xz / max(length(frame.sun_dir.xz), 1e-3);
    let towards_sun = cloud_fbm(p + warp + sun_flat * (CLOUD_SCALE * 0.08), footprint, 2);
    let lit = saturate(0.6 - (towards_sun - density) * 2.2);
    let sun = frame.sun_color.rgb * frame.sun_dir.w * saturate(frame.sun_dir.y * 2.5 + 0.1);
    let skylight = mix(frame.fog.rgb, frame.sky_zenith.rgb, 0.4);
    // Thick cores are greyer underneath, their sunlit tops bright; thin edges glow when the sun
    // is behind them.
    let thickness = saturate((density - threshold) * 1.8);
    var color = skylight * mix(0.95, 0.55, thickness) + sun * (0.22 + 0.62 * lit);
    let forward = pow(saturate(dot(dir, frame.sun_dir.xyz)), 10.0);
    color += frame.sun_color.rgb * frame.sun_dir.w * forward * (1.0 - thickness) * 1.2;
    // Aerial perspective: distant clouds sink into the horizon haze.
    let haze = 1.0 - exp(-plane.z / 70000.0);
    color = mix(color, haze_color(dir), haze);
    return vec4<f32>(color * coverage, coverage);
}

// The sun disc (HDR, limb darkened) along `dir`; `pixel` = angular size of a pixel (radians).
fn sun_disc(dir: vec3<f32>, pixel: f32) -> vec3<f32> {
    let radius = frame.sky.x;
    let sun = frame.sun_dir.xyz;
    if (radius <= 0.0 || dot(dir, sun) <= 0.0) {
        return vec3<f32>(0.0);
    }
    // sin of the angle to the sun's centre: accurate for the small angles that matter here.
    let angle = length(cross(dir, sun));
    let aa = max(pixel, radius * 0.02);
    let inside = 1.0 - smoothstep(radius - aa, radius + aa, angle);
    let r = saturate(angle / radius);
    let limb = 0.55 + 0.45 * sqrt(1.0 - r * r);
    // Sinks below the horizon smoothly.
    let above = smoothstep(-radius, radius, dir.y);
    // At night the disc is the moon: pale, not blue like its light, with its grey seas.
    let night = frame.night.a;
    var color = frame.sun_color.rgb;
    if (night > 0.0) {
        let e1 = normalize(cross(sun, vec3<f32>(0.0, 1.0, 0.0)));
        let e2 = cross(e1, sun);
        let local = vec2<f32>(dot(dir, e1), dot(dir, e2)) / radius;
        let seas = 0.78 + 0.22 * smoothstep(-0.2, 0.4, noise(local * 2.3 + vec2<f32>(3.1, 1.7)));
        color = mix(color, vec3<f32>(1.0, 0.97, 0.9) * seas, night);
    }
    return color * frame.sun_dir.w * (14.0 * inside * limb * above);
}
