// M4, merged (ADR-0018): each extended column's partials, page by page in
// order, summed as the pyramid sums (ties keep the earlier point), and its
// points written in index order: first, least and greatest, last, each once.

@group(0) @binding(0) var<uniform> m4: M4;
// Per page: its first and one past its last extended column, its first partial.
@group(0) @binding(1) var<storage, read> table: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> partials: array<u32>;
@group(0) @binding(3) var<storage, read_write> columns: array<u32>;

const PARTIAL: u32 = 24u;

struct Part {
    node: Node,
    // first, least, greatest, last: (x, y, floor) pixels.
    p: array<vec3<f32>, 4>,
}

fn load(k: u32) -> Part {
    let s = k * PARTIAL;
    var out: Part;
    out.node = Node(partials[s], partials[s + 1u], partials[s + 2u], partials[s + 3u], partials[s + 4u], partials[s + 5u]);
    for (var j = 0u; j < 4u; j += 1u) {
        let at = s + 6u + j * 3u;
        out.p[j] = vec3<f32>(bitcast<f32>(partials[at]), bitcast<f32>(partials[at + 1u]), bitcast<f32>(partials[at + 2u]));
    }
    return out;
}

fn join(a: Part, b: Part) -> Part {
    var out = a;
    if (b.node.min < a.node.min) {
        out.node.min = b.node.min;
        out.node.imin = b.node.imin;
        out.p[1] = b.p[1];
    }
    if (b.node.max > a.node.max) {
        out.node.max = b.node.max;
        out.node.imax = b.node.imax;
        out.p[2] = b.p[2];
    }
    if (a.node.first == NONE) {
        out.node.first = b.node.first;
        out.p[0] = b.p[0];
    }
    if (b.node.last != NONE) {
        out.node.last = b.node.last;
        out.p[3] = b.p[3];
    }
    return out;
}

@compute @workgroup_size(64)
fn merge(@builtin(global_invocation_id) gid: vec3<u32>) {
    let e = gid.x;
    if (e >= m4.n) {
        return;
    }
    var acc: Part;
    acc.node = empty_node();
    for (var k = 0u; k < m4.npages; k += 1u) {
        let t = table[k];
        if (e >= t.x && e < t.y) {
            acc = join(acc, load(t.z + e - t.x));
        }
    }
    let s = e * COLUMN;
    var n = 0u;
    if (acc.node.first != NONE) {
        var order = array<u32, 4>(0u, 1u, 2u, 3u);
        var idx = array<u32, 4>(acc.node.first, acc.node.imin, acc.node.imax, acc.node.last);
        if (idx[1] > idx[2]) {
            order[1] = 2u;
            order[2] = 1u;
        }
        var last = NONE;
        for (var j = 0u; j < 4u; j += 1u) {
            let o = order[j];
            let i = idx[o];
            if (n == 0u || i != last) {
                let at = s + 1u + n * 4u;
                columns[at] = bitcast<u32>(acc.p[o].x);
                columns[at + 1u] = bitcast<u32>(acc.p[o].y);
                columns[at + 2u] = bitcast<u32>(acc.p[o].z);
                columns[at + 3u] = i;
                n += 1u;
                last = i;
            }
        }
    }
    columns[s] = n;
}
