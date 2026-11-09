// TA's worlds on a planet's face: Conquest's planets (`ta_game::planet`'s `ground`) as the GPU
// paints them. The recipes are planet.rs's own (its hash, gradient noise, octaves, colours, craters
// and cells), so a seed gives the same world as the galaxy map shows; past the largest size
// planet.rs paints (six octaves, some 64 pixels across) every recipe gains the octaves the size on
// screen shows (`WorldCtx::detail`), the last fading in by its fraction, and each noise is held
// to octaves no finer than about two pixels (`w_octaves`: planet.rs paints stills; in motion
// finer ones shimmer). Colours here are as
// planet.rs paints them (sRGB); the shading takes them to linear light. The clouds are a layer of
// their own (turning on their own), so `ground` gives their cover and the recipe's amount, and
// `relief` a height for the light to show the ground's shape by.

struct WorldCtx {
    seed: u32,
    // octaves of the continents' noise (frequency 1.9) the size on screen shows, the finest
    // about two pixels across (planet.rs's `detail`, here 1 to 13)
    detail: f32,
    // a pixel's width on the unit ball (planet.rs's `fine`)
    fine: f32,
};

struct Ground {
    albedo: vec3<f32>,
    shine: f32,
    glow: vec3<f32>,
    // the cloud cover's threshold (2: none) and the recipe's amount of it
    cover: f32,
    cloud_amount: f32,
    // 1 on the sea (flat, glinting)
    sea: f32,
    // how deep the sea is there (0 at the shore, 1 deep)
    deep: f32,
};

fn w_rotl(x: u32, k: u32) -> u32 {
    return (x << k) | (x >> (32u - k));
}

fn w_hash(a: u32, b: u32) -> u32 {
    var x = a ^ w_rotl(b * 0x9e3779b9u, 13u) ^ 0x85ebca6bu;
    x ^= x >> 16u;
    x *= 0x7feb352du;
    x ^= x >> 15u;
    x *= 0x846ca68bu;
    return x ^ (x >> 16u);
}

fn w_hash3(c: vec3<i32>, s: u32) -> u32 {
    return w_hash(w_hash(w_hash(s, bitcast<u32>(c.x)), bitcast<u32>(c.y)), bitcast<u32>(c.z));
}

fn w_unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

var<private> W_GRADS: array<vec3<f32>, 12> = array<vec3<f32>, 12>(
    vec3<f32>(1.0, 1.0, 0.0), vec3<f32>(-1.0, 1.0, 0.0), vec3<f32>(1.0, -1.0, 0.0), vec3<f32>(-1.0, -1.0, 0.0),
    vec3<f32>(1.0, 0.0, 1.0), vec3<f32>(-1.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, -1.0), vec3<f32>(-1.0, 0.0, -1.0),
    vec3<f32>(0.0, 1.0, 1.0), vec3<f32>(0.0, -1.0, 1.0), vec3<f32>(0.0, 1.0, -1.0), vec3<f32>(0.0, -1.0, -1.0),
);

fn w_corner(c: vec3<i32>, d: vec3<i32>, f: vec3<f32>, s: u32) -> f32 {
    return dot(W_GRADS[w_hash3(c + d, s) % 12u], f - vec3<f32>(d));
}

