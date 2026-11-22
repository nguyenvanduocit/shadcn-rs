@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> pp: PageP;
@group(0) @binding(2) var<storage, read> page: array<u32>;
@group(0) @binding(3) var<storage, read> starts: array<u32>;
@group(0) @binding(4) var<storage, read_write> shown_out: array<u32>;

var<workgroup> part: array<u32, 256>;

// The points shown, in order: each thread's run counted, the runs scanned
// in the workgroup, each point written at its block's start plus what
// precedes it.
@compute @workgroup_size(256)
fn compact(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) t: u32) {
    let base = wg.x * 256u * PER + t * PER;
    var s = 0u;
    for (var j = 0u; j < PER; j += 1u) {
        if (shown(base + j)) {
            s += 1u;
        }
    }
    part[t] = s;
    workgroupBarrier();
    for (var d = 1u; d < 256u; d *= 2u) {
        var v = 0u;
        if (t >= d) {
            v = part[t - d];
        }
        workgroupBarrier();
        part[t] += v;
        workgroupBarrier();
    }
    var at = starts[wg.x];
    if (t > 0u) {
        at += part[t - 1u];
    }
    for (var j = 0u; j < PER; j += 1u) {
        if (shown(base + j)) {
            shown_out[at] = pp.first + base + j;
            at += 1u;
        }
    }
}
