// A series page's points and pyramid (`chunk::series`, `chunk::store`), read
// on the GPU exactly as the CPU reads them: a node is a range's least and
// greatest y (as order keys of y's bits), where they are (the earliest of
// equals), and its first and last valid point. `PAGE` is replaced by the
// page binding's name, and `PAGE_w(i)` reads its word `i` (one binding, or
// one of a gang's, `m4.rs`); the shared part is emitted once.

fn PAGE_node(l: Lay, level: u32, j: u32, off: u32) -> Node {
    if (level == 0u) {
        return leaf(j, PAGE_w(l.y + j));
    }
    let at = off + j * 6u;
    return Node(PAGE_w(at), PAGE_w(at + 1u), PAGE_w(at + 2u), PAGE_w(at + 3u), PAGE_w(at + 4u), PAGE_w(at + 5u));
}

// Point i's x is below (hi, lo).
fn PAGE_below(l: Lay, i: u32, hi: u32, lo: u32) -> bool {
    let xh = bitcast<i32>(PAGE_w(i));
    let h = bitcast<i32>(hi);
    if (xh != h) {
        return xh < h;
    }
    return PAGE_w(l.cap + i) < lo;
}

// The first point at or after x = (hi, lo).
fn PAGE_lower_bound(l: Lay, x: vec2<u32>) -> u32 {
    var a = 0u;
    var b = l.len;
    loop {
        if (a >= b) {
            break;
        }
        let m = a + (b - a) / 2u;
        if (PAGE_below(l, m, x.x, x.y)) {
            a = m + 1u;
        } else {
            b = m;
        }
    }
    return a;
}

// Points lo..hi summed up, in order, through the pyramid.
fn PAGE_fold(l: Lay, lo: u32, hi: u32) -> Node {
    var left = empty_node();
    var right = empty_node();
    var a = lo;
    var b = min(hi, l.len);
    var level = 0u;
    var off = 0u;
    var count = l.cap;
    loop {
        if (a >= b) {
            break;
        }
        loop {
            if (a >= b || a % 64u == 0u) {
                break;
            }
            left = then(left, PAGE_node(l, level, a, off));
            a += 1u;
        }
        loop {
            if (a >= b || b % 64u == 0u) {
                break;
            }
            b -= 1u;
            right = then(PAGE_node(l, level, b, off), right);
        }
        if (a >= b) {
            break;
        }
        a = a / 64u;
        b = b / 64u;
        if (level == 0u) {
            off = l.levels;
        } else {
            off = off + count * 6u;
        }
        count = (count + 63u) / 64u;
        level += 1u;
    }
    return then(left, right);
}
