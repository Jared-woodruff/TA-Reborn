// Procedural water shared by the water surface and the terrain shader (caustics on the sea
// bed, and the shoreline foam on the thin strip of painted water above sea level). Needs terrain_common.wgsl (art sampling) and noise.wgsl prepended. No textures are
// used.
//
// Waves: only slopes are needed, the surface is shaded, never displaced. They are a sum of
// directional swell waves (coherent travelling motion with deep-water dispersion, each coming
// in groups) plus four octaves of drifting gradient noise ("chop"), modulated by slowly
// drifting wind patches.
// Every component is band-limited: it fades out as its wavelength approaches the pixel
// footprint, and the slope variance removed that way is returned so the specular lobe can
// widen instead (a distant sea becomes a soft glitter path instead of shimmering noise). Where
// the waves' crests pile up is returned too: the whitecaps break there.

// ---- The liquid ---------------------------------------------------------------------------

// How acid the liquid is (0 water, 1 the acid of TA's acid worlds; see `Fluid::Acid`).
fn liquid_acid() -> f32 {
    return frame.liquid.x;
}

// The waves' clock at time `t`: acid moves more slowly, heavier than water.
fn liquid_time(t: f32) -> f32 {
    return t * mix(1.0, 0.6, liquid_acid());
}

// The liquid's own hue at a luminance of 1 (from its body colour, a little desaturated).
fn liquid_hue() -> vec3<f32> {
    let luma = vec3<f32>(0.2126, 0.7152, 0.0722);
    let c = frame.water_color.rgb;
    let h = clamp(mix(vec3<f32>(1.0), c / max(dot(c, luma), 1e-4), 0.85), vec3<f32>(0.15), vec3<f32>(2.0));
    return h / max(dot(h, luma), 1e-4);
}

// ---- Waves --------------------------------------------------------------------------------

struct Waves {
    // Surface slope: d(height)/dx, d(height)/dz.
    slope: vec2<f32>,
    // Slope variance removed by band limiting (widens the specular lobe).
    lost_var: f32,
    // How high the waves pile up here, weighed by their steepness (> 0 on crests; about -0.15
    // to 0.15, more where the crests of several meet): where they break into whitecaps.
    crest: f32,
    // The wind patches' strength here (`wind_gust`, 0.6..1.35).
    gust: f32,
};

// Fraction of a wave component that survives filtering at `footprint` world units per
// pixel: all of it at >= 8 pixels per wavelength, none at <= 3 (Nyquist is 2).
fn wave_band(wavelength: f32, footprint: f32) -> f32 {
    return smoothstep(3.0, 8.0, wavelength / max(footprint, 1e-4));
}

// One travelling sine wave: unit direction, wavelength (world units), steepness (slope
// amplitude) and phase offset (which also carries a slowly varying warp, so crests are not
// perfectly straight and distant glitter does not line up in rows). Deep-water dispersion
// ω = sqrt(g k), with g scaled so that the longest swell drifts at about 25 px/s. The wave
// comes in groups: its height swells and fades over a few wavelengths along it and a wavelength
// or two across it (its crests are short, as a wind sea's are), the groups travelling at half
// its speed (deep water's group velocity), so its crests come in sets instead of running on,
// all alike, into a lattice with the other swells' or into endless stripes to the horizon.
fn add_swell(w: ptr<function, Waves>, xz: vec2<f32>, t: f32, footprint: f32, dir: vec2<f32>, wavelength: f32, steepness0: f32, phase0: f32) {
    let k = TAU / wavelength;
    let omega = sqrt(23.0 * k);
    let b = wave_band(wavelength, footprint);
    var steepness = steepness0;
    if (b > 0.0) {
        let along = dot(dir, xz) - 0.5 * omega / k * t;
        let across = dir.x * xz.y - dir.y * xz.x;
        let group = noise(vec2<f32>(along / (wavelength * 3.0), across / (wavelength * 1.4)) + vec2<f32>(wavelength * 0.37, 7.7));
        steepness *= 0.3 + 1.4 * smoothstep(-0.25, 0.25, group);
    }
    let phase = dot(dir, xz) * k - omega * t + phase0;
    (*w).slope += dir * (steepness * b * cos(phase));
    (*w).lost_var += 0.5 * steepness * steepness * (1.0 - b * b);
    // (The wave's height is (steepness / k) sin(phase): its crest is where the sine is 1.)
    (*w).crest += steepness * b * sin(phase);
}

// One octave of drifting gradient noise in a rotated domain (so octaves never align).
// `steepness` = noise height amplitude / wavelength.
fn add_chop(w: ptr<function, Waves>, xz: vec2<f32>, t: f32, footprint: f32, wavelength: f32, steepness: f32, rot: vec2<f32>, drift: vec2<f32>, seed: f32) {
    let b = wave_band(wavelength, footprint);
    // The noise gradient has a total variance of ~0.52 (measured).
    (*w).lost_var += 0.52 * steepness * steepness * (1.0 - b * b);
    if (b <= 0.0) {
        return;
    }
    // q = R xz / λ + drift t: the slope in world space is R^T ∇n / λ (1/λ is in `steepness`).
    let q = vec2<f32>(rot.x * xz.x - rot.y * xz.y, rot.y * xz.x + rot.x * xz.y) / wavelength + drift * t + seed;
    let d = noise_d(q);
    let world_g = vec2<f32>(rot.x * d.y + rot.y * d.z, -rot.y * d.y + rot.x * d.z);
    (*w).slope += world_g * (steepness * b);
    // (The noise's value has a standard deviation of 0.18, a sine's 0.7.)
    (*w).crest += steepness * b * d.x * 3.9;
}

// Slowly drifting wind patches ("cat's paws"): a 0.45..1.45 multiplier on wave slopes, in
// patches a few hundred units across and smaller ones within them, so the sea has calm streaks
// and choppy patches rather than one ripple everywhere.
fn wind_gust(xz: vec2<f32>, t: f32) -> f32 {
    let n = noise_sum(array<vec2<f32>, 4>(xz / 460.0 + vec2<f32>(t * 0.011, t * 0.004), xz / 170.0 + vec2<f32>(3.3 - t * 0.02, t * 0.013), vec2<f32>(0.0), vec2<f32>(0.0)), vec4<f32>(1.0, 0.5, 0.0, 0.0), 2u, 0.0);
    return 0.45 + saturate(n * 1.5 + 0.5);
}

// Steepness of the capillary ripples (the finest octave of chop), and the slope variance they
// add once band-limited away: the sea's base roughness counts it in (see `water_fragment`).
const CAPILLARY: f32 = 0.035;
const CAPILLARY_VAR: f32 = 0.52 * CAPILLARY * CAPILLARY;

