@group(0) @binding(0) var<uniform> m4: M4;
@group(0) @binding(1) var<storage, read> table: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> partials: array<u32>;
// Per column: x first, x last, open, high, low, close pixels, points (0:
// none, 1: one, 2: several), 0.
@group(0) @binding(3) var<storage, read_write> candles: array<vec4<u32>>;

@compute @workgroup_size(64)
fn merge(@builtin(global_invocation_id) gid: vec3<u32>) {
    let c = gid.x;
    if (c >= m4.cols) {
        return;
    }
    var first = NONE;
    var last = NONE;
    var hk = 0u;
    var lk = 0xffffffffu;
    var v = array<u32, 6>(0u, 0u, 0u, 0u, 0u, 0u);
    for (var k = 0u; k < m4.npages; k += 1u) {
        let t = table[k];
        if (c < t.x || c >= t.y) {
            continue;
        }
        let s = (t.z + c - t.x) * PARTIAL;
        if (partials[s] == NONE) {
            continue;
        }
        if (first == NONE) {
            first = partials[s];
            v[0] = partials[s + 6u];
            v[2] = partials[s + 8u];
        }
        last = partials[s + 1u];
        v[1] = partials[s + 7u];
        v[5] = partials[s + 11u];
        if (partials[s + 2u] > hk) {
            hk = partials[s + 2u];
            v[3] = partials[s + 9u];
        }
        if (partials[s + 4u] < lk) {
            lk = partials[s + 4u];
            v[4] = partials[s + 10u];
        }
    }
    var count = 0u;
    if (first != NONE) {
        count = 1u;
        if (last != first) {
            count = 2u;
        }
    }
    candles[c * 2u] = vec4<u32>(v[0], v[1], v[2], v[3]);
    candles[c * 2u + 1u] = vec4<u32>(v[4], v[5], count, 0u);
}
