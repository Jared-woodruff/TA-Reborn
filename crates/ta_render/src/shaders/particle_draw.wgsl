// GPU particles, drawing (see gpu_particles.rs). Every live particle is a camera-facing quad:
// one indexed draw per layer pulls four vertices per particle straight from the particle buffer
// (additive particles from its front, alpha-blended ones from its back), no instancing, so the
// vertex work packs densely. Sparks stretch into streaks along their flight, droplets of spray
// and rain into fine ones; fire, dust and smoke are billows; grit is clods, irregular and of every
// size; a fireball is drawn per pixel. Particles are depth tested (no depth write) at their
// centre nudged towards the camera, so a spark lying on the ground is not cut in half by it.
// Light, fog of war, haze and the height above the ground are worked out per vertex: the
// fragments only shape and blend. The water mirrors them (the `_reflection` entry points).
// Prepended: common.wgsl, noise.wgsl, particle_common.wgsl.

@group(1) @binding(0) var<uniform> params: ParticleParams;
@group(1) @binding(1) var<storage, read> particles: array<Particle>;
// Four billow shapes in a 2 x 2 atlas (r = density), with mips.
@group(1) @binding(2) var billows: texture_2d<f32>;
@group(1) @binding(3) var billow_sampler: sampler;
// Sorted separately for this camera and its reflection, back to front within each layer.
@group(1) @binding(4) var<storage, read> draw_order: array<u32>;
struct DrawCounts { words: array<u32>, };
@group(1) @binding(5) var<storage, read> draw_counts: DrawCounts;

// Sparks stretch over the distance they fly in this long (s), up to STREAK_MAX world units.
const STREAK_SECONDS: f32 = 0.07;
const STREAK_MAX: f32 = 40.0;
// Droplets of spray and flying grit blur over a shorter time (s).
const DROPLET_SECONDS: f32 = 0.045;
const CLOD_SECONDS: f32 = 0.02;
// Rain streaks: never longer or wider on screen than this many pixels (a drop right in front of
// the eye would be a bar across the screen), and gone within this many units of the eye.
const RAIN_LONGEST_PIXELS: f32 = 56.0;
const RAIN_WIDEST_PIXELS: f32 = 1.4;
const RAIN_NEAR: vec2<f32> = vec2<f32>(24.0, 80.0);
// Nothing is drawn narrower than this many pixels: smaller particles fade instead, so distant
// sparks do not flicker between pixels.
const MIN_PIXELS: f32 = 1.75;
// Specks (`FLAG_SPECK`) are never drawn wider than this share of the screen's height (6 pixels
// at 1440 lines).
const SPECK_SHARE: f32 = 0.0042;
// How much light a fireball's soot sends back (its colour as smoke).
const SOOT: vec3<f32> = vec3<f32>(0.12, 0.11, 0.1);

// How the quad is filled (VOut.look.x).
const LOOK_GLOW: u32 = 0u;
const LOOK_STREAK: u32 = 1u;
const LOOK_BILLOW: u32 = 2u;
const LOOK_CLOD: u32 = 3u;
const LOOK_DROPLET: u32 = 4u;
const LOOK_FIREBALL: u32 = 5u;
const LOOK_RAIN: u32 = 6u;

struct VOut {
    @builtin(position) clip: vec4<f32>,
    // Position on the quad: -1..1 across (x) and along (y; +1 = a streak's head).
    @location(0) uv: vec2<f32>,
    // Additive: HDR colour (a fireball's: its light, and in a how much of it the pixel shows).
    // Alpha-blended: lit colour and opacity. Both already dimmed by the fog of war and the
    // distance haze.
    @location(1) color: vec4<f32>,
    // Alpha-blended and fireballs: the haze's colour times its amount, added over the particle.
    @location(2) haze: vec3<f32>,
    // Height above the ground, for fading out near it.
    @location(3) above: f32,
    // x = look, y = billow shape (0..3; a fireball's seed, 0..255), z = height over which the
    // particle fades out towards the ground (0 = never), w = a fireball's age over its life
    @location(4) @interpolate(flat) look: vec4<f32>,
    // Where the quad's point is (the water's reflection keeps only what is above the water).
    @location(5) world: vec3<f32>,
    // A fireball's soot as lit where it is (dimmed like `color`).
    @location(6) soot: vec3<f32>,
};

