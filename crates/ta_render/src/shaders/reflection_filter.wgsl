// The reflection's filter chain (see reflection.rs), run right after the reflection pass:
// level by level it halves the mirrored world's colour into the target's mip chain (an area
// average of premultiplied colour), so the water can look it up filtered by how fast its
// lookup moves across the screen, and beside it builds the reach: how far beyond the water
// surface the mirrored world lies along the eye's rays (`MIRROR_REACH` where nothing was
// mirrored), each level keeping the nearest of the four below it, so the reach the water
// smooths over a few texels never lets far things win over near ones. Prepended: common.wgsl
// (the scene's frame uniform, group 0), fullscreen.wgsl.

// As in water.wgsl.
const MIRROR_REACH: f32 = 400.0;

// The first level: the reflection itself and its depth.
@group(1) @binding(0) var source_color: texture_2d<f32>;
@group(1) @binding(1) var source_depth: texture_depth_2d;
// The levels after it: the level above's colour (binding 0) and reach.
@group(1) @binding(2) var source_reach: texture_2d<f32>;

struct Halved {
    @location(0) color: vec4<f32>,
    @location(1) reach: f32,
};

struct Footprint {
    lo: vec2<f32>,
    hi: vec2<f32>,
    begin: vec2<i32>,
    end: vec2<i32>,
};

// Mips round their dimensions down. A fixed 2 x 2 reduction would crop odd source dimensions
// and stretch their remaining texels across the next mip, making reflections slide when the
// waves change the selected mip. Cover the same normalized rectangle at every level instead.
fn footprint(p: vec2<i32>, size: vec2<i32>) -> Footprint {
    let scale = vec2<f32>(size) / vec2<f32>(max(size / 2, vec2<i32>(1)));
    let lo = vec2<f32>(p) * scale;
    let hi = min(vec2<f32>(p + 1) * scale, vec2<f32>(size));
    return Footprint(lo, hi, vec2<i32>(floor(lo)), vec2<i32>(ceil(hi)));
}

fn area_weight(t: vec2<i32>, f: Footprint) -> f32 {
    let overlap = max(min(vec2<f32>(t + 1), f.hi) - max(vec2<f32>(t), f.lo), vec2<f32>(0.0));
    return overlap.x * overlap.y;
}

// The reach at reflection texel `t`: the depth there unprojected with the scene camera gives the
// mirror image (below the water) of what the texel shows, on the eye's ray through it; its
// distance from where that ray meets the water.
fn reach_at(t: vec2<i32>, size: vec2<f32>) -> f32 {
    let z = textureLoad(source_depth, t, 0);
    if (z >= 1.0) {
        return MIRROR_REACH;
    }
    let uv = (vec2<f32>(t) + 0.5) / size;
    let h = frame.inv_view_proj * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, z, 1.0);
    let image = h.xyz / h.w;
    let eye = frame.camera_pos.xyz;
    let ray = image - eye;
    if (ray.y > -1e-4) {
        return 0.0;
    }
    let water = eye + ray * ((frame.world.y - eye.y) / ray.y);
    return min(distance(image, water), MIRROR_REACH);
}

fn halve_first(p: vec2<i32>) -> Halved {
    let size = vec2<f32>(textureDimensions(source_depth));
    let f = footprint(p, vec2<i32>(size));
    var out: Halved;
    out.reach = MIRROR_REACH;
    for (var y = f.begin.y; y < f.end.y; y++) {
        for (var x = f.begin.x; x < f.end.x; x++) {
            let t = vec2<i32>(x, y);
            let weight = area_weight(t, f);
            if (weight > 0.0) {
                out.color += textureLoad(source_color, t, 0) * weight;
                out.reach = min(out.reach, reach_at(t, size));
            }
        }
    }
    out.color /= (f.hi.x - f.lo.x) * (f.hi.y - f.lo.y);
    return out;
}

@fragment
fn fs_reflection_first(in: VsOut) -> Halved {
    return halve_first(vec2<i32>(in.pos.xy));
}

fn halve_down(p: vec2<i32>) -> Halved {
    let f = footprint(p, vec2<i32>(textureDimensions(source_color)));
    var out: Halved;
    out.reach = MIRROR_REACH;
    for (var y = f.begin.y; y < f.end.y; y++) {
        for (var x = f.begin.x; x < f.end.x; x++) {
            let t = vec2<i32>(x, y);
            let weight = area_weight(t, f);
            if (weight > 0.0) {
                out.color += textureLoad(source_color, t, 0) * weight;
                out.reach = min(out.reach, textureLoad(source_reach, t, 0).r);
            }
        }
    }
    out.color /= (f.hi.x - f.lo.x) * (f.hi.y - f.lo.y);
    return out;
}

@fragment
fn fs_reflection_down(in: VsOut) -> Halved {
    return halve_down(vec2<i32>(in.pos.xy));
}
