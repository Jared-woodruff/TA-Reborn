// Final composite: heat haze, sharpening, exposure, bloom, colour grading, tone mapping,
// vignette.
// Prepended: fullscreen.wgsl, noise.wgsl.

struct PostParams {
    // x = exposure, y = bloom strength, z = vignette, w = saturation
    grade: vec4<f32>,
    // x = 1 / width, y = 1 / height, z = time, w = the cinematic grade's strength (0 = TA's
    // colours exactly: the classic view)
    info: vec4<f32>,
    // x = heat haze distortion in pixels per unit of heat (0 = off), y = sharpening (0 = none),
    // z = the intro room's tone curve's toe (0: not the room), w = its shoulder
    heat: vec4<f32>,
    // x = 1 for space's look, y = its dither (8-bit steps, peak to peak), z = its fade (the
    // picture's sRGB values times it), w = 1 when the scene is drawn smaller than the picture
    look: vec4<f32>,
    // Photo mode's look: x = contrast, y = warmth, z = grain, w = colour fringe
    photo: vec4<f32>,
    // x = monochrome, y = black level lifted, z = special (1 thermal, 2 night sight), w = 1 on
    photo_tone: vec4<f32>,
    // The shadows' and the highlights' tints: rgb, and how much
    shadows: vec4<f32>,
    highlights: vec4<f32>,
};

@group(0) @binding(0) var<uniform> post: PostParams;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var lin: sampler;

// Identity below 0.8 so the original palette art keeps its exact colours; a smooth shoulder
// above that absorbs HDR highlights (explosions, lasers, bloom) without harsh clipping.
fn shoulder(x: f32) -> f32 {
    let k = 0.8;
    if (x <= k) {
        return x;
    }
    return k + (1.0 - k) * (1.0 - exp(-(x - k) / (1.0 - k)));
}

// `film` (0..1, the perspective camera's grade): how far the brightest light behaves as on film.
fn tonemap(c: vec3<f32>, film: f32) -> vec3<f32> {
    let peak = max(max(c.r, c.g), c.b);
    if (peak <= 0.8) {
        return c;
    }
    // Map the peak channel through the shoulder and scale the others to preserve hue, then
    // desaturate slightly towards white for very bright values.
    let mapped = shoulder(peak);
    let hue_preserved = c * (mapped / peak);
    let whiteness = clamp((peak - 1.0) / 6.0, 0.0, 1.0);
    let classic = mix(hue_preserved, vec3<f32>(mapped), whiteness * 0.6);
    if (film <= 0.0) {
        return classic;
    }
    // On film each channel saturates on its own, so the hottest light burns through orange
    // and yellow to white, as fire does (keeping its hue would leave it salmon pink).
    let per_channel = vec3<f32>(shoulder(c.r), shoulder(c.g), shoulder(c.b));
    return mix(classic, per_channel, min(film, 1.0) * smoothstep(0.8, 2.5, peak));
}

// Heat haze (see heat.rs): the scene target's alpha is 1 on geometry plus the heat of the hot
// air in front of it, and strongly negative on the sky. Where there is heat, the image is
// looked up a few pixels off along drifting, rising noise; a lookup that lands on the sky
// keeps the undistorted pixel, so the sky (and the UI, drawn later) never shimmer.
fn heat_haze(uv: vec2<f32>, base: vec4<f32>) -> vec3<f32> {
    let heat = clamp(base.a - 1.0, 0.0, 2.0);
    if (heat < 0.003) {
        return base.rgb;
    }
    let px = uv / post.info.xy;
    let t = post.info.z;
    // Two octaves of gradient noise rising up the screen (y grows downwards).
    let p = px / 42.0 + vec2<f32>(0.0, t * 1.3);
    let q = px / 17.0 + vec2<f32>(5.2, t * 2.9);
    let n = vec2<f32>(noise(p) + 0.45 * noise(q + vec2<f32>(3.7, 1.1)), noise(p + vec2<f32>(11.3, 7.9)) + 0.45 * noise(q + vec2<f32>(-4.1, 9.6)));
    let offset = n * (heat * post.heat.x) * post.info.xy;
    let s = textureSampleLevel(hdr, lin, uv + offset, 0.0);
    return mix(base.rgb, s.rgb, saturate(s.a));
}

