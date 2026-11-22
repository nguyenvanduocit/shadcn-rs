@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> pp: PageP;
@group(0) @binding(2) var<storage, read> page: array<u32>;
@group(0) @binding(3) var<storage, read_write> sums: array<u32>;

var<workgroup> part: array<u32, 256>;

// Each block's count of points shown.
@compute @workgroup_size(256)
fn reduce(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let base = wg.x * 256u * PER + t * PER;
    var s = 0u;
    for (var j = 0u; j < PER; j += 1u) {
        if (shown(base + j)) {
            s += 1u;
        }
    }
    part[t] = s;
    workgroupBarrier();
    for (var d = 128u; d > 0u; d /= 2u) {
        if (t < d) {
            part[t] += part[t + d];
        }
        workgroupBarrier();
    }
    if (t == 0u) {
        sums[wg.x] = part[0];
    }
}
