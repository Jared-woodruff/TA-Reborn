// Camera-depth sorting of particle draw indices. Simulation compaction uses atomics, whose
// arrival order is not stable: blending in that order made even paused smoke flicker.
// Sort each layer independently, keeping the particle payloads in place. A 64-element block
// sort is followed by parallel merges; completed layers write their final draw indices.
struct SortParams {
    depth: vec4<f32>,
    // capacity, simulated input list (the other list is drawn), merge run width, unused
    info: vec4<u32>,
};
struct Counts { words: array<u32>, };
struct Key { depth: f32, index: u32, };
@group(0) @binding(0) var<uniform> sorting: SortParams;
@group(0) @binding(1) var<storage, read> pool: array<Particle>;
@group(0) @binding(2) var<storage, read> counts: Counts;
@group(0) @binding(3) var<storage, read> input: array<Key>;
@group(0) @binding(4) var<storage, read_write> output: array<Key>;
@group(0) @binding(5) var<storage, read_write> order: array<u32>;

fn front_count() -> u32 {
    // With no flames/fireballs this layer is pure additive light: draw order is immaterial.
    return select(0u, counts.words[8u + 1u - sorting.info.y], counts.words[5] > 0u);
}
fn back_count() -> u32 { return counts.words[10u + 1u - sorting.info.y]; }
fn address(i: u32, back: bool) -> u32 { return select(i, sorting.info.x - 1u - i, back); }

// Equal-depth particles need a stable tie break independent of their compacted slot. The
// visible properties are compared first; identical particles can safely retain slot order.
fn before(a: Key, b: Key) -> bool {
    if (a.depth != b.depth) { return a.depth > b.depth; }
    if (a.index == 0xffffffffu) { return false; }
    if (b.index == 0xffffffffu) { return true; }
    let pa = pool[a.index];
    let pb = pool[b.index];
    for (var k = 0u; k < 4u; k++) {
        if (pa.data[k] != pb.data[k]) { return pa.data[k] < pb.data[k]; }
    }
    let aa = bitcast<vec4<u32>>(vec4<f32>(pa.pos, pa.age));
    let bb = bitcast<vec4<u32>>(vec4<f32>(pb.pos, pb.age));
    for (var k = 0u; k < 4u; k++) {
        if (aa[k] != bb[k]) { return aa[k] < bb[k]; }
    }
    let av = bitcast<vec4<u32>>(vec4<f32>(pa.vel, pa.life));
    let bv = bitcast<vec4<u32>>(vec4<f32>(pb.vel, pb.life));
    for (var k = 0u; k < 4u; k++) {
        if (av[k] != bv[k]) { return av[k] < bv[k]; }
    }
    return a.index < b.index;
}

var<workgroup> keys: array<Key, 64>;

@compute @workgroup_size(64)
fn cs_sort_blocks(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) local: u32) {
    let front = front_count();
    let front_groups = (front + 63u) / 64u;
    let back = group.x >= front_groups;
    let n = select(front, back_count(), back);
    let block = select(group.x, group.x - front_groups, back) * 64u;
    let i = block + local;
    var key = Key(-3.402823e38, 0xffffffffu);
    if (i < n) {
        let index = address(i, back);
        var depth = dot(sorting.depth, vec4<f32>(pool[index].pos, 1.0));
        // Keep the comparator total even if an extreme presentation-only burst overflows.
        if (!(abs(depth) <= 3.402823e38)) { depth = -3.402823e38; }
        key = Key(depth, index);
    }
    keys[local] = key;
    workgroupBarrier();
    for (var width = 2u; width <= 64u; width *= 2u) {
        for (var gap = width / 2u; gap > 0u; gap /= 2u) {
            let a = keys[local];
            let b = keys[local ^ gap];
            let ascending = (local & width) == 0u;
            let lower = (local & gap) == 0u;
            let take_b = select(before(a, b), before(b, a), ascending == lower);
            workgroupBarrier();
            keys[local] = a;
            if (take_b) { keys[local] = b; }
            workgroupBarrier();
        }
    }
    if (i < n) {
        let index = address(i, back);
        output[index] = keys[local];
        if (n <= 64u) { order[index] = keys[local].index; }
    }
}

@compute @workgroup_size(64)
fn cs_merge(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) local: u32) {
    let front = front_count();
    let front_groups = (front + 63u) / 64u;
    let back = group.x >= front_groups;
    let n = select(front, back_count(), back);
    let i = select(group.x, group.x - front_groups, back) * 64u + local;
    let width = sorting.info.z;
    if (i >= n || width >= n) { return; }
    let key = input[address(i, back)];
    let block = (i / (width * 2u)) * (width * 2u);
    let left = i - block < width;
    let other = min(block + select(0u, width, left), n);
    let mine = block + select(width, 0u, left);
    var lo = other;
    var hi = min(other + width, n);
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (before(input[address(mid, back)], key)) { lo = mid + 1u; }
        else { hi = mid; }
    }
    let destination = address(block + (i - mine) + (lo - other), back);
    output[destination] = key;
    if (width * 2u >= n) { order[destination] = key.index; }
}
