// Wakes behind ships and hovercraft (see wakes.rs), drawn into the water's wake map: each a
// ribbon along the way a hull came, from a little ahead of its bow, every texel working out the
// wake from where on it it lies: how far across the way (`l`), how far behind the bow (`s`), how
// long ago the bow passed and how fast it went then. Prepended: common.wgsl, noise.wgsl.
//
// Out: rg = the slope of the waves the hull raised (d height / dx, dz; where wakes cross their
// waves add up), b = how much its churn has smoothed the wind's ripples away (the slick), a =
// foam (the most of the wakes there: a column of ships sailing in each other's wakes does not
// whiten into a road).

struct WakeIn {
    // xy = world xz, zw = the unit vector across the way (towards positive `l`)
    @location(0) pos_side: vec4<f32>,
    // x = l, y = s, z = age (s), w = speed then (world units a second)
    @location(1) shape: vec4<f32>,
    // x = how far the hull had come along its way here (world units), y = its length, z = its
    // half-width, w = 1 for a hovercraft
    @location(2) hull: vec4<f32>,
};

struct WakeOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec2<f32>,
    @location(1) side: vec2<f32>,
    @location(2) shape: vec4<f32>,
    // x = how far the ribbon reaches across here (its edge, where everything has faded)
    @location(3) hull: vec4<f32>,
    @location(4) travelled: f32,
};

@vertex
fn vs_wake(in: WakeIn) -> WakeOut {
    var out: WakeOut;
    out.clip = frame.view_proj * vec4<f32>(in.pos_side.x, frame.world.y, in.pos_side.y, 1.0);
    out.world = in.pos_side.xy;
    out.side = in.pos_side.zw;
    out.shape = in.shape;
    out.hull = vec4<f32>(abs(in.shape.x), in.hull.yzw);
    out.travelled = in.hull.x;
    return out;
}

// tan(19.47°): the arms of a wake open at the Kelvin angle, whatever the speed.
const KELVIN: f32 = 0.354;
// How long a wake lasts (s; `WAKE_LIFE` in wakes.rs): it fades out over its last third.
const WAKE_LIFE: f32 = 14.0;

// A crest across a line, `w` wide: its height (1 on the line) and how fast that changes.
fn crest(x: f32, w: f32) -> f32 {
    return exp(-(x * x) / (w * w));
}

fn crest_d(x: f32, w: f32) -> f32 {
    return -2.0 * x / (w * w) * exp(-(x * x) / (w * w));
}

