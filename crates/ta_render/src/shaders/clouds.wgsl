// The cloud layer's shape, shared by the sky (sky_common.wgsl, drawn and reflected) and the
// clouds' shadows on the ground (cloud_shadow.wgsl, see clouds.rs), so the shadows are those
// of the clouds in the sky, drifting with them. Needs noise.wgsl prepended; no bindings.

// Height of the cloud layer above the eye, the planet radius that curves it, and the size of
// the largest cloud features (world units).
const CLOUD_HEIGHT: f32 = 5000.0;
const CLOUD_PLANET: f32 = 120000.0;
const CLOUD_SCALE: f32 = 7000.0;
// Wind direction of the cloud drift (unit, world xz); the field moves CLOUD_WIND * speed * t.
const CLOUD_WIND: vec2<f32> = vec2<f32>(0.8, 0.6);

// Fractal noise of the cloud layer at `p`: `octaves` octaves, each faded out as its wavelength
// approaches `footprint` (world units per pixel on the layer). Mean 0, std about 0.2.
fn cloud_fbm(p: vec2<f32>, footprint: f32, octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 1.0;
    var wavelength = CLOUD_SCALE;
    var q = p / CLOUD_SCALE;
    for (var i = 0; i < octaves; i++) {
        let band = smoothstep(2.5, 6.0, wavelength / max(footprint, 1e-3));
        sum += amp * band * noise(q + vec2<f32>(f32(i) * 17.3, f32(i) * -9.1));
        // Rotate and scale each octave so the lattices never line up.
        q = vec2<f32>(q.x * 1.6 - q.y * 1.2, q.x * 1.2 + q.y * 1.6);
        amp *= 0.5;
        wavelength *= 0.5;
    }
    return sum;
}

// A slow domain warp of the layer at `p` (drifting coordinates): gives the billows some swirl.
fn cloud_warp(p: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(noise(p / 19000.0 + vec2<f32>(1.7, 9.2)), noise(p / 19000.0 + vec2<f32>(-6.1, 2.4))) * 5000.0;
}

// The density threshold above which there is cloud, for a cloud cover (0 = clear, 1 = overcast).
fn cloud_threshold(cover: f32) -> f32 {
    return mix(0.34, -0.26, cover);
}

// Cloud coverage (0..1) of a layer `density` above `threshold`: soft, wispy edges.
fn cloud_coverage(density: f32, threshold: f32) -> f32 {
    return smoothstep(threshold - 0.04, threshold + 0.32, density);
}