// planet.rs's `gradient`: Perlin noise, about -1 to 1.
fn w_gradient(p: vec3<f32>, s: u32) -> f32 {
    let i = floor(p);
    let f = p - i;
    let c = vec3<i32>(i);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x00 = mix(w_corner(c, vec3<i32>(0, 0, 0), f, s), w_corner(c, vec3<i32>(1, 0, 0), f, s), u.x);
    let x10 = mix(w_corner(c, vec3<i32>(0, 1, 0), f, s), w_corner(c, vec3<i32>(1, 1, 0), f, s), u.x);
    let x01 = mix(w_corner(c, vec3<i32>(0, 0, 1), f, s), w_corner(c, vec3<i32>(1, 0, 1), f, s), u.x);
    let x11 = mix(w_corner(c, vec3<i32>(0, 1, 1), f, s), w_corner(c, vec3<i32>(1, 1, 1), f, s), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// planet.rs's `fbm`, its octave count fractional: 0 to 1 round 0.5.
fn w_fbm(p: vec3<f32>, octaves: f32, s: u32) -> f32 {
    let n = max(octaves, 1.0);
    var sum = 0.0;
    var amp = 1.0;
    var f = 1.0;
    var total = 0.0;
    var o = 0u;
    loop {
        if (f32(o) >= n) {
            break;
        }
        let w = min(n - f32(o), 1.0);
        sum += amp * w * w_gradient(p * f, s + o * 0x9e37u);
        total += amp * w;
        amp *= 0.5;
        f *= 2.03;
        o += 1u;
    }
    return clamp(0.5 + 0.75 * sum / total, 0.0, 1.0);
}

// How many octaves of noise at frequency `f` (cycles across the ball) a recipe gets: planet.rs's
// `min(octaves, detail)` while the planet is as small as planet.rs paints them, those the size adds
// past that, and none finer than about two pixels whatever their frequency (in motion finer ones
// shimmer).
fn w_octaves(octaves: f32, f: f32, ctx: WorldCtx) -> f32 {
    let d = ctx.detail - log2(max(f, 1.9) / 1.9);
    return max(select(min(octaves, d), octaves + d - 6.0, d > 6.0), 1.0);
}

// planet.rs's `noise(p, octaves, salt)`, `p` at frequency `f`.
fn w_noise_f(p: vec3<f32>, f: f32, octaves: f32, salt: u32, ctx: WorldCtx) -> f32 {
    return w_fbm(p, w_octaves(octaves, f, ctx), ctx.seed ^ salt);
}

// planet.rs's `noise(at(f, o), octaves, salt)`.
fn w_noise_at(q: vec3<f32>, f: f32, o: f32, octaves: f32, salt: u32, ctx: WorldCtx) -> f32 {
    return w_noise_f(w_at(q, f, o), f, octaves, salt, ctx);
}

fn w_at(q: vec3<f32>, f: f32, o: f32) -> vec3<f32> {
    return vec3<f32>(q.x * f + o, q.y * f + o * 0.7, q.z * f - o * 1.3);
}

// 0 at `a`, 1 at `b` (either way round), smoothly between.
fn w_smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// planet.rs's clouds: big soft swirls, stretched along the lines of latitude.
fn w_clouds(q: vec3<f32>, cover: f32, ctx: WorldCtx) -> f32 {
    let p = vec3<f32>(q.x * 2.6, q.y * 4.7, q.z * 2.6);
    let w = w_noise_f(vec3<f32>(p.x + 5.1, p.y, p.z - 2.3), 4.7, 3.0, 0x55u, ctx);
    return w_smooth(cover, cover + 0.18, w_noise_f(vec3<f32>(p.x + w * 1.6, p.y + w, p.z), 4.7, 5.0, 0x77u, ctx)) * 0.85;
}

// A network of lines where the noise crosses its middle: 1 on them, 0 away.
fn w_lines(q: vec3<f32>, f: f32, o: f32, octaves: f32, salt: u32, width: f32, ctx: WorldCtx) -> f32 {
    let v = abs(w_noise_at(q, f, o, octaves, salt, ctx) - 0.5);
    return w_smooth(width + ctx.fine * 1.5, max(width - ctx.fine * 1.5, 0.0), v);
}

struct Cells {
    near: f32,
    second: f32,
    id: u32,
};

// Cellular noise: the distances to the nearest and second nearest points scattered one to a cell.
fn w_cells(p: vec3<f32>, s: u32) -> Cells {
    let base = vec3<i32>(floor(p));
    var out: Cells;
    out.near = 1e9;
    out.second = 1e9;
    out.id = 0u;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = base + vec3<i32>(dx, dy, dz);
                let h = w_hash3(c, s);
                let at = vec3<f32>(c) + vec3<f32>(w_unit(h), w_unit(w_hash(h, 1u)), w_unit(w_hash(h, 2u)));
                let d = length(at - p);
                if (d < out.near) {
                    out.second = out.near;
                    out.near = d;
                    out.id = h;
                } else if (d < out.second) {
                    out.second = d;
                }
            }
        }
    }
    return out;
}