// The swells (see `water_waves`): direction, wavelength, steepness; and the phase offset, as a
// constant plus shares of the broad warp and the finer bend.
const SWELLS = array<vec4<f32>, 6>(
    vec4<f32>(0.800, 0.600, 173.0, 0.055),
    vec4<f32>(0.423, 0.906, 117.0, 0.050),
    vec4<f32>(0.978, -0.208, 79.0, 0.045),
    vec4<f32>(0.139, 0.990, 53.0, 0.036),
    vec4<f32>(0.906, 0.423, 35.0, 0.032),
    vec4<f32>(0.629, -0.777, 23.0, 0.028),
);
const SWELL_PHASES = array<vec3<f32>, 6>(
    vec3<f32>(0.0, 1.0, 0.0),
    vec3<f32>(1.7, -0.8, 0.0),
    vec3<f32>(4.1, 0.6, 0.3),
    vec3<f32>(2.3, -0.5, -0.6),
    vec3<f32>(5.9, 0.4, 1.0),
    vec3<f32>(0.9, -0.3, -0.8),
);
// The chop's octaves: wavelength, steepness, turn (cos, sin); then drift and seed.
const CHOPS = array<vec4<f32>, 5>(
    vec4<f32>(19.0, 0.085, 0.966, 0.259),
    vec4<f32>(10.0, 0.060, 0.500, 0.866),
    vec4<f32>(5.2, 0.045, -0.259, 0.966),
    vec4<f32>(2.6, 0.04, 0.848, -0.530),
    vec4<f32>(1.15, CAPILLARY, -0.669, 0.743),
);
const CHOP_DRIFTS = array<vec4<f32>, 5>(
    vec4<f32>(0.21, 0.09, 0.0, 0.0),
    vec4<f32>(-0.17, 0.26, 17.3, 0.0),
    vec4<f32>(0.33, -0.15, 41.7, 0.0),
    vec4<f32>(-0.4, -0.3, 73.1, 0.0),
    vec4<f32>(0.55, 0.42, 101.9, 0.0),
);

// Surface slopes of the sea at world position `xz`, time `t` (s), for a pixel footprint in
// world units. Unscaled: multiply the slope by the wave strength (and lost_var by its square).
fn water_waves(xz: vec2<f32>, t: f32, footprint: f32) -> Waves {
    var w: Waves;
    w.slope = vec2<f32>(0.0);
    w.lost_var = 0.0;
    w.crest = 0.0;
    // Low-frequency phase warps shared by the swells (their own slope is neglected: they vary
    // over hundreds of units): a broad one, and for the shorter swells a finer one too, which
    // bends their crests so that no two of them cross in a regular lattice of glints.
    let warps = noise_at(array<vec2<f32>, 4>(xz / 260.0 + vec2<f32>(3.7, t * 0.006), xz / 85.0 + vec2<f32>(-2.1, 5.3 - t * 0.01), vec2<f32>(0.0), vec2<f32>(0.0)), 2u);
    let warp = warps.x * 7.0;
    let bend = warps.y * 9.0;
    // Swell travelling roughly south-east, spread over about ±70° (`SWELLS`), then chop:
    // drifting noise at four scales, and ripples only resolved when zoomed in, which break
    // close-up highlights into sparkles; the finest, capillary ripples, give the surface crisp
    // detail close up (further off they are part of the base roughness: see `CAPILLARY_VAR`).
    // (Each in a loop, written once: see `noise_sum`.)
    for (var k = 0u; k < 6u; k++) {
        let s = SWELLS[k];
        let phase = SWELL_PHASES[k];
        add_swell(&w, xz, t, footprint, s.xy, s.z, s.w, phase.x + phase.y * warp + phase.z * bend);
    }
    for (var k = 0u; k < 5u; k++) {
        let c = CHOPS[k];
        let d = CHOP_DRIFTS[k];
        add_chop(&w, xz, t, footprint, c.x, c.y, c.zw, d.xy, d.z);
    }
    let gust = wind_gust(xz, t);
    w.slope *= gust;
    w.lost_var *= gust * gust;
    w.gust = gust;
    return w;
}

// ---- Scum on acid ----------------------------------------------------------------------------

// Scum on acid at world `xz`, time `t`, for a pixel covering `px`: rafts of film from a few to
// a hundred and more units across drifting downwind, gathered in slicks hundreds of units wide
// with open acid between them, more of them in sheltered water (by the share of the ground
// around under the sea: bays and narrow pools), fraying at their edges and holed. (Rafts of one
// size, as thick everywhere, read from high up as a camouflage print laid over the sea.) x =
// coverage (0..1), y = how much of it is a raft's edge or a hole's rim (where the film piles up
// thicker).
fn acid_scum(xz: vec2<f32>, t: f32, px: FoamPixel) -> vec2<f32> {
    let q = xz - frame.drift.xy * 2.5;
    let warp = vec2<f32>(noise(q / 90.0 + vec2<f32>(1.7, 0.0)), noise(q / 90.0 + vec2<f32>(8.3, 2.9))) * 25.0;
    let a = noise_d((q + warp) / 70.0);
    let b = noise_d((q + warp) / 26.0 + 4.1);
    let c = noise_d((q + warp) / 190.0 + 7.3);
    // (About a standard deviation of 0.13, changing by 0.016 per unit.)
    let v = a.x * 0.5 + b.x * 0.3 + c.x * 0.5;
    let g = a.yz * (0.5 / 70.0) + b.yz * (0.3 / 26.0) + c.yz * (0.5 / 190.0);
    // (The slicks: the film thins out and is gone over wide stretches.)
    let slicks = noise(q / 640.0 + vec2<f32>(3.9, -2.2)) * 0.7 + noise(q / 270.0 + vec2<f32>(-6.1, 4.4)) * 0.3;
    let threshold = mix(0.05, 0.13, smoothstep(0.7, 0.95, sea_fraction(xz, 4.0))) - slicks * 0.55;
    // (Softer from high up: a film's edge thins out over units, it is not cut.)
    let w = 0.045 + 0.05 * smoothstep(0.8, 3.0, max(length(px.dx), length(px.dy))) + 1.5 * length(vec2<f32>(dot(g, px.dx), dot(g, px.dy)));
    let rafts = smoothstep(threshold - w, threshold + w, v);
    if (rafts <= 0.0) {
        return vec2<f32>(0.0);
    }
    let holes = foam_cover(foam_noise(xz * 0.7 - frame.drift.xy * 1.75, t * 0.4, max(length(px.dx), length(px.dy))), -0.6, px).x;
    let rim = max(1.0 - abs(rafts * 2.0 - 1.0), (1.0 - abs(holes * 2.0 - 1.0)) * rafts);
    return vec2<f32>(rafts * mix(0.25, 1.0, holes), rim);
}

// The film of scum up close (drifting with its rafts): x = how thick it is here (about -1 to 1:
// thin and wrinkled to crusted), yz = how that changes (per world unit), so it catches the light
// in wrinkles; band-limited by `footprint` (world units per pixel): flat far off.
fn scum_texture(xz: vec2<f32>, t: f32, footprint: f32) -> vec3<f32> {
    let q = xz - frame.drift.xy * 2.5;
    var out = vec3<f32>(0.0);
    let b1 = wave_band(7.0, footprint);
    if (b1 > 0.0) {
        let d = noise_d(q / 7.0 + vec2<f32>(3.1, -1.7));
        out += vec3<f32>(d.x, d.yz / 7.0) * (3.0 * b1);
    }
    let b2 = wave_band(2.4, footprint);
    if (b2 > 0.0) {
        let d = noise_d(vec2<f32>(0.6 * q.x - 0.8 * q.y, 0.8 * q.x + 0.6 * q.y) / 2.4 + vec2<f32>(8.7, 2.2));
        let r = vec2<f32>(0.6 * d.y + 0.8 * d.z, -0.8 * d.y + 0.6 * d.z);
        out += vec3<f32>(d.x, r / 2.4) * (2.0 * b2);
    }
    return out;
}

