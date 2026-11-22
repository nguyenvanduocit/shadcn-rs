// An exclusive prefix sum in one workgroup, in order (ADR-0018: never an
// atomic append): `count(i)` for i below `scan_n()`, into `scan_out(i)`,
// the total at `scan_out(n)`. Each of 256 threads sums a run of
// consecutive entries, the runs' sums are scanned in shared memory
// (Hillis–Steele), then each thread writes its run.

var<workgroup> scan_sums: array<u32, 256>;

@compute @workgroup_size(256)
fn scan(@builtin(local_invocation_index) t: u32) {
    let n = scan_n();
    let per = (n + 255u) / 256u;
    let start = t * per;
    var s = 0u;
    for (var i = 0u; i < per; i += 1u) {
        let e = start + i;
        if (e < n) {
            s += count(e);
        }
    }
    scan_sums[t] = s;
    workgroupBarrier();
    for (var d = 1u; d < 256u; d *= 2u) {
        var v = 0u;
        if (t >= d) {
            v = scan_sums[t - d];
        }
        workgroupBarrier();
        scan_sums[t] += v;
        workgroupBarrier();
    }
    var run = 0u;
    if (t > 0u) {
        run = scan_sums[t - 1u];
    }
    for (var i = 0u; i < per; i += 1u) {
        let e = start + i;
        if (e < n) {
            scan_out(e, run);
            run += count(e);
        }
    }
    if (t == 255u) {
        scan_out(n, scan_sums[255]);
        scan_done(scan_sums[255]);
    }
}