// planet.rs's craters (a lattice `f` to the ball's radius, `amount` of its cells holding one at
// a point of their own): x their darkening, y their height (bowl down, rim up, in their own size).
// planet.rs takes the nearest point's crater only, which cuts a crater straight where another
// cell's point is nearer (unseen at its sizes, plain close up); here each of the cells round adds
// its own.
fn w_crater(q: vec3<f32>, s: u32, f: f32, amount: f32) -> vec2<f32> {
    let p = q * f;
    let base = vec3<i32>(floor(p));
    var k = 1.0;
    var height = 0.0;
    for (var dz = -1; dz <= 1; dz++) {
        for (var dy = -1; dy <= 1; dy++) {
            for (var dx = -1; dx <= 1; dx++) {
                let c = base + vec3<i32>(dx, dy, dz);
                let id = w_hash3(c, s);
                if (w_unit(w_hash(id, 3u)) > amount) {
                    continue;
                }
                let at = vec3<f32>(c) + vec3<f32>(w_unit(id), w_unit(w_hash(id, 1u)), w_unit(w_hash(id, 2u)));
                let d = length(at - p);
                let r = 0.18 + 0.22 * w_unit(w_hash(id, 5u));
                if (d >= r * 1.4) {
                    continue;
                }
                let floor_ = w_smooth(r, r * 0.6, d);
                let rim = w_smooth(r * 0.75, r, d) * w_smooth(r * 1.4, r, d);
                // (Each its own age: the old ones' floors and rims faded towards the ground round
                // them, as close up a field of them all alike reads as stamped.)
                let young = w_unit(w_hash(id, 9u));
                let age = 0.15 + 0.85 * young * young * young;
                k *= 1.0 - (floor_ * 0.25 - rim * 0.18) * age;
                height += (rim * 0.35 - floor_ * 0.6) * r / f;
            }
        }
    }
    return vec2<f32>(k, height);
}

// Craters past planet.rs's two lattices, smaller, as the size shows them (a crater some pixels
// across at least): their darkening and their height.
fn w_small_craters(q: vec3<f32>, s: u32, ctx: WorldCtx) -> vec2<f32> {
    var out = vec2<f32>(1.0, 0.0);
    var f = 20.0;
    var amount = 0.3;
    for (var k = 0u; k < 3u; k++) {
        // (A crater's radius is about 0.3 / f of the ball's: shown once it is a few pixels across.)
        let show = w_smooth(0.6, 1.4, 0.3 / f / max(ctx.fine * 4.0, 1e-7)) * 0.6;
        if (show <= 0.0) {
            break;
        }
        let c = w_crater(q, s ^ (0xb1u + k * 0x9du), f, amount);
        out = vec2<f32>(out.x * mix(1.0, c.x, show), out.y + c.y * show);
        f *= 2.3;
        amount *= 0.85;
    }
    return out;
}

fn plain(albedo: vec3<f32>) -> Ground {
    var g: Ground;
    g.albedo = albedo;
    g.shine = 0.0;
    g.glow = vec3<f32>(0.0);
    g.cover = 2.0;
    g.cloud_amount = 0.0;
    g.sea = 0.0;
    g.deep = 0.0;
    return g;
}