// ---- Whitecaps -----------------------------------------------------------------------------

// Whitecaps out at sea (0..1 coverage) at world `xz`, time `t`, for a pixel covering `px`: where
// the wind (`wind`, world xz, length 0 calm to about 1.5 a gale) blows hard, the wind sea's
// crests break, here and there along their length: a ragged line of foam right on the crest,
// moving downwind with it, leaves a patch of foam behind that frays and narrows as it falls
// back, only the middle of the stretch lasting, broken up by the foam's own texture (as the
// surf's). Which stretches break follows where the waves' crests pile up (`crest`, see
// `Waves`), more of them in the gusts (`gust`), so the caps come and go in groups as the swell
// passes.
fn whitecaps(xz: vec2<f32>, t: f32, px: FoamPixel, wind: vec2<f32>, crest: f32, gust: f32) -> f32 {
    let speed = length(wind);
    let blow = smoothstep(0.35, 1.2, speed);
    if (blow <= 0.0) {
        return 0.0;
    }
    let footprint = max(length(px.dx), length(px.dy));
    let d = wind / max(speed, 1e-3);
    // Downwind, and along the crests.
    let a = dot(xz, d);
    let c = dot(xz, vec2<f32>(-d.y, d.x));
    // The breaking wind sea: waves 38 units long travelling downwind, their crests bent so
    // they never run in straight rows. `s` = 0 on a crest, rising downwind to 1 at the next.
    let k = TAU / 38.0;
    let omega = sqrt(23.0 * k);
    // (Its noise in loops, `noise` written once each: see `noise_sum`.)
    let warp = noise_sum(array<vec2<f32>, 4>(vec2<f32>(c / 70.0, a / 140.0 + 2.0), vec2<f32>(c / 23.0 + 5.0, a / 40.0), vec2<f32>(0.0), vec2<f32>(0.0)), vec4<f32>(3.0, 0.8, 0.0, 0.0), 2u, 0.0);
    let u = (a * k - omega * t + warp) / TAU - 0.25;
    let s = fract(u);
    // Which crest this is (each breaks along its own stretches; the count changes in the
    // troughs, where there is no foam), and how far (wavelengths) the crest has gone on past
    // this point.
    let wave = round(u);
    let behind = select(0.0, 1.0 - s, s > 0.5);
    // The stretches of crest that break now: a few units to a few tens long, different on
    // every wave, more where the swell's crests pile up and in the gusts; their ends fray, and
    // the further the foam has fallen behind the crest, the less of the stretch it holds.
    let along = noise_at(array<vec2<f32>, 4>(vec2<f32>(c / 16.0 + 9.0, wave * 3.7 + t * 0.12), vec2<f32>(c / 6.5, wave * 1.9 - t * 0.2), vec2<f32>(c / 3.7 + 1.3, a / 4.1 - 2.2), vec2<f32>(0.0)), 3u);
    let seg = along.x * 0.65 + along.y * 0.35;
    let fray = along.z * 0.3;
    let th = 0.3 - 0.12 * blow + 0.25 * behind;
    let pick = smoothstep(th, th + 0.15, crest * gust * gust * 1.1 + seg * 0.4 + fray);
    if (pick <= 0.0) {
        return 0.0;
    }
    // The crest's line of foam, ragged (never thinner than about a pixel and a half), and the
    // patch it leaves behind it, thinning out.
    let edge = noise_at(array<vec2<f32>, 4>(vec2<f32>(c / 3.1 + 4.0, wave * 2.3), vec2<f32>(c / 2.3, wave * 1.7 + 6.0), vec2<f32>(c / 7.0 + 2.0, wave * 2.9), vec2<f32>(0.0)), 3u);
    let w = max(0.05, 1.5 * footprint / 38.0) * (0.75 + 0.5 * edge.x);
    let sf = fract(s + edge.y * 0.05 + edge.z * 0.06);
    let front = max(1.0 - smoothstep(0.0, w, sf), smoothstep(1.0 - 0.5 * w, 1.0, sf));
    let trail = exp(-behind * 7.0) * smoothstep(0.5, 0.65, s);
    // The foam's texture, drawn out a little along the wind: on the crest a ragged white line,
    // behind it soft lace thinning out (not streaks combed along the wind).
    let q = vec2<f32>(a / 1.8, c) * 1.2;
    let qx = vec2<f32>(dot(px.dx, d) / 1.8, dot(px.dx, vec2<f32>(-d.y, d.x))) * 1.2;
    let qy = vec2<f32>(dot(px.dy, d) / 1.8, dot(px.dy, vec2<f32>(-d.y, d.x))) * 1.2;
    let n = foam_noise(q, t, max(length(qx), length(qy)));
    let line = foam_cover(n, 1.0 - 2.6 * front, FoamPixel(qx, qy, 0.0, 0.0)).x * front;
    let lace = smoothstep(-1.2, 1.0, n.v);
    return max(line, trail * (0.2 + 0.8 * lace)) * pick * mix(0.4, 1.0, blow);
}

// The whitecaps (`whitecaps`) for a wind that may be turning: laid out along one of a few fixed
// headings at a time, the two either side of the wind's own crossing over between them. (Laid
// out along the wind's own heading, they turn about the map's corner with it: far from it, a
// degree of turn swept their crests several wavelengths on, and as the wind came round after
// each change the whitecaps all over a sea were new from frame to frame.)
const WHITECAP_HEADINGS: f32 = 12.0;

fn turning_whitecaps(xz: vec2<f32>, t: f32, px: FoamPixel, wind: vec2<f32>, crest: f32, gust: f32) -> f32 {
    let speed = length(wind);
    if (speed <= 0.35) {
        return 0.0;
    }
    let f = atan2(wind.y, wind.x) / TAU * WHITECAP_HEADINGS;
    let i = floor(f);
    // (Over the middle half of the way from one heading to the next; a quarter of the way
    // between them at most, 7.5 degrees, the crests are square to that heading's.)
    let w = smoothstep(0.25, 0.75, f - i);
    var caps = 0.0;
    // (The two headings in a loop, `whitecaps` written once: see `noise_sum`.)
    for (var j = 0u; j < 2u; j++) {
        if ((j == 0u && w < 1.0) || (j == 1u && w > 0.0)) {
            let a = (i + f32(j)) * TAU / WHITECAP_HEADINGS;
            caps += whitecaps(xz, t, px, vec2<f32>(cos(a), sin(a)) * speed, crest, gust) * select(1.0 - w, w, j == 1u);
        }
    }
    return caps;
}

// ---- Caustics -----------------------------------------------------------------------------

