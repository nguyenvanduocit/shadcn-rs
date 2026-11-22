// A pyramid node and how two in order are summed (`chunk::series::Node`).

const NONE: u32 = 0xffffffffu;

struct Node {
    min: u32,
    max: u32,
    imin: u32,
    imax: u32,
    first: u32,
    last: u32,
}

fn empty_node() -> Node {
    return Node(0xffffffffu, 0u, NONE, NONE, NONE, NONE);
}

fn key_of(b: u32) -> u32 {
    if ((b & 0x80000000u) == 0u) {
        return b | 0x80000000u;
    }
    return ~b;
}

// A finite y: not a gap.
fn finite(b: u32) -> bool {
    return (b & 0x7f800000u) != 0x7f800000u;
}

fn leaf(i: u32, b: u32) -> Node {
    if (!finite(b)) {
        return empty_node();
    }
    let k = key_of(b);
    return Node(k, k, i, i, i, i);
}

// `a` followed by `b`: ties keep `a`'s, the earlier, point.
fn then(a: Node, b: Node) -> Node {
    var out = a;
    if (b.min < a.min) {
        out.min = b.min;
        out.imin = b.imin;
    }
    if (b.max > a.max) {
        out.max = b.max;
        out.imax = b.imax;
    }
    if (a.first == NONE) {
        out.first = b.first;
    }
    if (b.last != NONE) {
        out.last = b.last;
    }
    return out;
}

fn shifted(i: u32, by: u32) -> u32 {
    if (i == NONE) {
        return NONE;
    }
    return i + by;
}

// A page's layout, as a pass reads it.
struct Lay {
    len: u32,
    cap: u32,
    y: u32,
    levels: u32,
}