// What the world is at `q` (a point of the unit ball, the world's own axes): planet.rs's `ground`.
fn ground(world: u32, q: vec3<f32>, ctx: WorldCtx) -> Ground {
    let s = ctx.seed;
    switch world {
        // Green, Lush, Archipelago, Water, Acid.
        case 0u, 1u, 2u, 3u, 14u: {
            var sea = 0.50;
            var cover = 0.56;
            if (world == 1u) {
                sea = 0.42;
            } else if (world == 2u) {
                sea = 0.60;
                cover = 0.58;
            } else if (world == 3u) {
                sea = 0.67;
            } else if (world == 14u) {
                cover = 0.66;
            }
            let acid = world == 14u;
            let h = w_noise_at(q, 1.9, 0.0, 6.0, 0u, ctx);
            if (h < sea) {
                // The sea: deeper away from the shore.
                let deep = w_smooth(0.0, 0.12, sea - h);
                var g = plain(select(mix(vec3<f32>(0.16, 0.42, 0.56), vec3<f32>(0.05, 0.16, 0.36), deep), mix(vec3<f32>(0.70, 0.80, 0.20), vec3<f32>(0.42, 0.56, 0.08), deep), acid));
                g.shine = select(1.0, 0.5, acid);
                g.glow = select(vec3<f32>(0.0), vec3<f32>(0.06, 0.09, 0.0), acid);
                g.cover = cover;
                g.cloud_amount = 1.0;
                g.sea = 1.0;
                g.deep = deep;
                return g;
            }
            let up = w_smooth(sea, sea + 0.22, h);
            let wood = w_noise_at(q, 4.3, 7.0, 4.0, 0x13u, ctx);
            var albedo: vec3<f32>;
            if (acid) {
                albedo = mix(vec3<f32>(0.30, 0.31, 0.16), vec3<f32>(0.17, 0.16, 0.10), wood);
            } else {
                let lush = select(0.0, 0.25, world == 1u);
                let low = mix(vec3<f32>(0.31, 0.52, 0.22), vec3<f32>(0.14, 0.35, 0.16), clamp(wood - 0.15 + lush, 0.0, 1.0));
                let beach = w_smooth(0.025, 0.0, h - sea) * 0.7;
                let land = mix(low, vec3<f32>(0.47, 0.41, 0.29), clamp(up * 1.3 - 0.35 - lush, 0.0, 1.0));
                albedo = mix(land, vec3<f32>(0.72, 0.66, 0.48), beach);
            }
            // Snow on the peaks and towards the poles.
            var snow = 0.0;
            if (!acid) {
                snow = max(w_smooth(0.80, 0.90, abs(q.y) + (wood - 0.5) * 0.2), w_smooth(0.80, 0.86, h) * 0.8);
            }
            var g = plain(mix(albedo, vec3<f32>(0.90, 0.93, 0.96), snow));
            g.cover = cover;
            g.cloud_amount = 1.0;
            return g;
        }
        // Wet desert.
        case 4u: {
            let h = w_noise_at(q, 2.1, 0.0, 6.0, 0u, ctx);
            let rock = w_noise_at(q, 5.0, 3.0, 4.0, 0x21u, ctx);
            if (h < 0.42) {
                let deep = w_smooth(0.0, 0.1, 0.42 - h);
                var g = plain(mix(vec3<f32>(0.26, 0.48, 0.52), vec3<f32>(0.10, 0.26, 0.38), deep));
                g.shine = 0.9;
                g.cover = 0.68;
                g.cloud_amount = 1.0;
                g.sea = 1.0;
                g.deep = deep;
                return g;
            }
            let green = w_smooth(0.06, 0.0, h - 0.42) * 0.8;
            let sand = mix(vec3<f32>(0.78, 0.64, 0.42), vec3<f32>(0.56, 0.44, 0.30), w_smooth(0.45, 0.7, rock));
            var g = plain(mix(sand, vec3<f32>(0.36, 0.44, 0.24), green));
            g.cover = 0.68;
            g.cloud_amount = 1.0;
            return g;
        }
        // Desert: dunes in bands round the world, rock showing through.
        case 5u: {
            let w = w_noise_at(q, 2.4, 1.0, 4.0, 0x31u, ctx);
            let band = 0.5 + 0.5 * sin(q.y * 26.0 + w * 9.0);
            let rock = w_noise_at(q, 3.2, 4.0, 5.0, 0x33u, ctx);
            let sand = mix(vec3<f32>(0.86, 0.68, 0.42), vec3<f32>(0.72, 0.53, 0.31), band * 0.6);
            return plain(mix(sand, vec3<f32>(0.48, 0.34, 0.22), w_smooth(0.56, 0.70, rock)));
        }
        // Red.
        case 6u: {
            let h = w_noise_at(q, 2.0, 0.0, 6.0, 0u, ctx);
            var base = mix(vec3<f32>(0.68, 0.32, 0.19), vec3<f32>(0.40, 0.19, 0.13), w_smooth(0.42, 0.62, h));
            base = mix(base, vec3<f32>(0.82, 0.48, 0.30), w_smooth(0.62, 0.78, w_noise_at(q, 3.6, 2.0, 4.0, 0x41u, ctx)) * 0.6);
            base *= w_crater(q, s ^ 0x43u, 5.0, 0.55).x * w_small_craters(q, s ^ 0x45u, ctx).x;
            let pole = w_smooth(0.86, 0.93, abs(q.y) + (h - 0.5) * 0.15);
            return plain(mix(base, vec3<f32>(0.92, 0.86, 0.84), pole));
        }
        // Lava: a dark crust in plates, split by glowing rifts, melt pooled in the low ground.
        case 7u: {
            let crust = w_noise_at(q, 3.0, 0.0, 5.0, 0u, ctx);
            let albedo = mix(vec3<f32>(0.17, 0.14, 0.13), vec3<f32>(0.07, 0.06, 0.06), crust);
            let rift = w_lines(q, 1.7, 9.0, 4.0, 0x51u, 0.022, ctx);
            let near = w_lines(q, 1.7, 9.0, 4.0, 0x51u, 0.09, ctx) * 0.35;
            let pool = w_smooth(0.71, 0.77, w_noise_at(q, 1.4, 4.0, 3.0, 0x53u, ctx));
            let heat = max(rift, pool);
            let hot = mix(vec3<f32>(0.80, 0.20, 0.03), vec3<f32>(1.0, 0.70, 0.26), heat * heat);
            var g = plain(mix(albedo, vec3<f32>(0.30, 0.10, 0.04), heat));
            g.glow = vec3<f32>(hot.x * heat + 0.45 * near, hot.y * heat + 0.06 * near, hot.z * heat);
            return g;
        }
        // Metal and urban: plating in courses round the world, dark canyons, cities' lights.
        case 8u, 9u: {
            let urban = world == 9u;
            let lon = atan2(q.z, q.x);
            let lat = asin(clamp(q.y, -1.0, 1.0));
            let rows = 13.0;
            let v = (lat / PI + 0.5) * rows;
            let row = floor(v);
            let cols = max(round(26.0 * cos(lat)), 3.0);
            let u = (lon / (2.0 * PI) + 0.5) * cols + w_unit(w_hash(s ^ 0x61u, bitcast<u32>(i32(row))));
            let plate = w_hash(s ^ 0x62u, (bitcast<u32>(i32(floor(u))) * 131u) ^ bitcast<u32>(i32(row)));
            let fu = min(fract(u), 1.0 - fract(u));
            let fv = min(fract(v), 1.0 - fract(v));
            let seam = max(w_smooth(ctx.fine * cols / (2.0 * PI) / max(cos(lat), 0.2) * 1.1, 0.0, fu), w_smooth(ctx.fine * rows / PI * 1.1, 0.0, fv));
            let shade = 0.84 + 0.30 * w_unit(plate);
            let base = select(vec3<f32>(0.50, 0.52, 0.55), vec3<f32>(0.42, 0.46, 0.53), urban);
            let dark = select(vec3<f32>(0.22, 0.24, 0.27), vec3<f32>(0.18, 0.20, 0.25), urban);
            let canyon = w_lines(q, 1.6, 3.0, 4.0, 0x63u, 0.03, ctx);
            var g = plain(mix(base * shade, dark, max(seam * 0.45, canyon * 0.9)));
            let city = w_smooth(select(0.56, 0.42, urban), 0.68, w_noise_at(q, 2.2, 5.0, 3.0, 0x65u, ctx));
            let spark = w_smooth(0.62, 0.74, w_noise_at(q, 14.0, 1.0, 2.0, 0x67u, ctx));
            let lamp = city * spark;
            g.shine = 0.25;
            g.glow = vec3<f32>(1.0, 0.74, 0.40) * lamp;
            return g;
        }
        // Ice.
        case 10u: {
            let h = w_noise_at(q, 2.2, 0.0, 5.0, 0u, ctx);
            let crevasse = w_lines(q, 2.6, 6.0, 4.0, 0x71u, 0.02, ctx);
            let floe = w_smooth(0.40, 0.33, h);
            var albedo = mix(vec3<f32>(0.90, 0.94, 0.98), vec3<f32>(0.68, 0.80, 0.90), w_smooth(0.45, 0.65, h));
            albedo = mix(albedo, vec3<f32>(0.30, 0.48, 0.66), max(floe, crevasse * 0.7));
            var g = plain(albedo);
            g.shine = 0.35 + floe * 0.4;
            g.cover = 0.68;
            g.cloud_amount = 0.7;
            return g;
        }
        // Slate: grey-blue rock in folded bands, dark faults across them.
        case 11u: {
            let w = w_noise_at(q, 1.8, 0.0, 4.0, 0u, ctx);
            let band = 0.5 + 0.5 * sin(q.x * 7.0 + q.y * 11.0 + w * 8.0);
            var base = mix(vec3<f32>(0.46, 0.48, 0.54), vec3<f32>(0.29, 0.31, 0.37), w_smooth(0.4, 0.65, w_noise_at(q, 3.0, 5.0, 5.0, 0x81u, ctx)));
            base = mix(base, vec3<f32>(0.58, 0.61, 0.66), w_smooth(0.55, 0.95, band) * 0.5);
            let fault = w_lines(q, 2.4, 2.0, 4.0, 0x83u, 0.018, ctx);
            return plain(mix(base, vec3<f32>(0.14, 0.15, 0.18), fault * 0.75));
        }
        // Lunar: grey, its seas darker, cratered.
        case 12u: {
            let h = w_noise_at(q, 1.8, 0.0, 5.0, 0u, ctx);
            let base = mix(vec3<f32>(0.62, 0.62, 0.60), vec3<f32>(0.36, 0.36, 0.37), w_smooth(0.48, 0.60, h));
            let k = w_crater(q, s ^ 0x91u, 4.0, 0.7).x * w_crater(q, s ^ 0x93u, 9.0, 0.6).x * w_small_craters(q, s ^ 0x95u, ctx).x;
            return plain(base * k);
        }
        // Crystal: facets, each catching the light its own way, bright along their edges.
        case 13u: {
            let c = w_cells(q * 5.0, s ^ 0xa1u);
            let tone = w_unit(c.id);
            let face = 0.62 + 0.62 * w_unit(w_hash(c.id, 7u));
            let edge = w_smooth(0.05 + ctx.fine * 3.0, 0.0, c.second - c.near);
            let base = mix(vec3<f32>(0.50, 0.78, 0.92), vec3<f32>(0.64, 0.52, 0.88), tone);
            var g = plain(mix(base * face, vec3<f32>(0.92, 0.97, 1.0), edge * 0.45));
            g.shine = 0.6;
            g.glow = vec3<f32>(0.04 * tone, 0.07, 0.10);
            return g;
        }
        default: {
            return plain(vec3<f32>(0.5));
        }
    }
}