// Distance to the nearest cell border of an animated Voronoi pattern (F2 - F1): 0 on the
// borders. Each cell's feature point wanders on a small Lissajous path.
fn voronoi_edge(p: vec2<f32>, t: f32) -> f32 {
    let cell = floor(p);
    let f = p - cell;
    var d1 = 8.0;
    var d2 = 8.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let o = vec2<f32>(f32(i), f32(j));
            let h = lattice_gradient(vec2<i32>(cell + o)) * 0.5 + 0.5;
            let wander = sin(TAU * h + t * (0.6 + 0.8 * h.yx));
            let d = distance(o + 0.5 + 0.4 * wander, f);
            if (d < d1) {
                d2 = d1;
                d1 = d;
            } else if (d < d2) {
                d2 = d;
            }
        }
    }
    return d2 - d1;
}

// Half width of a caustic filament in focus, in cells of its layer.
const CAUSTIC_W0: f32 = 0.07;
// Mean of `l1 + 0.75 l2` in `caustics` over space and time (measured numerically, 0.136 to
// 0.147 over the depths it shows at), so the pattern never changes the average brightness of
// the painted sea bed.
const CAUSTICS_MEAN: f32 = 0.145;

// One layer of caustic filaments: the borders of a drifting Voronoi pattern of `cell` units,
// turned by `rot` and bent by a domain warp of about a third of a cell (so they curve and
// never run straight), as soft lines `width` cells wide (they carry the same light however wide
// they spread), brightening and fading along their length.
fn caustic_layer(xz: vec2<f32>, t: f32, cell: f32, rot: vec2<f32>, drift: vec2<f32>, seed: f32, speed: f32, width: f32) -> f32 {
    let p = vec2<f32>(rot.x * xz.x - rot.y * xz.y, rot.y * xz.x + rot.x * xz.y) / cell + drift * (t * 0.03) + seed;
    let warp = noise_d(p * 0.6 + vec2<f32>(t * 0.05, -t * 0.04)).yz * 0.4;
    let e = voronoi_edge(p + warp, t * speed);
    let line = exp(-(e * e) / (width * width)) * (CAUSTIC_W0 / width);
    let along = noise_d(p * 0.8 + vec2<f32>(11.0 + t * 0.02, -4.0)).x;
    return line * (0.15 + 0.85 * smoothstep(-0.25, 0.25, along));
}

// Animated caustics on the sea bed `depth` under the water (zero mean: light gathered into
// soft, curved, broken filaments from the cells between them): two layers of different scales,
// turns and drifts, so the net never lines up into one crisp grid. The focus spreads with
// depth; a filament is never narrower than about a pixel (`footprint` world units), and the
// pattern is gone once its cells would be only a few pixels across.
fn caustics(xz: vec2<f32>, t: f32, footprint: f32, depth: f32) -> f32 {
    let b = 1.0 - smoothstep(0.7, 1.5, footprint);
    if (b <= 0.0) {
        return 0.0;
    }
    let spread = CAUSTIC_W0 + 0.01 * depth;
    let l1 = caustic_layer(xz, t, 24.0, vec2<f32>(0.966, 0.259), vec2<f32>(0.9, 0.4), 0.0, 0.7, max(spread, 1.2 * footprint / 24.0));
    let l2 = caustic_layer(xz, t, 15.0, vec2<f32>(0.5, -0.866), vec2<f32>(-0.6, 0.8), 5.3, -0.55, max(spread, 1.2 * footprint / 15.0));
    return (l1 + 0.75 * l2 - CAUSTICS_MEAN) * b;
}

// ---- Shoreline foam -----------------------------------------------------------------------
//
// Surf where the water meets the ground: a soft, translucent band of drifting foam, densest in
// a line along the water's edge, thinning out seawards in lines that wash in. It follows the
// *painted* shoreline where that agrees with the heightmap's sea-level contour (which comes
// from 16 px cells and can sit a few pixels off the painting). When the map was loaded, the
// average colours of its painted water and of its painted beach next to the water were
// measured (`frame.shore_water`, `frame.shore_land`); projecting an art colour onto the axis
// between them says how "painted water" it is. Sampling the art at a coarse mip spreads that
// 0..1 transition over a known width, which turns it into a signed distance from the painted
// shoreline. Because the foam is a function of the art coordinate and the world position, the
// terrain shader (on the strip of ground just above sea level) and the water surface give
// identical results where they meet, in both camera modes. Every edge in it is filtered by
// what the pixel covers, so it never breaks into stair steps, specks or crawling lines.

// Lit foam colour: off-white tinted a little towards the liquid's own hue (blue-white on the
// sea; on acid a dingy yellow-green froth), lit by ambient, sun (with `shadow`; the shade keeps
// the sky's fill, as on the ground) and the point light irradiance `point`. Thin foam (`thick`
// 0) is a little greyer than a thick crest (1). Shared so foam matches where terrain and water
// meet.
fn foam_color(shadow: f32, point: vec3<f32>, thick: f32) -> vec3<f32> {
    let hue = frame.water_color.rgb / max(max(frame.water_color.r, frame.water_color.g), max(frame.water_color.b, 1e-4));
    // (Acid froths a dingy yellow-green, not white.)
    let froth = mix(mix(vec3<f32>(0.84, 0.87, 0.89), hue * 0.85, 0.18), vec3<f32>(0.42, 0.46, 0.2), 0.7 * liquid_acid());
    let albedo = froth * mix(0.82, 1.0, thick);
    let sun = frame.sun_color.rgb * frame.sun_dir.w * saturate(frame.sun_dir.y) * mix(shade_fill(), vec3<f32>(1.0), shadow);
    return albedo * (frame.ambient.rgb * 0.85 + sun + point);
}

// 0 = looks like the painted beach, 1 = looks like the painted water. Compared in a roughly
// perceptual space (square root of linear), where dark worlds still separate well; the
// reference colours are stored in that space. The scale saturates at 70% of the way to the
// deep-sea colour, so lighter painted shallows still count as water.
fn painted_water(c: vec3<f32>) -> f32 {
    let axis = frame.shore_water.rgb - frame.shore_land.rgb;
    return saturate(dot(sqrt(max(c, vec3<f32>(0.0))) - frame.shore_land.rgb, axis) * frame.shore_water.a);
}

// What a pixel covers of the foam: the world xz change across it along the screen's x and y
// (anisotropic: seen grazing, one is far longer than the other), and how much the distance
// from the waterline changes across it (it widens the band's edges so they never alias).
// `above` (0..1): how far the view looks down on the ground there (see `foam_from_above`).
struct FoamPixel {
    dx: vec2<f32>,
    dy: vec2<f32>,
    band: f32,
    above: f32,
};

// Whether a view looking down by `v_y` (the y of the unit vector to the eye) sees the surf on a
// bank lying flat: from above (TA's own camera, a high one), foam drawn on the ground just above
// the water lies on the picture like the painted water there; seen from low down a bank stands
// up like a wall, and the foam on it would stand up the rock as bars.
fn foam_from_above(v_y: f32) -> f32 {
    return smoothstep(0.55, 0.8, v_y);
}

// How far above sea level (world units) the ground may carry surf: a little way up a beach; seen
// from above, as far up a bank as its painted water reaches.
fn foam_strip(above: f32) -> f32 {
    return mix(3.0, 13.0, above);
}