// The scene at `uv` when it is drawn smaller than the picture (a render scale under 1): through a
// Catmull-Rom cubic, which keeps the edges and the fine detail a bilinear lookup smears over the
// two or so pixels each texel covers, in five bilinear lookups (its sixteen texels with the
// corners, which weigh least, left out), then kept within the four texels around it: the
// cubic's overshoot would ring dark round the brightest light (a lamp, a muzzle flash, the sun
// on the water). The alpha (the heat over geometry, the sky's marker) as a bilinear lookup's.
fn upscaled(uv: vec2<f32>) -> vec4<f32> {
    let texel = post.info.xy;
    let p = uv / texel;
    let centre = floor(p - 0.5) + 0.5;
    let f = p - centre;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let at0 = (centre - 1.0) * texel;
    let at12 = (centre + w2 / w12) * texel;
    let at3 = (centre + 2.0) * texel;
    let c = textureSampleLevel(hdr, lin, vec2<f32>(at12.x, at0.y), 0.0).rgb * (w12.x * w0.y)
        + textureSampleLevel(hdr, lin, vec2<f32>(at0.x, at12.y), 0.0).rgb * (w0.x * w12.y)
        + textureSampleLevel(hdr, lin, at12, 0.0).rgb * (w12.x * w12.y)
        + textureSampleLevel(hdr, lin, vec2<f32>(at3.x, at12.y), 0.0).rgb * (w3.x * w12.y)
        + textureSampleLevel(hdr, lin, vec2<f32>(at12.x, at3.y), 0.0).rgb * (w12.x * w3.y);
    let sum = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    let last = vec2<i32>(textureDimensions(hdr)) - 1;
    let i = vec2<i32>(centre - 0.5);
    let a = textureLoad(hdr, clamp(i, vec2<i32>(0), last), 0);
    let b = textureLoad(hdr, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last), 0);
    let d = textureLoad(hdr, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last), 0);
    let e = textureLoad(hdr, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last), 0);
    let lo = min(min(a.rgb, b.rgb), min(d.rgb, e.rgb));
    let hi = max(max(a.rgb, b.rgb), max(d.rgb, e.rgb));
    let alpha = mix(mix(a.a, b.a, f.x), mix(d.a, e.a, f.x), f.y);
    return vec4<f32>(clamp(c / sum, lo, hi), alpha);
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// Sharpening (strength `k`): the pixel pushed away from the mean of its four neighbours, by
// less where they differ much (edges keep their antialiasing, flat art gains crispness).
fn sharpen(uv: vec2<f32>, c: vec3<f32>, k: f32) -> vec3<f32> {
    let d = post.info.xy;
    let n = textureSampleLevel(hdr, lin, uv + vec2<f32>(0.0, -d.y), 0.0).rgb;
    let s = textureSampleLevel(hdr, lin, uv + vec2<f32>(0.0, d.y), 0.0).rgb;
    let e = textureSampleLevel(hdr, lin, uv + vec2<f32>(d.x, 0.0), 0.0).rgb;
    let w = textureSampleLevel(hdr, lin, uv + vec2<f32>(-d.x, 0.0), 0.0).rgb;
    let mean = (n + s + e + w) * 0.25;
    let lo = min(min(min(n, s), min(e, w)), c);
    let hi = max(max(max(n, s), max(e, w)), c);
    // Contrast-adaptive, as AMD's CAS: full on gentle detail, little on hard edges.
    let range = luma(hi) - luma(lo);
    let amount = k * (1.0 - smoothstep(0.08, 0.6, range / max(luma(hi), 0.05)));
    return clamp(c + (c - mean) * amount, lo, hi);
}

