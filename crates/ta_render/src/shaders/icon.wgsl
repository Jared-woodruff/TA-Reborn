// Strategic icons (see icons.rs): each instance is a screen-aligned quad around a projected
// world position, `size` pixels across (plus a margin for the outline and anti-aliasing),
// whose fragments evaluate the icon shape's signed distance: a filled shape in the icon's
// colour, lit a little from above, inside a dark outline.

struct IconUniform {
    view_proj: mat4x4<f32>,
    // xy = the target's size in pixels
    target_size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: IconUniform;

struct IconIn {
    // xyz = world position, w = size in pixels
    @location(0) pos_size: vec4<f32>,
    // rgb = linear colour, a = opacity
    @location(1) color: vec4<f32>,
    // x = shape (see IconShape)
    @location(2) shape: vec4<u32>,
};

struct IconOut {
    @builtin(position) clip: vec4<f32>,
    // Position in the icon, 1 = half its size (y up the screen).
    @location(0) local: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
    @location(3) @interpolate(flat) half_px: f32,
};

// Pixels around the shape the quad covers: the outline and its anti-aliasing.
const MARGIN: f32 = 2.5;
// Width of the dark outline (pixels).
const OUTLINE: f32 = 1.5;

@vertex
fn vs_icon(@builtin(vertex_index) vi: u32, i: IconIn) -> IconOut {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u)) * 2.0 - 1.0;
    var clip = u.view_proj * vec4<f32>(i.pos_size.xyz, 1.0);
    let half_px = max(i.pos_size.w * 0.5, 1.0);
    let reach = half_px + MARGIN;
    var out: IconOut;
    if (clip.w <= 0.0) {
        // Behind the camera: a degenerate quad.
        out.clip = vec4<f32>(2.0, 2.0, 0.0, 1.0);
    } else {
        clip.x += corner.x * reach * 2.0 / u.target_size.x * clip.w;
        clip.y += corner.y * reach * 2.0 / u.target_size.y * clip.w;
        // (No depth test: icons show over hills and buildings.)
        out.clip = vec4<f32>(clip.xy, 0.0, clip.w);
    }
    out.local = corner * reach / half_px;
    out.color = i.color;
    out.shape = i.shape.x;
    out.half_px = half_px;
    return out;
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(p) - b;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// Equilateral triangle pointing up, `r` = half its side (Inigo Quilez's).
fn sd_triangle(p_in: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    var p = vec2<f32>(abs(p_in.x) - r, p_in.y + r / k);
    if (p.x + k * p.y > 0.0) {
        p = vec2<f32>(p.x - k * p.y, -k * p.x - p.y) / 2.0;
    }
    p.x -= clamp(p.x, -2.0 * r, 0.0);
    return -length(p) * sign(p.y);
}

// Regular hexagon, `r` = distance to its sides (Inigo Quilez's).
fn sd_hexagon(p_in: vec2<f32>, r: f32) -> f32 {
    let k = vec3<f32>(-0.866025404, 0.5, 0.577350269);
    var p = abs(p_in);
    p -= 2.0 * min(dot(k.xy, p), 0.0) * k.xy;
    p -= vec2<f32>(clamp(p.x, -k.z * r, k.z * r), r);
    return length(p) * sign(p.y);
}

// Five-pointed star pointing up, `r` = outer radius, `rf` = inner radius / outer (Inigo
// Quilez's).
fn sd_star5(p_in: vec2<f32>, r: f32, rf: f32) -> f32 {
    let k1 = vec2<f32>(0.809016994375, -0.587785252292);
    let k2 = vec2<f32>(-k1.x, k1.y);
    var p = vec2<f32>(abs(p_in.x), p_in.y);
    p -= 2.0 * max(dot(k1, p), 0.0) * k1;
    p -= 2.0 * max(dot(k2, p), 0.0) * k2;
    p.x = abs(p.x);
    p.y -= r;
    let ba = rf * vec2<f32>(-k1.y, k1.x) - vec2<f32>(0.0, 1.0);
    let h = clamp(dot(p, ba) / dot(ba, ba), 0.0, r);
    return length(p - ba * h) * sign(p.y * ba.x - p.x * ba.y);
}

// Signed distance (units of half the icon's size) to shape `shape` at `p`.
fn shape_sdf(shape: u32, p: vec2<f32>) -> f32 {
    switch shape {
        case 1u: {
            return sd_box(p, vec2<f32>(0.64)) - 0.12;
        }
        case 2u: {
            return sd_triangle(p + vec2<f32>(0.0, 0.1), 0.86);
        }
        case 3u: {
            return (abs(p.x) + abs(p.y) - 0.94) * 0.7071;
        }
        case 4u: {
            return sd_star5(p + vec2<f32>(0.0, 0.06), 0.98, 0.46);
        }
        case 5u: {
            return sd_hexagon(p, 0.8);
        }
        case 6u: {
            return abs(length(p) - 0.84) - 0.1;
        }
        case 7u: {
            return min(abs(sd_box(p, vec2<f32>(0.6)) - 0.1) - 0.13, length(p) - 0.2);
        }
        case 8u: {
            return min(abs(length(p) - 0.72) - 0.15, length(p) - 0.2);
        }
        default: {
            return length(p) - 0.8;
        }
    }
}

@fragment
fn fs_icon(in: IconOut) -> @location(0) vec4<f32> {
    let d = shape_sdf(in.shape, in.local) * in.half_px;
    let fill = saturate(0.5 - d);
    let total = saturate(0.5 - (d - OUTLINE));
    if (total <= 0.0) {
        discard;
    }
    let shade = 1.0 + 0.2 * clamp(in.local.y, -1.0, 1.0);
    let rgb = in.color.rgb * shade * fill + vec3<f32>(0.012) * (total - fill);
    return vec4<f32>(rgb, total) * in.color.a;
}