// The texture of foam: drifting gradient noise in four octaves (12 to 1.3 units) under a slow
// domain warp, so its patches are ragged and never repeat as shapes, normalised to a standard
// deviation of 1. `v` = its value, `g` = its gradient (per world unit), `lost` = the share of
// its variance band-limited away where an octave would alias (it softens the threshold
// instead, so far away the foam converges to its mean coverage rather than breaking up).
struct FoamNoise {
    v: f32,
    g: vec2<f32>,
    lost: f32,
};

// Variance of `noise_d`'s value (its standard deviation is about 0.18).
const NOISE_VAR: f32 = 0.0324;

// The foam's octaves (see `foam_noise`): wavelength, amplitude, turn (cos, sin); then drift and
// seed.
const FOAM_OCTAVES = array<vec4<f32>, 4>(
    vec4<f32>(12.0, 1.0, 0.966, 0.259),
    vec4<f32>(5.7, 0.62, 0.500, 0.866),
    vec4<f32>(2.7, 0.40, -0.259, 0.966),
    vec4<f32>(1.3, 0.26, 0.848, -0.530),
);
const FOAM_DRIFTS = array<vec4<f32>, 4>(
    vec4<f32>(0.050, -0.030, 0.0, 0.0),
    vec4<f32>(-0.040, 0.060, 7.1, 0.0),
    vec4<f32>(0.070, 0.020, 13.7, 0.0),
    vec4<f32>(-0.050, -0.070, 29.3, 0.0),
);

fn foam_octave(n: ptr<function, FoamNoise>, q: vec2<f32>, t: f32, footprint: f32, wavelength: f32, amp: f32, rot: vec2<f32>, drift: vec2<f32>, seed: f32) {
    let b = wave_band(wavelength, footprint);
    (*n).lost += amp * amp * NOISE_VAR * (1.0 - b * b);
    if (b <= 0.0) {
        return;
    }
    let r = vec2<f32>(rot.x * q.x - rot.y * q.y, rot.y * q.x + rot.x * q.y) / wavelength + drift * t + seed;
    let d = noise_d(r);
    (*n).v += amp * b * d.x;
    (*n).g += vec2<f32>(rot.x * d.y + rot.y * d.z, -rot.y * d.y + rot.x * d.z) * (amp * b / wavelength);
}

fn foam_noise(xz: vec2<f32>, t: f32, footprint: f32) -> FoamNoise {
    var n: FoamNoise;
    n.v = 0.0;
    n.g = vec2<f32>(0.0);
    n.lost = 0.0;
    // (The warp's own slope is neglected: it changes over tens of units.)
    let warp = noise_at(array<vec2<f32>, 4>(xz / 37.0 + vec2<f32>(t * 0.010 + 5.3, -t * 0.013), xz / 37.0 + vec2<f32>(11.9 - t * 0.012, t * 0.009), vec2<f32>(0.0), vec2<f32>(0.0)), 2u).xy;
    let q = xz + warp * 6.0;
    // (The octaves in a loop, `noise_d` written once: see `noise_sum`.)
    for (var k = 0u; k < 4u; k++) {
        let a = FOAM_OCTAVES[k];
        let b = FOAM_DRIFTS[k];
        foam_octave(&n, q, t, footprint, a.x, a.y, a.zw, b.xy, b.z);
    }
    // Unit total variance: NOISE_VAR * (1 + 0.62² + 0.4² + 0.26²).
    let k = inverseSqrt(NOISE_VAR * 1.6120);
    n.v *= k;
    n.g *= k;
    n.lost *= k * k;
    return n;
}

// Foam where the noise `n` rises above `threshold`, its edge softened by how much the noise
// changes across the pixel `px` and by what band limiting took out: x = density (0..1),
// y = thickness (well above the threshold: thick foam).
fn foam_cover(n: FoamNoise, threshold: f32, px: FoamPixel) -> vec2<f32> {
    let along_x = dot(n.g, px.dx);
    let along_y = dot(n.g, px.dy);
    let sigma = sqrt(NOISE_VAR + 0.2 * (along_x * along_x + along_y * along_y) + n.lost);
    let density = smoothstep(threshold - 1.9 * sigma, threshold + 1.9 * sigma, n.v);
    let thick = smoothstep(threshold, threshold + 1.2 + 1.9 * sigma, n.v);
    return vec2<f32>(density, thick);
}

// The art at mip `lod` through a cubic B-spline from four bilinear taps (Sigg and Hadwiger):
// averaged smoothly over a few of its texels, without the kinks a bilinear sample has at every
// texel, so contours traced in it are smooth curves rather than the painting's pixel steps.
fn art_bspline(px: vec2<f32>, lod: u32) -> vec3<f32> {
    let s = f32(1u << lod);
    let p = px / s - 0.5;
    let i = floor(p);
    let f = p - i;
    let f2 = f * f;
    let f3 = f2 * f;
    let w0 = (1.0 - 3.0 * f + 3.0 * f2 - f3) / 6.0;
    let w1 = (4.0 - 6.0 * f2 + 3.0 * f3) / 6.0;
    let w2 = (1.0 + 3.0 * f + 3.0 * f2 - 3.0 * f3) / 6.0;
    let w3 = f3 / 6.0;
    let g0 = w0 + w1;
    let g1 = w2 + w3;
    let h0 = (i - 0.5 + w1 / g0) * s;
    let h1 = (i + 1.5 + w3 / g1) * s;
    return (art_bilinear(h0, lod, 0.0) * g0.x + art_bilinear(vec2<f32>(h1.x, h0.y), lod, 0.0) * g1.x) * g0.y
        + (art_bilinear(vec2<f32>(h0.x, h1.y), lod, 0.0) * g0.x + art_bilinear(h1, lod, 0.0) * g1.x) * g1.y;
}

// How much foam lies at distance `d` from a waterline (> 0 seawards): a bright line along the
// water's edge, broken here and there (`gap`), and lines of foam parallel to the shore washing
// in behind it, thinning out up to `reach`. The surf breathes with the sets of waves coming in
// (`swell`, -1..1): the line widens and narrows, its seaward edge ragged with the foam's own
// texture (`rag`, about -2..2). `aa` = the distance's change across the pixel: the band's
// edges are never sharper than it. `beach` (0..1): whether the ground there carries the swash
// up it (a beach, or seen from above any bank, see `foam_from_above`); elsewhere the band fades
// in on the water, so the water's edge never cuts it off. `wash` = how strong the lines washing
// in are.
fn foam_band(d: f32, aa: f32, swell: f32, reach: f32, gap: f32, rag: f32, beach: f32, wash: f32, t: f32) -> f32 {
    let w = max(0.7 * aa, 0.35);
    // From a little up the beach (where the swash wets the sand) out over the water.
    let wet = smoothstep(mix(-0.2, -2.5 - w, beach), mix(1.2 + w, -0.5 + w, beach), d);
    let width = max(3.8 + 1.2 * swell + 0.9 * rag, 1.0);
    let line = wet * (1.0 - smoothstep(0.5 - w, width + w, d)) * mix(0.15, 1.0, smoothstep(-0.25, 0.15, gap + 0.05));
    // (The rows fade out where the pixel cannot resolve them.)
    let rows_k = smoothstep(3.0, 6.0, 6.0 / max(aa, 1e-3));
    let rows = 0.5 + 0.5 * sin(TAU * (d / 6.0 + 0.18 * t));
    let fringe = wet * (1.0 - smoothstep(-1.0, reach, d)) * (1.0 - 0.6 * rows_k * (1.0 - rows));
    return max(line, wash * fringe);
}