// The perspective camera's colour grade (strength `k`; 0 = TA's colours exactly): a gentle
// filmic contrast around middle grey (darks deeper, lights brighter), vibrance (dull colours
// gain more saturation than vivid ones), and a hint of cool shadows and warm highlights.
// The contrast eases off again towards the highlights, which keep about their own value: lit
// sand and snow, near white as TA painted them, were pushed into the shoulder (and the warm
// tint's red with them), where their grain and relief flattened out into a blank cream.
fn cinematic(c: vec3<f32>, k: f32) -> vec3<f32> {
    let l = max(luma(c), 1e-5);
    let mid = 0.18;
    let expanded = mid * pow(l / mid, 1.0 + 0.12 * k);
    var out = c * (mix(expanded, l, smoothstep(0.3, 0.9, l)) / l);
    let mx = max(max(out.r, out.g), out.b);
    let mn = min(min(out.r, out.g), out.b);
    let sat = (mx - mn) / max(mx, 1e-4);
    let l2 = luma(out);
    out = mix(vec3<f32>(l2), out, 1.0 + 0.3 * k * (1.0 - sat));
    let t = smoothstep(0.02, 0.45, l2);
    let tone = mix(vec3<f32>(0.955, 0.995, 1.065), vec3<f32>(1.02, 0.995, 0.935), t);
    return max(out * mix(vec3<f32>(1.0), tone, k * 0.55), vec3<f32>(0.0));
}

// The intro room's picture, as through a camera's lens at night: its colours fringe a little
// towards the corners, a filmic curve rolls the tube's glare off softly and keeps the dark
// readable (a gentle toe: the room is mostly shadow), the corners darken, and the film's grain
// shows.
fn room_look(uv: vec2<f32>, base: vec3<f32>) -> vec3<f32> {
    let d = uv - 0.5;
    let shift = d * dot(d, d) * 0.005;
    let r = textureSampleLevel(hdr, lin, uv + shift, 0.0).r;
    let b = textureSampleLevel(hdr, lin, uv - shift, 0.0).b;
    var c = vec3<f32>(r, base.g, b) * post.grade.x;
    c += textureSampleLevel(bloom, lin, uv, 0.0).rgb * post.grade.y;
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3<f32>(l), c, post.grade.w), vec3<f32>(0.0));
    let toe = pow(c, vec3<f32>(post.heat.z));
    c = toe / (toe + post.heat.w);
    // Night's split: the shadows cool towards the moonlight, the lit warm towards the lamps.
    let lit = smoothstep(0.04, 0.55, dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)));
    c *= mix(vec3<f32>(0.9, 0.97, 1.1), vec3<f32>(1.04, 1.0, 0.95), lit);
    let aspect = post.info.y / post.info.x;
    let corner = length(d * vec2<f32>(aspect, 1.0)) / length(vec2<f32>(aspect * 0.5, 0.5));
    c *= 1.0 - (0.34 + post.grade.z) * smoothstep(0.3, 1.05, corner);
    // Grain: a fresh speckle every frame, as film's, the same share of the light at every
    // brightness (and a whisper in the black).
    let px = floor(uv / post.info.xy);
    var h = vec3<u32>(vec2<u32>(px), u32(post.info.z * 60.0));
    h = h * 1664525u + 1013904223u;
    h.x += h.y * h.z;
    h.y += h.z * h.x;
    h.z += h.x * h.y;
    h ^= h >> vec3<u32>(16u);
    h.x += h.y * h.z;
    let n = f32(h.x & 0xffffu) / 65535.0 - 0.5;
    c = c * (1.0 + n * 0.16) + vec3<f32>(n * 0.0015);
    return max(c, vec3<f32>(0.0));
}

// Space's filmic curve (Lottes's): a gentle toe that keeps the faint nebulae and stars, middle
// grey a little lifted, a long shoulder towards white.
fn space_curve(x: vec3<f32>) -> vec3<f32> {
    let a = 1.25;
    let d = 0.975;
    let hdr_max = 24.0;
    let mid_in = 0.18;
    let mid_out = 0.22;
    let ma = pow(hdr_max, a);
    let mad = pow(hdr_max, a * d);
    let ia = pow(mid_in, a);
    let iad = pow(mid_in, a * d);
    let b = (-ia + ma * mid_out) / ((mad - iad) * mid_out);
    let c = (mad * ia - ma * iad * mid_out) / ((mad - iad) * mid_out);
    let xa = pow(max(x, vec3<f32>(0.0)), vec3<f32>(a));
    return xa / (pow(max(x, vec3<f32>(0.0)), vec3<f32>(a * d)) * b + c);
}

