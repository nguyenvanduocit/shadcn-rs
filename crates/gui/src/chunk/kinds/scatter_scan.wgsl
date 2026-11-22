@group(0) @binding(0) var<uniform> pp: PageP;
@group(0) @binding(1) var<storage, read> sums: array<u32>;
@group(0) @binding(2) var<storage, read_write> starts: array<u32>;
@group(0) @binding(3) var<storage, read_write> args: array<u32>;

fn scan_n() -> u32 {
    return pp.blocks;
}

fn count(e: u32) -> u32 {
    return sums[e];
}

fn scan_out(e: u32, v: u32) {
    starts[e] = v;
}

fn scan_done(total: u32) {
    let a = pp.slot * 4u;
    args[a] = 4u;
    args[a + 1u] = total;
    args[a + 2u] = 0u;
    args[a + 3u] = 0u;
}