// How much ground of slope `rise` (height per unit) lies like a beach, which the surf runs up
// (1), rather than a bank (0, steeper than about 30°; see `foam_from_above`).
fn foam_beach(rise: f32) -> f32 {
    return smoothstep(0.87, 0.97, inverseSqrt(1.0 + rise * rise));
}

// Whether ground of slope `rise` carries the surf up it, seen from `above` (see
// `foam_from_above`): a beach always; seen from above a bank too, but not a cliff, which the
// view squeezes into a pixel or two (surf on it would draw a bright contour round the coast).
fn foam_runs_up(rise: f32, above: f32) -> f32 {
    return max(foam_beach(rise), above * smoothstep(0.4, 0.6, inverseSqrt(1.0 + rise * rise)));
}

// How far (world units) from the waterline surf can reach: a band around a painted shoreline
// up to 14 units off it reaches about 13 units further (callers skip `shore_foam` beyond).
const SURF_REACH: f32 = 29.0;

// How much of the band of surf lies at a point `height_dist` from the waterline and `line` from
// the painted shoreline (see `shore_foam`; the rest as `foam_band`, `rise` and `px` as there).
// Where the water meets the ground there is surf, drawn along the painted shoreline where the
// two lie within a few units of each other (a blend of distances that agree that well cannot
// fold); a trusted painted shoreline further off gets its own, which seen from above (where it
// reads as the coast) replaces the waterline's. Not around a vertex or two of the heightmap that
// reach the surface out in a painted sea (shoals the art does not show): no lone flecks of surf
// out at sea.
fn shore_cover(line: f32, height_dist: f32, rise: f32, xz: vec2<f32>, px: FoamPixel, swell: f32, reach: f32, gap: f32, rag: f32, runs_up: f32, wash: f32, t: f32) -> f32 {
    var trust = 0.0;
    if (frame.shore_land.a > 0.0) {
        let strip = foam_strip(px.above) - 1.0;
        trust = (1.0 - smoothstep(6.0, 14.0, abs(line - height_dist))) * (1.0 - smoothstep(strip * 0.4, strip, abs(line - height_dist) * rise));
    }
    let agree = trust * (1.0 - smoothstep(2.0, 6.0, abs(line - height_dist)));
    let apart = trust - agree;
    var real = 1.0 - apart * px.above;
    if (frame.shore_land.a > 0.0 && line > 8.0) {
        real *= 1.0 - smoothstep(8.0, 12.0, line) * smoothstep(0.85, 0.95, sea_fraction(xz, 2.0));
    }
    return max(foam_band(mix(height_dist, line, agree), px.band, swell, reach, gap, rag, runs_up, wash, t) * real, foam_band(line, px.band, swell, reach, gap, rag, runs_up, wash, t) * apart);
}

// Foam at a surface point near the shore: x = opacity (0..1, never fully opaque), y = how thick
// it is there (see `foam_color`).
// `art_px`: the point's art coordinate (x, z - y/2); `xz`: its world position (anchors the
// animated texture); `height_dist`: its distance from the waterline (`shore_distance`) and
// `rise` the ground's slope there (height per unit); `px`: what the pixel covers.
//
// The band lies along the painted shoreline where the painting agrees with the heights: the
// per-tile average colour (one bilinear fetch) spreads its 0..1 water mask over ~32 px, a smooth
// distance field that saturates at +16 px (open water), traced more finely near the edge from
// the art at a 4 px mip through a B-spline. Some maps paint shallows up onto the land (Red
// River), paint water up a bank, or light sand bars under deeper water: where the painted
// shoreline lies far from the real waterline, or higher above or below it than the ground may
// carry surf (`foam_strip`), only the real waterline has surf. (The bands are weighed, not
// their distances: blending distances that disagree in sign folds the band into stray loops.
// So too the fine trace's and the coarse one's: where a patch of the painting looks like the
// other side of the shore from the coarse field, their blend crossed the waterline along where
// it changes, a ruler-straight line of surf or of none along the coarse field's contour.)
// Where the painter faded water into land over more than those widths, its mask changes more
// slowly than either assumes and would draw the band as much wider: both are measured by how
// fast the mask changes there, so a softly painted shore gets as narrow a band as a crisp one.
fn shore_foam(art_px: vec2<f32>, xz: vec2<f32>, height_dist: f32, rise: f32, t: f32, px: FoamPixel) -> vec2<f32> {
    var coarse = height_dist;
    var fine = height_dist;
    var near = 0.0;
    if (frame.shore_land.a > 0.0) {
        let c0 = painted_water(art_sample(art_px, 5.0, 0.0));
        let cx = painted_water(art_sample(art_px + vec2<f32>(16.0, 0.0), 5.0, 0.0));
        let cz = painted_water(art_sample(art_px + vec2<f32>(0.0, 16.0), 5.0, 0.0));
        let soft = clamp(length(vec2<f32>(cx - c0, cz - c0)) * 2.0, 0.25, 1.0);
        coarse = (c0 - 0.5) * 32.0 / soft;
        fine = coarse;
        near = 1.0 - smoothstep(3.0, 6.0, abs(coarse));
        if (near > 0.0) {
            let m0 = painted_water(art_sample(art_px, 3.0, 0.0));
            let mx = painted_water(art_sample(art_px + vec2<f32>(4.0, 0.0), 3.0, 0.0));
            let mz = painted_water(art_sample(art_px + vec2<f32>(0.0, 4.0), 3.0, 0.0));
            let soft_fine = clamp(length(vec2<f32>(mx - m0, mz - m0)) * 2.0, 0.2, 1.0);
            fine = (painted_water(art_bspline(art_px, 2u)) - 0.62) * 6.0 / soft_fine;
        }
    }
    if (min(min(abs(coarse), abs(fine)), abs(height_dist)) > 15.0) {
        return vec2<f32>(0.0);
    }
    let footprint = max(length(px.dx), length(px.dy));
    let n = foam_noise(xz, t, footprint);
    // Slow variation along the coast: band width, swash rhythm, gaps in the line at the edge.
    let meander = noise(xz / 110.0 + vec2<f32>(t * 0.013, -t * 0.007));
    let breathe = sin(t * 0.9 + meander * 9.0);
    let gap = noise(xz / 45.0 + vec2<f32>(0.0, t * 0.03));
    let reach = 9.0 + 3.0 * clamp(meander, -0.7, 0.7) + 1.5 * breathe;
    let runs_up = foam_runs_up(rise, px.above);
    // Zoomed far out the band is a few pixels wide: only its line along the water's edge, faint,
    // so the coasts never read as outlined.
    let far = smoothstep(0.7, 2.0, footprint);
    let wash = 0.8 * (1.0 - 0.75 * far);
    var cover = 0.0;
    if (near < 1.0) {
        cover = shore_cover(coarse, height_dist, rise, xz, px, breathe, reach, gap, n.v, runs_up, wash, t) * (1.0 - near);
    }
    if (near > 0.0) {
        cover += shore_cover(fine, height_dist, rise, xz, px, breathe, reach, gap, n.v, runs_up, wash, t) * near;
    }
    if (cover <= 0.0) {
        return vec2<f32>(0.0);
    }
    // The denser the band, the more of the noise foams; denser foam is thicker and more opaque.
    // Seen from above (TA's own camera, a high one) the band lies flat on the picture at its full
    // width, and as dense as it is drawn for the foreshortened band seen from low down it was an
    // opaque white outline traced round the coast like a sticker's border: from above the foam is
    // laid thinner, so that its own texture breaks it into lace.
    // (And unevenly along the coast, sparser where the line along the water's edge breaks: an
    // even lace all round still traced the coast as a dashed rim.)
    let sparse = mix(1.0, 0.35 + 0.3 * smoothstep(-0.3, 0.3, gap), px.above);
    let f = foam_cover(n, 1.2 - 3.2 * cover * sparse, px);
    let alpha = f.x * mix(0.6, 0.95, f.y) * smoothstep(0.0, 0.4, cover) * mix(1.0, 0.75, px.above);
    return vec2<f32>(alpha * (1.0 - 0.5 * far), f.y);
}