// Light glowing at temperature `heat` (1 = white-hot, 0 = cold): white, yellow, orange, deep
// red, then nothing; in units of the particle's brightness.
fn incandescence(heat: f32) -> vec3<f32> {
    let h = saturate(heat) * 4.0;
    let white = vec3<f32>(1.0, 0.9, 0.72) * 2.0;
    let yellow = vec3<f32>(1.0, 0.62, 0.16) * 1.4;
    let orange = vec3<f32>(1.0, 0.3, 0.04);
    let red = vec3<f32>(0.62, 0.05, 0.01) * 0.6;
    if (h > 3.0) {
        return mix(yellow, white, h - 3.0);
    }
    if (h > 2.0) {
        return mix(orange, yellow, h - 2.0);
    }
    if (h > 1.0) {
        return mix(red, orange, h - 1.0);
    }
    return red * h;
}

// A fireball's own colours at temperature `heat` (0..1): from nothing through a deep red to
// orange, a yellow-orange and, only where it is hottest, pale yellow. Held below what would
// burn out to white on screen, so a fireball keeps its colour and the detail of its billows
// (the bloom carries its glare).
fn fire_ramp(heat: f32) -> vec3<f32> {
    let h = saturate(heat);
    let dark = vec3<f32>(0.15, 0.018, 0.003);
    let orange = vec3<f32>(1.05, 0.24, 0.02);
    let amber = vec3<f32>(1.35, 0.6, 0.08);
    let pale = vec3<f32>(1.7, 1.2, 0.5);
    if (h < 0.2) {
        return dark * (h / 0.2);
    }
    if (h < 0.45) {
        return mix(dark, orange, (h - 0.2) / 0.25);
    }
    if (h < 0.7) {
        return mix(orange, amber, (h - 0.45) / 0.25);
    }
    return mix(amber, pale, (h - 0.7) / 0.3);
}

