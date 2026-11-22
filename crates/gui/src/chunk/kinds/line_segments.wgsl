// A line's segments laid out once a frame (ADR-0018): for every piece of
// the compact M4 points (a sixteenth of a segment, on a natural curve) its
// ends, its end planes, its box, and its length; then, for dashes, where
// along the line each starts, by an ordered segmented prefix sum (a break
// starts the pattern again, as Skia restarts it at every contour).

@group(0) @binding(0) var<uniform> style: Style;
@group(0) @binding(1) var<storage, read> points: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read> offsets: array<u32>;
@group(0) @binding(3) var<storage, read> ctrl: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read> fctrl: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read_write> segs: array<Seg>;

fn hide(i: u32) {
    var s: Seg;
    s.ab = vec4<f32>(0.0);
    s.ea = vec4<f32>(0.0);
    s.eb = vec4<f32>(0.0);
    s.ba = vec4<f32>(0.0);
    s.bb = vec4<f32>(0.0);
    s.bx = vec4<f32>(1.0, 1.0, 0.0, 0.0);
    // Not drawn, no length: the run goes on through it.
    s.arc = vec4<f32>(0.0);
    segs[i] = s;
}

@compute @workgroup_size(64)
fn segments(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x + gid.y * 65535u * 64u;
    if (i >= style.cap) {
        return;
    }
    let sub = max(style.sub, 1u);
    let k = i / sub;
    let j = i % sub;
    if (k >= total() || !connected(k)) {
        hide(i);
        return;
    }
    let c = curved(k);
    if (!c && j > 0u) {
        hide(i);
        return;
    }
    var n = 1u;
    if (c) {
        n = sub;
    }
    let fs = f32(n);
    let a = at_t(k, f32(j) / fs, false);
    let b = at_t(k, f32(j + 1u) / fs, false);
    let len = length(b - a);
    if (len < 1e-5) {
        hide(i);
        return;
    }
    let d = (b - a) / len;
    // The neighbours, for the joins.
    var has_p = false;
    var p = a;
    if (j > 0u) {
        has_p = true;
        p = at_t(k, f32(j - 1u) / fs, false);
    } else if (k > 0u && connected(k - 1u)) {
        has_p = true;
        if (curved(k - 1u)) {
            p = at_t(k - 1u, f32(sub - 1u) / f32(sub), false);
        } else {
            p = pt(k - 1u).xy;
        }
    }
    var has_q = false;
    var q = b;
    if (c && j + 1u < n) {
        has_q = true;
        q = at_t(k, f32(j + 2u) / fs, false);
    } else if (connected(k + 1u)) {
        has_q = true;
        if (curved(k + 1u)) {
            q = at_t(k + 1u, 1.0 / f32(sub), false);
        } else {
            q = pt(k + 2u).xy;
        }
    }
    has_p = has_p && length(a - p) > 1e-5;
    has_q = has_q && length(q - b) > 1e-5;
    var dp = d;
    if (has_p) {
        dp = normalize(a - p);
    }
    var dq = d;
    if (has_q) {
        dq = normalize(q - b);
    }
    let ea = end_of(a, d, dp, has_p);
    // The far end, seen from the next segment going back.
    let eb = end_of(b, -d, -dq, has_q);
    let hw = style.hw;
    let nrm = perp(d);
    // The box of the quad the coverage lies in.
    var lo = vec2<f32>(1e30);
    var hi = vec2<f32>(-1e30);
    for (var v = 0u; v < 4u; v += 1u) {
        let along = select(-ea.reach, len + eb.reach, (v & 1u) != 0u);
        let across = select(-(hw + 1.0), hw + 1.0, (v & 2u) != 0u);
        let at = a + d * along + nrm * across;
        lo = min(lo, at);
        hi = max(hi, at);
    }
    var s: Seg;
    s.ab = vec4<f32>(a, b);
    s.ea = ea.plane;
    s.eb = eb.plane;
    s.ba = ea.bevel;
    s.bb = eb.bevel;
    s.bx = vec4<f32>(lo, hi);
    // Its length, and whether a break comes before it (no neighbour).
    s.arc = vec4<f32>(0.0, len, select(1.0, 0.0, has_p || (k > 0u && j == 0u && connected(k - 1u))), 0.0);
    segs[i] = s;
}

// Where along its run each segment starts: an exclusive prefix sum of
// lengths that starts again after every break, in one workgroup and a
// fixed order (each thread a run of consecutive segments, the runs joined
// by a Hillis-Steele scan of (sum, broken) pairs).
var<workgroup> run_sum: array<f32, 256>;
var<workgroup> run_cut: array<u32, 256>;

@compute @workgroup_size(256)
fn arcs(@builtin(local_invocation_index) t: u32) {
    let n = style.cap;
    let per = (n + 255u) / 256u;
    let start = t * per;
    // The run's own: its total after its last break, and whether it has one.
    var sum = 0.0;
    var cut = 0u;
    for (var e = start; e < min(start + per, n); e += 1u) {
        let a = segs[e].arc;
        if (a.z != 0.0) {
            sum = 0.0;
            cut = 1u;
        }
        sum += a.y;
    }
    run_sum[t] = sum;
    run_cut[t] = cut;
    workgroupBarrier();
    for (var d = 1u; d < 256u; d *= 2u) {
        var s = 0.0;
        var c = 0u;
        if (t >= d) {
            s = run_sum[t - d];
            c = run_cut[t - d];
        }
        workgroupBarrier();
        if (t >= d && run_cut[t] == 0u) {
            run_sum[t] += s;
            run_cut[t] = c;
        }
        workgroupBarrier();
    }
    var at = 0.0;
    if (t > 0u) {
        at = run_sum[t - 1u];
    }
    for (var e = start; e < min(start + per, n); e += 1u) {
        var a = segs[e].arc;
        if (a.z != 0.0) {
            at = 0.0;
        }
        a.x = at;
        at += a.y;
        segs[e].arc = a;
    }
}