// ---- Lava ---------------------------------------------------------------------------------
//
// On TA's lava worlds the "sea" below sea level is molten rock. Instead of reflections and
// foam it gets slowly drifting dark crust plates with glowing cracks, pulsing hot spots, an
// incandescent rim along the painted shore, and a glow cast onto the rock around it. The
// painted lava colour (normalised to a peak of 1) arrives in `frame.water_color.rgb`.

// Incandescence for a temperature-like value: 0 = the painted lava colour, 1 = yellow-orange
// (kept saturated: the tone mapper whitens anything much brighter).
fn lava_heat_color(heat: f32) -> vec3<f32> {
    let h = saturate(heat);
    let orange = vec3<f32>(1.0, 0.33, 0.04);
    let yellow = vec3<f32>(1.0, 0.62, 0.16);
    return mix(mix(frame.water_color.rgb, orange, smoothstep(0.0, 0.6, h)), yellow, smoothstep(0.6, 1.0, h));
}

struct LavaSample {
    // Additive HDR emission.
    emission: vec3<f32>,
    // Coverage of dark, cooled crust (0..1).
    crust: f32,
    // The crust's grain seen close up (lighter and darker, about -1..1; 0 from further off).
    grain: f32,
};

// The lava at world `xz`, time `t`, `shore` units from its bank (where it has cooled into a band
// of crust).
fn lava_surface(xz: vec2<f32>, t: f32, footprint: f32, shore: f32) -> LavaSample {
    let flow = vec2<f32>(0.92, 0.38);
    // Heat: drifting noise at three scales gives hot channels and cooler, crusted patches; the
    // melt cools against the banks.
    // (In a loop, `noise` written once: see `noise_sum`.)
    let n = noise_at(array<vec2<f32>, 4>(xz / 110.0 + flow * (t * 0.008), xz / 37.0 - flow.yx * (t * 0.015) + 5.1, xz / 13.0 + flow * (t * 0.03) + 9.7, vec2<f32>(0.0)), 3u);
    let n1 = n.x;
    let n2 = n.y;
    let n3 = n.z * wave_band(13.0, footprint);
    let banks = smoothstep(2.0, 16.0, shore);
    let heat = saturate(0.5 + n1 * 1.9 + n2 * 0.7 + n3 * 0.4) * mix(0.35, 1.0, banks);
    let pulse = 0.9 + 0.1 * sin(t * 1.3 + n2 * 9.0);
    // Crust forms where the melt is cooler: irregular plates (domain-warped Voronoi) drifting
    // with the flow, split by thin seams that glow, and smaller plates breaking up the larger
    // ones; along the banks a band of it, nearly solid.
    let detail = wave_band(38.0, footprint);
    let fine = wave_band(14.0, footprint);
    let warp = noise_d(xz / 53.0 + 2.0).yz * 10.0;
    let drift = xz + warp - flow * (t * 1.2);
    // (A seam is a share of its plate wide, but seen close up no wider than a few pixels or half
    // a unit: a crack, not a broad blurred band. F2 - F1 is about twice the distance to the
    // border, in cells.)
    let crack = max(7.0 * footprint, 0.5);
    let edge = voronoi_edge(drift / 38.0, t * 0.05);
    let w = min(0.09, crack / 38.0);
    let seam = (1.0 - smoothstep(0.22 * w, w, edge)) * detail;
    // Up close the crust is rough basalt (lighter and darker patches), and thin enough at the
    // plates' edges for the melt to glow through it beside the cracks.
    let close = wave_band(1.5, footprint);
    var halo = (1.0 - smoothstep(0.0, 2.4, edge * 19.0)) * close;
    var seam2 = 0.0;
    if (fine > 0.0) {
        let edge2 = voronoi_edge(drift / 14.0 + 3.3, t * 0.08);
        let w2 = min(0.1, crack / 14.0);
        seam2 = (1.0 - smoothstep(0.3 * w2, w2, edge2)) * fine;
        halo = max(halo, (1.0 - smoothstep(0.0, 1.2, edge2 * 7.0)) * 0.6 * close);
    }
    let cool = max(1.0 - smoothstep(0.2, 0.55, heat), 1.0 - banks);
    // (Thick along the banks, its plates still parted by glowing seams: melt painted up a bank
    // above the level is all bank, and with the seams faded it was a dull sheet.)
    let seams = max(seam, seam2 * 0.55) * mix(0.6, 1.0, banks);
    let crust = cool * (1.0 - seams) * mix(0.6, 0.95, detail);
    var s: LavaSample;
    s.crust = crust;
    s.grain = 0.0;
    if (close > 0.0) {
        s.grain = (noise(drift / 3.0 + 1.3) * 0.65 + noise(drift / 1.1 + 6.1) * 0.35 * wave_band(1.1, footprint)) * 3.0 * close;
    }
    // The melt glows from the painted red towards orange-yellow where hottest, under a thin
    // skin whose fine cracks glow brighter (so the hottest melt keeps its detail rather than
    // flattening into a blown-out sheet); the seams in the crust glow orange.
    let skin = 1.0 - 0.2 * (1.0 - seam2 / max(fine, 1e-3)) * fine * (1.0 - cool);
    s.emission = lava_heat_color(heat) * ((0.12 + 0.68 * heat * heat) * pulse * (1.0 - crust) * skin)
        + lava_heat_color(0.45) * (seams * cool * 1.1) + lava_heat_color(0.2) * (halo * halo * crust * 0.45);
    return s;
}

// How much of the ground at art pixel `art_px`, `depth` under the lava's level (< 0 above it),
// the melt covers (0..1). The heightmap's 16-unit cells cannot hold TA's painted shore (rock
// spikes reaching into the river, channels of melt between them, a cell's row cutting a bank
// straight), so within a band of the level the painting decides, where it is painted as melt;
// deep under the level melt, well above it rock; without a measured shore the heightmap alone.
// The water pass and the terrain pass share it, so their melt meets without a seam.
fn lava_melt(art_px: vec2<f32>, depth: f32, footprint: f32) -> f32 {
    let wet = smoothstep(0.0, 2.0, depth);
    if (frame.shore_land.a <= 0.0) {
        return wet;
    }
    let paint = painted_water(art_sample(art_px, clamp(log2(max(footprint, 1e-3)) + 1.0, 1.0, 3.0), 0.0));
    // (Above the level only paint that is surely melt: a world's rust and red rock are not.)
    let above = 1.0 - smoothstep(-2.0, 2.0, depth);
    let painted = smoothstep(mix(0.25, 0.45, above), mix(0.6, 0.7, above), paint);
    let near = (1.0 - smoothstep(12.0, 20.0, depth)) * (1.0 - smoothstep(6.0, 12.0, -depth));
    return mix(wet, painted, near);
}