// Gradient noise summed over octaves, each turned and finer than the last (about ±0.5).
fn fbm(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    let turn = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    for (var i = 0; i < octaves; i++) {
        v += a * noise(q);
        q = turn * q * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

// Turbulence: the noise's magnitude summed over octaves (0..about 1): rounded billows with
// creases between them.
fn turbulence(p: vec2<f32>, octaves: i32) -> f32 {
    var v = 0.0;
    var a = 0.5;
    var q = p;
    let turn = mat2x2<f32>(0.8, 0.6, -0.6, 0.8);
    for (var i = 0; i < octaves; i++) {
        v += a * abs(noise(q)) * 2.0;
        q = turn * q * 2.03 + vec2<f32>(1.7, 9.2);
        a *= 0.5;
    }
    return v;
}

struct Fireball {
    // Light given (in units of the particle's brightness).
    glow: vec3<f32>,
    // How much of what lies behind it hides: the gas, thick while it burns, thinning as it
    // turns to smoke.
    gas: f32,
    // How much of the gas is soot (cooled: dark smoke, lit by the world's light).
    soot: f32,
};

// A fireball at the point `uv` of its quad (-1..1) at `t` (its age over its life), `seed` its
// own: a ball of flame with a lumpy outline, billowing inside (cauliflower: rounded billows,
// hotter than the creases between them) and churning upwards. It ignites white-hot, burns
// yellow and orange for the first two thirds of its life, cooling from the outside in, and is
// left a ball of soot that the creases eat into, thinning into wisps. `pixels`: how many the
// quad spans across (the bigger it is on screen, the finer the billows it shows).
fn fireball_look(uv: vec2<f32>, t: f32, seed: f32, pixels: f32) -> Fireball {
    let r = length(uv);
    let q = uv * 1.3 + vec2<f32>(seed * 1.731, seed * 0.917);
    // The outline: big lobes, swelling and drifting a little.
    let lobes = fbm(q * 1.1 + vec2<f32>(t * 0.25, -t * 0.35), 3);
    let reach = 0.6 + 0.3 * sqrt(t);
    let d = r + lobes * (0.42 + 0.25 * t);
    let outline = 1.0 - smoothstep(reach - (0.12 + 0.2 * t), reach, d);
    var f: Fireball;
    if (outline <= 0.0) {
        f.glow = vec3<f32>(0.0);
        f.gas = 0.0;
        f.soot = 0.0;
        return f;
    }
    // The billows, churning upwards through a slow swirl.
    let warp = vec2<f32>(noise(q * 0.8 + vec2<f32>(0.0, t * 0.9)), noise(q * 0.8 + vec2<f32>(5.2, -t * 0.6)));
    let octaves = 3 + i32(clamp(log2(pixels / 160.0), 0.0, 2.0));
    var b = saturate(turbulence(q * 1.25 + warp * 0.9 + vec2<f32>(0.0, t * 1.2), octaves));
    b = b * b * (3.0 - 2.0 * b);
    // As it thins, the creases between the billows open up first.
    let erode = smoothstep(0.55, 1.0, t) * 0.55;
    let body = outline * smoothstep(erode - 0.15, erode + 0.15, b + 0.15);
    // Temperature: white-hot as it ignites, cooling fast; hottest inside and in the billows.
    let peak = 1.0 - smoothstep(0.0, 0.72, t);
    let hot = 1.0 - smoothstep(0.0, reach, d * (1.0 + 1.2 * t));
    let heat = body * (0.15 + peak) * (0.45 + 0.65 * hot) * (0.6 + 0.55 * b);
    f.glow = fire_ramp(heat);
    f.gas = body * (0.85 + 0.15 * b) * (1.0 - smoothstep(0.6, 1.0, t)) * mix(1.0, 0.75, t);
    f.soot = 1.0 - smoothstep(0.15, 0.45, heat);
    return f;
}

// Light on smoke-like particles, as the smoke sprites get it: ambient, sun (under the clouds'
// shadows), the point lights of fires and explosions and on lava worlds the melt's glow.
fn billow_light(world: vec3<f32>) -> vec3<f32> {
    var sun = frame.sun_color.rgb * frame.sun_dir.w * 0.45;
    if (frame.clouds.x > 0.0) {
        sun *= cloud_light(world);
    }
    return frame.ambient.rgb * 0.8 + sun + point_lighting_omni(world) * 0.6 + lava_light(world) * 0.6;
}

// Texture coordinates of billow shape `shape` for a point of the quad.
fn billow_uv(uv: vec2<f32>, shape: f32) -> vec2<f32> {
    let s = u32(shape);
    return (uv * 0.5 + 0.5 + vec2<f32>(f32(s & 1u), f32(s >> 1u))) * 0.5;
}

// How much bigger or smaller than its burst's size a particle is: grit and droplets vary the
// most (a few big clods among many small ones), billows the least.
fn size_variation(kind: u32, seed: u32) -> f32 {
    let r = particle_random(seed, 1u);
    if (kind == KIND_DIRT) {
        return mix(0.35, 1.45, r * r);
    }
    if (kind == KIND_SPARK || kind == KIND_SPRAY) {
        return mix(0.5, 1.3, r);
    }
    if (kind == KIND_EMBER) {
        return mix(0.6, 1.4, r);
    }
    if (kind == KIND_GLOW) {
        return 1.0;
    }
    return mix(0.85, 1.15, r);
}

fn particle_vertex(p: Particle, corner_index: u32) -> VOut {
    var out: VOut;
    let corner = vec2<f32>(f32(corner_index & 1u), f32(corner_index >> 1u)) * 2.0 - 1.0;
    out.uv = corner;
    out.color = vec4<f32>(0.0);
    out.haze = vec3<f32>(0.0);
    out.above = 1.0e4;
    out.look = vec4<f32>(0.0);
    out.world = p.pos;
    out.soot = vec3<f32>(0.0);
    let c0 = frame.view_proj * vec4<f32>(p.pos, 1.0);
    if (p.age < 0.0 || c0.w <= 1e-3) {
        // Not born yet, or behind the camera: outside the clip volume.
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        return out;
    }
    let kind = particle_kind(p);
    let seed = particle_seed(p);
    let t = saturate(p.age / p.life);
    let sizes = unpack2x16float(p.data.x);
    // (A fireball swells fast at first, then more and more slowly.)
    let grown = select(t, 1.0 - pow(1.0 - t, 2.2), kind == KIND_FIREBALL);
    var size = max(mix(sizes.x, sizes.y, grown), 0.01) * size_variation(kind, seed);
    let tint = unpack4x8unorm(p.data.z);
    let brightness = particle_brightness(p);
    let eye = view_dir(p.pos);

    // Screen-aligned basis: true world sizes in perspective, TA's square pixels in classic.
    let right = normalize(frame.billboard_right.xyz);
    var up = frame.billboard_up.xyz;
    if (frame.camera_pos.w > 0.5) {
        up = normalize(up);
    }
    // Pixels per world unit at the particle.
    let c1 = frame.view_proj * vec4<f32>(p.pos + right, 1.0);
    let ppu = length((c1.xy / c1.w - c0.xy / c0.w) * frame.screen.xy) * 0.5;
    var fade = 1.0;
    // (Rain stays a pixel or so wide however near, fading as it thins further.)
    let thinnest = select(MIN_PIXELS, 1.0, kind == KIND_RAIN);
    if (size * ppu < thinnest) {
        let k = size * ppu / thinnest;
        fade = k * k;
        size = thinnest / max(ppu, 1e-4);
    } else if (kind == KIND_RAIN) {
        size = min(size, RAIN_WIDEST_PIXELS / max(ppu, 1e-4));
    } else if ((particle_flags(p) & FLAG_SPECK) != 0u) {
        // Ambient motes passing near the eye stay specks, a little fainter: at their own size
        // the embers over the lava swelled into soft discs across a low camera's view.
        let widest = max(SPECK_SHARE * frame.screen.y, thinnest);
        let k = widest / (size * ppu);
        if (k < 1.0) {
            size = widest / max(ppu, 1e-4);
            fade = 0.6 + 0.4 * k;
        }
    }

    var across = right;
    var along = up;
    var half = vec2<f32>(0.5 * size);
    var centre = p.pos;
    var look = LOOK_GLOW;
    // Streaks: the particle trails behind itself along its flight as seen on screen, over the
    // distance it flies in `blur` seconds (at most `longest`), dimmer for its length (the same
    // light smeared further).
    var blur = 0.0;
    var longest = 0.0;
    if (kind == KIND_SPARK) {
        look = LOOK_STREAK;
        blur = STREAK_SECONDS;
        longest = STREAK_MAX;
    } else if (kind == KIND_SPRAY) {
        look = LOOK_DROPLET;
        blur = DROPLET_SECONDS;
        longest = 14.0;
        half.x *= 0.6;
    } else if (kind == KIND_DIRT) {
        look = LOOK_CLOD;
        blur = CLOD_SECONDS;
        longest = size * 2.0;
    } else if (kind == KIND_RAIN) {
        look = LOOK_RAIN;
        blur = DROPLET_SECONDS;
        longest = RAIN_LONGEST_PIXELS / max(ppu, 1e-4);
        // Drops falling past the eye are not drawn: up close they would be bars across it.
        if (frame.camera_pos.w > 0.5) {
            fade *= smoothstep(RAIN_NEAR.x, RAIN_NEAR.y, distance(p.pos, frame.camera_pos.xyz));
        }
    }
    if (blur > 0.0) {
        let v = p.vel - eye * dot(p.vel, eye);
        let speed = length(v);
        let len = min(speed * blur, longest);
        if (kind == KIND_RAIN) {
            // Seen from above, a drop falling straight at the eye is a dot: faint rather than a
            // blob, the rain then showing as fine specks.
            fade *= smoothstep(1.0, 5.0, len / max(size, 1e-3));
        }
        if (speed > 1e-3) {
            along = v / speed;
            across = normalize(cross(along, eye));
        }
        half.y += 0.5 * len;
        centre -= along * (0.5 * len);
        fade *= mix(1.0, size / (size + len), 0.5);
    }
    if (kind == KIND_FIRE || kind == KIND_DUST || kind == KIND_SMOKE || kind == KIND_FIREBALL || (kind == KIND_DIRT && dot(p.vel, p.vel) < 1.0)) {
        // Billows (and grit at rest) turn slowly, each its own way. Dust and smoke slide along
        // the ground they touch: their bulk billows above that point rather than half under the
        // ground.
        if (kind != KIND_DIRT) {
            look = select(LOOK_BILLOW, LOOK_FIREBALL, kind == KIND_FIREBALL);
        }
        if (kind == KIND_DUST || kind == KIND_SMOKE) {
            centre.y += 0.35 * size;
        }
        let spin = select(-0.3, 0.3, (seed & 1u) == 0u) * select(1.0, 0.25, kind == KIND_FIREBALL);
        let angle = f32(seed) * 0.0246 + p.age * spin;
        let cs = cos(angle);
        let sn = sin(angle);
        across = right * cs + up * sn;
        along = up * cs - right * sn;
    }
    let world = centre + across * (corner.x * half.x) + along * (corner.y * half.y);
    out.clip = frame.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    // Depth of the centre (a streak's head) nudged towards the camera, flattened like the
    // sprites.
    let head = select(centre, p.pos, blur > 0.0);
    let anchor = frame.view_proj * vec4<f32>(head + eye * min(size * 0.6, 6.0), 1.0);
    out.clip.z = anchor.z / anchor.w * out.clip.w;

    // Fog of war and distance haze: additive light simply dims; lit particles are hazed like
    // every other surface (`apply_fog`), split so the fragments can shade billows in between.
    let seen = los_light(world);
    let hazed = fog_factor(world);
    var soft = 0.0;
    if (kind == KIND_SPARK && (particle_flags(p) & FLAG_COLD) != 0u) {
        // Cold sparks (electricity, energy beams): a white flash settling to their own colour,
        // then fading out.
        let c = mix(vec3<f32>(1.0), tint.rgb, smoothstep(0.0, 0.25, t));
        out.color = vec4<f32>(c * brightness * (1.0 - t) * (1.0 - smoothstep(0.85, 1.0, t)), 0.0);
    } else if (kind == KIND_SPARK) {
        // Cooling: a flash of white-hot, then yellow, orange and deep red as it dies; some
        // hotter than others.
        let heat = (0.8 - 0.7 * pow(t, 0.8)) * mix(0.82, 1.08, particle_random(seed, 2u)) + 0.2 * max(1.0 - t * 12.0, 0.0);
        out.color = vec4<f32>(incandescence(heat) * tint.rgb * brightness * (1.0 - smoothstep(0.85, 1.0, t)), 0.0);
    } else if (kind == KIND_EMBER) {
        let flicker = 0.72 + 0.28 * sin(params.step.y * (6.0 + f32(seed & 7u)) + f32(seed));
        let heat = (0.62 - 0.5 * t) * flicker;
        out.color = vec4<f32>(incandescence(heat) * tint.rgb * brightness * smoothstep(0.0, 0.05, t), 0.0);
    } else if (kind == KIND_GLOW) {
        out.color = vec4<f32>(tint.rgb * brightness * (1.0 - t) * (1.0 - t), 0.0);
        soft = 0.5 * size;
    } else if (kind == KIND_FIRE) {
        let k = smoothstep(0.0, 0.08, t) * (1.0 - t);
        out.color = vec4<f32>(incandescence(0.68 - 0.45 * t) * tint.rgb * brightness * k, 0.0);
        soft = 0.5 * size;
    } else if (kind == KIND_RAIN) {
        // A drop catches the light of the sky around it: faint, pale and cool, some drops more
        // than others.
        let sky = mix(frame.fog.rgb, frame.ambient.rgb * 1.5, 0.5) * mix(0.55, 1.3, particle_random(seed, 4u));
        out.color = vec4<f32>(sky * tint.rgb * brightness * (1.0 - smoothstep(0.85, 1.0, t)), 0.0);
    } else if (kind == KIND_FIREBALL) {
        // Its light, its soot as lit here, the haze in front of it; how much of it shows (a).
        out.color = vec4<f32>(tint.rgb * brightness * seen * (1.0 - hazed), fade);
        out.soot = SOOT * billow_light(world) * seen * (1.0 - hazed);
        out.haze = fog_color_at(world) * hazed;
        soft = 0.35 * size;
    } else {
        var light = billow_light(world);
        var a = tint.a;
        var albedo = tint.rgb;
        if (kind == KIND_DUST || kind == KIND_SMOKE) {
            a *= smoothstep(0.0, 0.1, t) * pow(1.0 - t, 1.3);
            soft = params.step.w;
        } else if (kind == KIND_DIRT) {
            // Clods of one burst in shades of the ground's colour.
            albedo *= mix(0.72, 1.15, particle_random(seed, 3u));
            a *= 1.0 - smoothstep(0.7, 1.0, t);
        } else {
            // Spray catches the light.
            a *= 1.0 - t;
            light = light * 1.15 + vec3<f32>(0.08);
        }
        out.color = vec4<f32>(albedo * light * seen * (1.0 - hazed), a * fade);
        out.haze = fog_color_at(world) * hazed;
    }
    if (kind_additive(kind) && kind != KIND_FIREBALL) {
        out.color = vec4<f32>(out.color.rgb * (fade * seen * (1.0 - hazed)), 0.0);
        if (kind == KIND_FIRE && frame.camera_pos.w > 0.5) {
            // Flames hide some of what burns behind them (perspective camera): a fireball over
            // pale sand stays orange instead of adding up to white.
            out.color.a = 0.75 * smoothstep(0.0, 0.08, t) * (1.0 - t) * fade * (1.0 - hazed);
        }
    }
    if (soft > 0.0) {
        out.above = world.y - ground_height(world.xz);
    }
    let shape = select(f32(seed & 3u), f32(seed), kind == KIND_FIREBALL);
    out.look = vec4<f32>(f32(look), shape, soft, t);
    return out;
}

// Four vertices per particle (see the index buffer in gpu_particles.rs): additive particles
// from the front of the buffer, alpha-blended ones from its back.
@vertex
fn vs_particle_additive(@builtin(vertex_index) vi: u32) -> VOut {
    var index = vi >> 2u;
    if (draw_counts.words[5] > 0u) { index = draw_order[index]; }
    return particle_vertex(particles[index], vi & 3u);
}

@vertex
fn vs_particle_alpha(@builtin(vertex_index) vi: u32) -> VOut {
    return particle_vertex(particles[draw_order[params.counts.x - 1u - (vi >> 2u)]], vi & 3u);
}

// How much of the particle covers this point of its quad (x, 0..1), and a billow's density
// there (y). `gx`, `gy` = screen derivatives of the billow texture coordinates.
fn coverage(in: VOut, gx: vec2<f32>, gy: vec2<f32>) -> vec2<f32> {
    let look = u32(in.look.x + 0.5);
    let d2 = dot(in.uv, in.uv);
    if (look == LOOK_BILLOW) {
        let billow = textureSampleGrad(billows, billow_sampler, billow_uv(in.uv, in.look.y), gx, gy).r;
        // Masked to the disc too: a far (tiny) billow reads a coarse mip level.
        return vec2<f32>(min(billow * billow * saturate(1.0 - d2) * 1.3, 1.0), billow);
    }
    if (look == LOOK_CLOD) {
        // A clod of earth: the dense heart of a billow shape, so each is lumpy in its own way,
        // with an edge just soft enough not to flicker.
        let billow = textureSampleGrad(billows, billow_sampler, billow_uv(in.uv * 0.92, in.look.y), gx * 0.92, gy * 0.92).r;
        let edge = max(fwidth(billow), 0.04);
        return vec2<f32>(smoothstep(0.34 - edge, 0.34 + edge, billow * saturate(1.3 - d2)), billow);
    }
    let glow = saturate(1.0 - d2);
    if (look == LOOK_RAIN) {
        // A fine line tapering away at both ends.
        let along = in.uv.y * 0.5 + 0.5;
        return vec2<f32>(saturate(1.0 - in.uv.x * in.uv.x) * smoothstep(0.0, 0.4, along) * (1.0 - smoothstep(0.75, 1.0, along)), 0.0);
    }
    if (look == LOOK_STREAK || look == LOOK_DROPLET) {
        // Brightest towards the head, a tail behind.
        return vec2<f32>(glow * glow * mix(0.35, 1.0, saturate(in.uv.y * 0.5 + 0.5)), 0.0);
    }
    return vec2<f32>(glow * glow, 0.0);
}

// Soft particles: fading out over `look.z` above the ground instead of being cut by it.
fn ground_fade(in: VOut) -> f32 {
    if (in.look.z <= 0.0) {
        return 1.0;
    }
    return smoothstep(0.0, in.look.z, in.above);
}

// An additive particle's light (rgb) and how much of what lies behind it hides (a).
fn additive_color(in: VOut) -> vec4<f32> {
    // (Derivatives first, while every pixel of the quad takes the same path.)
    let buv = billow_uv(in.uv, in.look.y);
    let gx = dpdx(buv);
    let gy = dpdy(buv);
    let span = max(fwidth(in.uv.x), fwidth(in.uv.y));
    if (u32(in.look.x + 0.5) == LOOK_FIREBALL) {
        let f = fireball_look(in.uv, in.look.w, in.look.y, 2.0 / max(span, 1e-4));
        let k = f.gas * ground_fade(in) * in.color.a;
        return vec4<f32>((f.glow * in.color.rgb + in.soot * f.soot + in.haze) * k, k);
    }
    let k = coverage(in, gx, gy).x * ground_fade(in);
    return vec4<f32>(in.color.rgb * k, in.color.a * k);
}

// An alpha-blended particle's colour, premultiplied like the smoke sprites.
fn alpha_color(in: VOut) -> vec4<f32> {
    let buv = billow_uv(in.uv, in.look.y);
    let cover = coverage(in, dpdx(buv), dpdy(buv));
    let a = in.color.a * cover.x * ground_fade(in);
    var c = in.color.rgb;
    let look = u32(in.look.x + 0.5);
    if (look == LOOK_BILLOW) {
        // Dense parts of a billow shade themselves: darker cores, lighter fringes.
        c *= mix(1.15, 0.7, cover.y);
    } else if (look == LOOK_CLOD) {
        // A lump: its middle catches the light, its rim falls into shade.
        c *= mix(0.62, 1.18, saturate((cover.y - 0.3) * 2.5));
    }
    return vec4<f32>((c + in.haze) * a, a);
}

@fragment
fn fs_particle_additive(in: VOut) -> @location(0) vec4<f32> {
    let c = additive_color(in);
    if (all(c <= vec4<f32>(0.0))) {
        discard;
    }
    return vec4<f32>(c.rgb * select(1.0, 0.2, params.wind.z > 0.5), c.a);
}

@fragment
fn fs_particle_alpha(in: VOut) -> @location(0) vec4<f32> {
    let c = alpha_color(in);
    if (c.a <= 0.002) {
        discard;
    }
    return c;
}

// ---- Mirrored in the water (the reflection pass, see reflection.rs) ------------------------
// What flies above the water, where the water can show it; the frame's billboard basis is
// turned over for this pass, so the quads face the mirrored eye.

@fragment
fn fs_particle_additive_reflection(in: VOut) -> @location(0) vec4<f32> {
    let c = additive_color(in);
    // (Rain mirrored in the sea it falls into would only blur it.)
    if (all(c <= vec4<f32>(0.0)) || u32(in.look.x + 0.5) == LOOK_RAIN || !seen_in_water(in.world)) {
        discard;
    }
    return vec4<f32>(c.rgb * select(1.0, 0.2, params.wind.z > 0.5), c.a);
}

@fragment
fn fs_particle_alpha_reflection(in: VOut) -> @location(0) vec4<f32> {
    let c = alpha_color(in);
    if (c.a <= 0.002 || !seen_in_water(in.world)) {
        discard;
    }
    return c;
}
