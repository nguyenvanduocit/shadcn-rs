// M4's points, compacted in column order: two vec4s a point, (x pixel,
// y pixel, floor pixel, index) and (its extended column, 0, 0, 0).

@group(0) @binding(0) var<uniform> m4: M4;
@group(0) @binding(1) var<storage, read> columns: array<u32>;
@group(0) @binding(2) var<storage, read> offsets: array<u32>;
@group(0) @binding(3) var<storage, read_write> points: array<vec4<u32>>;

@compute @workgroup_size(64)
fn compact(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if (e >= m4.n) {
        return;
    }
    let s = e * COLUMN;
    let n = columns[s];
    let base = offsets[e];
    for (var j = 0u; j < n; j += 1u) {
        let at = s + 1u + j * 4u;
        let k = (base + j) * 2u;
        points[k] = vec4<u32>(columns[at], columns[at + 1u], columns[at + 2u], columns[at + 3u]);
        points[k + 1u] = vec4<u32>(e, 0u, 0u, 0u);
    }
}