// The ground's height at `q` for its relief (0 to about 1; the sea flat).
fn relief(world: u32, q: vec3<f32>, ctx: WorldCtx) -> f32 {
    let s = ctx.seed;
    switch world {
        case 0u, 1u, 2u, 3u, 14u: {
            var sea = 0.50;
            if (world == 1u) {
                sea = 0.42;
            } else if (world == 2u) {
                sea = 0.60;
            } else if (world == 3u) {
                sea = 0.67;
            }
            return max(w_noise_at(q, 1.9, 0.0, 6.0, 0u, ctx) - sea, 0.0);
        }
        case 4u: {
            return max(w_noise_at(q, 2.1, 0.0, 6.0, 0u, ctx) - 0.42, 0.0);
        }
        case 5u: {
            return w_noise_at(q, 3.2, 4.0, 5.0, 0x33u, ctx) * 0.5;
        }
        case 6u: {
            return w_noise_at(q, 2.0, 0.0, 6.0, 0u, ctx) * 0.5 + w_crater(q, s ^ 0x43u, 5.0, 0.55).y * 4.0 + w_small_craters(q, s ^ 0x45u, ctx).y * 4.0;
        }
        case 7u: {
            return w_noise_at(q, 3.0, 0.0, 5.0, 0u, ctx) * 0.4 - w_lines(q, 1.7, 9.0, 4.0, 0x51u, 0.022, ctx) * 0.05;
        }
        case 8u, 9u: {
            return -w_lines(q, 1.6, 3.0, 4.0, 0x63u, 0.03, ctx) * 0.06;
        }
        case 10u: {
            return w_noise_at(q, 2.2, 0.0, 5.0, 0u, ctx) * 0.4;
        }
        case 11u: {
            return w_noise_at(q, 3.0, 5.0, 5.0, 0x81u, ctx) * 0.5 - w_lines(q, 2.4, 2.0, 4.0, 0x83u, 0.018, ctx) * 0.05;
        }
        case 12u: {
            let h = w_noise_at(q, 1.8, 0.0, 5.0, 0u, ctx);
            return h * 0.15 + (w_crater(q, s ^ 0x91u, 4.0, 0.7).y + w_crater(q, s ^ 0x93u, 9.0, 0.6).y + w_small_craters(q, s ^ 0x95u, ctx).y) * 2.5;
        }
        case 13u: {
            let c = w_cells(q * 5.0, s ^ 0xa1u);
            return -c.near * 0.06;
        }
        default: {
            return 0.0;
        }
    }
}