@fragment
fn fs_wake(in: WakeOut) -> @location(0) vec4<f32> {
    let l = in.shape.x;
    let s = in.shape.y;
    let age = max(in.shape.z, 0.0);
    let speed = in.shape.w;
    let reach = in.hull.x;
    let hull_len = in.hull.y;
    let beam = in.hull.z;
    let hover = in.hull.w > 0.5;
    let across = abs(l);
    // World units a texel of the wake map spans: nothing in it narrower than about a texel and
    // a half (what would be thinner spreads out, as faint).
    let texel = max(length(dpdx(in.world)), length(dpdy(in.world)));
    // How much of a wake the hull makes at the speed it went (a ship drifting makes none), and
    // how big it is (its foam and waves broader).
    let go = smoothstep(5.0, 40.0, speed);
    let size = clamp(beam / 10.0, 0.6, 2.5);

    // ---- The arms ------------------------------------------------------------------------------
    // The bow wave leaves the stem in two arms that open at the Kelvin angle: each point of an
    // arm has moved out from the way by the distance the bow has gone since, times tan 19.5°. A
    // blunt hull pushes it out wider along its sides as far as its shoulder (a ship's pointed bow
    // widens over its first third, a skirt is round), and the arms go on from there. Each arm is
    // a crest on a short train of waves inside it, which go on catching the light for a couple of
    // seconds; its foam is white where the bow wave breaks along the hull and breaks up within a
    // hull's length or so behind it, a few flecks left on the crest.
    let hull_side = select(beam * saturate(s / max(0.3 * hull_len, 1.0)), beam * sqrt(saturate(s / max(0.5 * hull_len, 1.0))), hover);
    let spread = age * speed * KELVIN;
    let w0 = (1.6 + 0.08 * spread) * size;
    let w = max(w0, 1.5 * texel);
    let thin = w0 / w;
    // (The crest lies just clear of the hull's side.)
    let d = across - sqrt(hull_side * hull_side + spread * spread) - 0.5 * w;
    // (No arm ahead of the bow.)
    let behind_bow = smoothstep(-w, w, s);
    // Behind the hull the arm's waves come as short crests in echelon, each slanting out across
    // it as it goes back (the divergent waves), a third of a hull apart; where the texels cannot
    // hold them, an even line.
    // (Not quite evenly: each arm's crests come a little irregularly, some stronger.)
    let echelon = smoothstep(0.3, 0.9, s / max(hull_len, 1.0));
    let period = max(0.35 * hull_len, 8.0);
    let jitter = noise(vec2<f32>(s / (2.7 * period), sign(l) * 5.3 + 1.1));
    let u = fract(s / period + 0.6 * jitter) - 0.5;
    let dashed = echelon * smoothstep(4.0 * texel, 8.0 * texel, period);
    let e = d - u * period * 0.6 * dashed;
    let dash = (1.0 - 0.85 * dashed * smoothstep(0.2, 0.5, abs(u))) * (1.0 + dashed * 0.9 * noise(vec2<f32>(s / period + 0.5, sign(l) * 3.7)));
    let breaking = (1.0 + 0.8 * (1.0 - smoothstep(0.1, 0.6, s / max(hull_len, 1.0)))) * exp(-s / (1.3 * max(hull_len, 1.0))) * exp(-age / 2.5);
    // (A hull going fast for its length, a boat's more than a ship's, throws white water down
    // its arms' crests for longer.)
    let fast = smoothstep(0.6, 1.8, speed / max(hull_len, 1.0));
    let flecks = (0.18 + 0.4 * fast) * exp(-age / (2.0 + 1.5 * fast)) * crest(e, w) * dash;
    var foam = go * behind_bow * thin * (breaking * crest(d, w) + flecks) * select(0.9, 0.45, hover);
    // (A crest, then a trough and a lower crest inside it.)
    let amp = 0.45 * go * behind_bow * exp(-age / 3.0 - s / (5.0 * max(hull_len, 1.0))) * select(1.0, 0.45, hover) * min(size, 1.6) * thin * dash;
    let g = 2.2 * w;
    let dh = crest_d(e, w) - 0.55 * crest_d(e + g, w) + 0.25 * crest_d(e + 2.0 * g, w);
    var slope = in.side * (sign(l) * amp * w * dh);

    // ---- The bow ----------------------------------------------------------------------------
    // Water heaped up white against the stem; round a hovercraft's skirt, the spray its air
    // throws up all round it.
    let cushion = max(0.9 * size + 0.15 * beam, 1.5 * texel);
    foam += go * crest(length(vec2<f32>(s * 1.6, l)), cushion) * select(1.0, 0.7, hover);
    if (hover) {
        let x = saturate(s / max(hull_len, 1.0));
        let skirt = beam * sqrt(saturate(4.0 * x * (1.0 - x)));
        foam += go * 0.7 * crest(across - skirt - 0.5 * w, w) * smoothstep(-w, w, s) * (1.0 - smoothstep(hull_len - w, hull_len + w, s));
    }

    // ---- The churn down the middle -------------------------------------------------------------
    // Behind the stern the water the screws (a skirt's fans) churned: white for a couple of
    // seconds (a long trail behind a fast boat, a short one behind a slow ship), then a pale band
    // of bubbles slowly widening (the water shows little foam in it, see water.wgsl), streaked
    // along the way, and a slick in which the wind's waves stay smoothed long after.
    let past_stern = s - hull_len;
    let churned = max(age - hull_len / max(speed, 1.0), 0.0);
    let width = beam * min(select(0.65, 1.1, hover) + 0.07 * churned, 1.8);
    let starts = smoothstep(-0.3 * beam, 0.5 * beam, past_stern);
    let core = (1.0 - smoothstep(0.4 * width, width, across)) * starts;
    // (The streaks stay where they were left on the water.)
    let streaks = clamp(0.75 + 0.6 * noise(vec2<f32>(l / max(0.9 * size, texel * 2.0), in.travelled / 30.0 + 3.7)), 0.35, 1.2);
    // (A skirt's trail is broader, and sooner gone.)
    let white = exp(-churned / select(2.0, 1.5, hover));
    foam += go * core * (0.8 * white + 0.22 * exp(-churned / 6.0)) * streaks;
    let slick = 0.6 * smoothstep(0.05, 0.4, go) * (1.0 - smoothstep(0.6 * width, 1.2 * width, across)) * starts * exp(-churned / 16.0);

    // Faded out at the ribbon's edge, and over the wake's last few seconds.
    let k = (1.0 - smoothstep(0.8 * reach, reach, across)) * (1.0 - smoothstep(WAKE_LIFE * 0.65, WAKE_LIFE, age));
    return vec4<f32>(slope * k, slick * k, foam * k);
}