// A hash of a pixel and a frame: 0 to 1.
fn dither_hash(px: vec2<u32>, n: u32) -> f32 {
    var h = vec3<u32>(px, n);
    h = h * 1664525u + 1013904223u;
    h.x += h.y * h.z;
    h.y += h.z * h.x;
    h.z += h.x * h.y;
    h ^= h >> vec3<u32>(16u);
    h.x += h.y * h.z;
    return f32(h.x & 0xffffu) / 65535.0;
}

// Space's picture: its exposure and glow, the filmic curve (each channel through it where bright,
// so the hottest light burns to white; its hue kept below), the corners darkened a little, faded
// as a video fades (its sRGB values scaled), and dithered: triangular noise of a step of the 8-bit
// picture, added where it is stored (sRGB), so the dark sky's gradients do not band.
fn space_look(uv: vec2<f32>, base: vec3<f32>, frag: vec2<f32>) -> vec3<f32> {
    var c = base * post.grade.x + textureSampleLevel(bloom, lin, uv, 0.0).rgb * post.grade.y;
    let peak = max(max(c.r, c.g), max(c.b, 1e-6));
    let hue = c * (space_curve(vec3<f32>(peak)).x / peak);
    let burn = space_curve(c);
    c = mix(hue, burn, smoothstep(0.35, 1.0, space_curve(vec3<f32>(peak)).x));
    let aspect = post.info.y / post.info.x;
    let d = uv - 0.5;
    let corner = length(d * vec2<f32>(aspect, 1.0)) / length(vec2<f32>(aspect * 0.5, 0.5));
    c *= 1.0 - post.grade.z * smoothstep(0.45, 1.1, corner);
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let n = u32(post.info.z * 60.0 + 0.5);
    let px = vec2<u32>(frag);
    let noise = (dither_hash(px, n) + dither_hash(px, n + 7919u) - 1.0) * post.look.y / 255.0;
    // Into sRGB, the noise added, and back: the swapchain's sRGB view stores it so.
    let s = select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308)) * post.look.z;
    let t = clamp(s + vec3<f32>(noise), vec3<f32>(0.0), vec3<f32>(1.0));
    return select(pow((t + 0.055) / 1.055, vec3<f32>(2.4)), t / 12.92, t <= vec3<f32>(0.04045));
}

// A colour fringe, as a lens's towards its corners: the red looked up a little outwards, the
// blue a little inwards (strength `k`).
fn fringe(uv: vec2<f32>, c: vec3<f32>, k: f32) -> vec3<f32> {
    let d = uv - 0.5;
    let shift = d * length(d) * 0.028 * k;
    let r = textureSampleLevel(hdr, lin, uv + shift, 0.0).r;
    let b = textureSampleLevel(hdr, lin, uv - shift, 0.0).b;
    return vec3<f32>(r, c.g, b);
}

// A thermal camera's palette: cold blacks and blues through purple and red to white-hot.
fn thermal(t: f32) -> vec3<f32> {
    var stops = array<vec3<f32>, 6>(
        vec3<f32>(0.02, 0.02, 0.09),
        vec3<f32>(0.20, 0.04, 0.50),
        vec3<f32>(0.70, 0.06, 0.45),
        vec3<f32>(0.97, 0.33, 0.06),
        vec3<f32>(1.00, 0.80, 0.16),
        vec3<f32>(1.00, 1.00, 0.92),
    );
    let x = clamp(t, 0.0, 1.0) * 5.0;
    let i = min(u32(floor(x)), 4u);
    return mix(stops[i], stops[i + 1u], x - f32(i));
}