// The glowing rim of the melt along TA's painted shore at art pixel `art_px`: a few pixels wide
// (half as wide seen close up), and never more than a few screen pixels (`dx`, `dy`: the art
// coordinate's change across one): close up, where TA's pixels are large on the screen, a thin
// line along the edge, not a broad band outlining the melt like a sticker's. It fades when
// zoomed far out rather than flickering.
fn lava_rim(art_px: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    let footprint = max(length(dx), length(dy));
    let lod = clamp(log2(max(footprint, 1e-3)) + 2.0, 1.0, 2.0);
    let fine = painted_water(art_sample(art_px, lod, 0.0));
    var rim = smoothstep(0.2, 0.5, fine) * (1.0 - smoothstep(0.55, 0.95, fine)) * (1.0 - smoothstep(3.0, 8.0, footprint));
    if (rim > 0.0) {
        // (How far from the line the mask's change across a screen pixel puts this one: its
        // slope over a texel.)
        let e = exp2(lod);
        let g = vec2<f32>(painted_water(art_sample(art_px + vec2<f32>(e, 0.0), lod, 0.0)), painted_water(art_sample(art_px + vec2<f32>(0.0, e), lod, 0.0))) - fine;
        let per_px = max(abs(dot(g, dx)), abs(dot(g, dy))) / e;
        rim *= 1.0 - smoothstep(0.75, 1.75, abs(fine - 0.52) / max(per_px, 1e-4));
    }
    return rim;
}

// The lava's surface at `p` (on its level, `shore` units from its bank), over as much of the
// ground as `melt` covers, `shadow` the sun's light on it: the cooled crust, lit, the melt's
// glow and the rim's (`rim`), premultiplied (rgb is added over what lies beneath, a is how much
// of it the crust hides).
fn lava_layer(p: vec3<f32>, shore: f32, melt: f32, rim: f32, shadow: f32, footprint: f32, t: f32) -> vec4<f32> {
    let s = lava_surface(p.xz, t, footprint, shore);
    let sun = frame.sun_color.rgb * frame.sun_dir.w * saturate(frame.sun_dir.y) * mix(0.5, 1.0, shadow);
    // Cooled basalt: dark, lit by the sky and faintly by the melt underneath.
    let crust_color = vec3<f32>(0.05, 0.04, 0.035) * max(1.0 + s.grain, 0.25) * (frame.ambient.rgb + sun) + frame.water_color.rgb * 0.04;
    return vec4<f32>((crust_color * s.crust + s.emission) * melt + lava_heat_color(0.7) * rim, s.crust * 0.9 * melt);
}

// Light that lava casts on nearby rock: `near` = how much painted lava surrounds the point
// (the painted-water mask sampled at a very coarse mip), `around` the same over a few times
// that reach, in 0..1: bright right beside the melt, a faint warm glow further off.
fn lava_glow(near: f32, around: f32, t: f32) -> vec3<f32> {
    return lava_heat_color(0.35) * ((near * near * 1.2 + around * 0.7) * (1.0 + 0.12 * sin(t * 1.7)));
}

// ---- Rain ---------------------------------------------------------------------------------

// Rain rings: drops land in cells `RAIN_CELL` units across, two at a time in each, one after
// another (a drop lives 0.55 to 1.05 s), each somewhere new in its cell, a ring spreading out to
// between 0.55 and 1 times `RAIN_REACH` units as a soft crest that widens and flattens as it goes
// (never further than a cell from where it fell: only the cells around are looked at).
const RAIN_CELL: f32 = 4.5;
const RAIN_REACH: f32 = 3.0;

// How much of the rain rings the pixels can show at `footprint` world units per pixel: all of
// them up close, none once a young ring's crest would be under a pixel and a half wide.
fn rain_band(footprint: f32) -> f32 {
    return 1.0 - smoothstep(0.2, 0.6, footprint);
}

// Rain rings on standing water around world `xz` while it rains (`rain` 0..1): drops land at
// random all the time, each a ring spreading and fading within a second or so, of its own size,
// more of them the harder it rains. Returns the rings' slope (d height / dx, dz), faded out where
// they are too fine for the pixels (`footprint`, world units per pixel); further off
// `rain_roughness` stands in for them.
fn rain_ripples(xz: vec2<f32>, t: f32, rain: f32, footprint: f32) -> vec2<f32> {
    let band = rain_band(footprint);
    if (rain <= 0.0 || band <= 0.0) {
        return vec2<f32>(0.0);
    }
    let p = xz / RAIN_CELL;
    let base = floor(p);
    var slope = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let c = base + vec2<f32>(f32(i), f32(j));
            let cell = bitcast<u32>(i32(c.x)) * 73856093u ^ bitcast<u32>(i32(c.y)) * 19349663u;
            for (var k = 0u; k < 2u; k++) {
                let h0 = hash_u32(cell + k * 0x9e3779b9u);
                // (Each of the cell's two runs of drops at its own pace; a drop's number picks
                // where it lands, how big it spreads and whether one falls at all.)
                let life = 0.55 + f32(h0 & 1023u) / 1023.0 * 0.5;
                let cycle = t / life + f32((h0 >> 10u) & 1023u) / 1023.0;
                let drop = floor(cycle);
                let age = cycle - drop;
                let h = hash_u32(h0 ^ (bitcast<u32>(i32(drop)) * 2654435761u));
                if (f32(h & 1023u) / 1023.0 > rain * 0.6) {
                    continue;
                }
                let at = c + vec2<f32>(f32((h >> 10u) & 1023u), f32((h >> 20u) & 1023u)) / 1023.0;
                let size = 0.55 + 0.45 * f32(h >> 30u) / 3.0;
                let d = (p - at) * RAIN_CELL;
                let r = length(d);
                // A crest (height exp(-x²)) running outwards, wider and lower as it spreads.
                let w = mix(0.4, 0.75, age) * sqrt(size);
                let x = (r - RAIN_REACH * size * (0.1 + 0.9 * age)) / w;
                let amp = smoothstep(0.0, 0.12, age) * pow(1.0 - age, 1.5) * size;
                slope += d / max(r, 1e-3) * (-2.0 * x * exp(-x * x) / w * amp);
            }
        }
    }
    return slope * (0.35 * min(rain * 1.5, 1.0) * band);
}

// The slope variance of the rain rings the pixels cannot show at `footprint` (see
// `rain_ripples`): it roughens the water instead, so rain-pocked water far off is duller rather
// than speckled.
fn rain_roughness(rain: f32, footprint: f32) -> f32 {
    let band = rain_band(footprint);
    return 0.02 * min(rain * 1.5, 1.0) * (1.0 - band * band);
}
