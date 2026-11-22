// M4's columns scanned: where each column's points start in the compact list.

@group(0) @binding(0) var<uniform> m4: M4;
@group(0) @binding(1) var<storage, read> columns: array<u32>;
@group(0) @binding(2) var<storage, read_write> offsets: array<u32>;

fn scan_n() -> u32 {
    return m4.n;
}

fn count(e: u32) -> u32 {
    return columns[e * COLUMN];
}

fn scan_out(e: u32, v: u32) {
    offsets[e] = v;
}

fn scan_done(total: u32) {
}