// Photo mode's look over the finished picture (`c`, linear; `marker`, the scene's alpha: the
// sky's below 0, hot air's over 1), in its display values: the black level and the contrast, black and white, the warmth, the tints of the
// shadows and of the highlights (their colour only: the brightness kept, the middle tones taking
// some of each); or a special sight in their place; then the grain, a fresh speckle every frame.
fn photo_look(c: vec3<f32>, frag: vec2<f32>, marker: f32) -> vec3<f32> {
    var g = pow(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    let special = post.photo_tone.z;
    if (special > 1.5) {
        // Night sight: brightness as a green phosphor's (its brightest whiter), lifted from the
        // dark, every other line a little dimmer.
        let l = clamp(pow(luma(g), 0.85) * 1.1, 0.0, 1.0);
        let scan = 1.0 - 0.16 * pow(sin(frag.y * 1.5707963), 2.0);
        g = (vec3<f32>(0.3, 1.0, 0.38) * l + vec3<f32>(0.35, 0.0, 0.3) * l * l * l + vec3<f32>(0.0, 0.02, 0.0)) * scan;
    } else if (special > 0.5) {
        // Thermal: the land cool, what is bright (lit metal, fires, a blast) hot, and hotter
        // still where the air shimmers; the sky colder than anything.
        let heat = pow(luma(g), 1.7) + 0.6 * clamp(marker - 1.0, 0.0, 1.0);
        g = thermal(select(heat, 0.1 * luma(g), marker < 0.0));
    } else {
        g = mix(vec3<f32>(post.photo_tone.y), vec3<f32>(1.0), g);
        g = (g - 0.5) * post.photo.x + 0.5;
        g = mix(g, vec3<f32>(luma(g)), post.photo_tone.x);
        let w = post.photo.y;
        g = g * vec3<f32>(1.0 + 0.14 * w, 1.0 + 0.02 * w, 1.0 - 0.14 * w);
        let l = luma(g);
        let lows = 1.0 - smoothstep(0.0, 0.75, l);
        let highs = smoothstep(0.25, 1.0, l);
        g += (post.shadows.rgb - vec3<f32>(luma(post.shadows.rgb))) * lows * post.shadows.w;
        g += (post.highlights.rgb - vec3<f32>(luma(post.highlights.rgb))) * highs * post.highlights.w;
    }
    // (Heavier in the middle tones, as film's.)
    let n = dither_hash(vec2<u32>(frag), u32(post.info.z * 60.0)) - 0.5;
    let middle = 1.0 - abs(luma(g) - 0.5);
    g += vec3<f32>(n * post.photo.z * 0.2 * middle);
    return pow(clamp(g, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    var scene = textureSampleLevel(hdr, lin, in.uv, 0.0);
    if (post.look.w > 0.5) {
        scene = upscaled(in.uv);
    }
    if (post.heat.z > 0.5) {
        return vec4<f32>(room_look(in.uv, scene.rgb), 1.0);
    }
    if (post.look.x > 0.5) {
        return vec4<f32>(space_look(in.uv, scene.rgb, in.pos.xy), 1.0);
    }
    var c = scene.rgb;
    if (post.heat.x > 0.0) {
        c = heat_haze(in.uv, scene);
    }
    if (post.heat.y > 0.0) {
        c = sharpen(in.uv, c, post.heat.y);
    }
    if (post.photo.w > 0.0) {
        c = fringe(in.uv, c, post.photo.w);
    }
    c = c * post.grade.x;
    c += textureSampleLevel(bloom, lin, in.uv, 0.0).rgb * post.grade.y;
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = max(mix(vec3<f32>(l), c, post.grade.w), vec3<f32>(0.0));
    if (post.info.w > 0.0) {
        c = cinematic(c, post.info.w);
    }
    c = tonemap(c, post.info.w);
    if (post.photo_tone.w > 0.5) {
        c = photo_look(c, in.pos.xy, scene.a);
    }
    let d = distance(in.uv, vec2<f32>(0.5));
    c *= 1.0 - post.grade.z * smoothstep(0.35, 0.85, d);
    return vec4<f32>(c, 1.0);
}
