// x as an i64 in two words (high: the sign's, low), less the view's first
// x, as a float: exact while the difference fits a float's 24 bits, as
// near as a float holds beyond, on either side of the view's start (a
// negative difference is negated in 64 bits first, never summed from
// two large halves).
fn x_delta(hi: u32, lo: u32, x0: vec2<u32>) -> f32 {
    var dlo = lo - x0.y;
    var borrow = 0u;
    if (lo < x0.y) {
        borrow = 1u;
    }
    var dhi = hi - x0.x - borrow;
    let negative = bitcast<i32>(dhi) < 0;
    if (negative) {
        dlo = ~dlo + 1u;
        var carry = 0u;
        if (dlo == 0u) {
            carry = 1u;
        }
        dhi = ~dhi + carry;
    }
    let v = f32(dhi) * 4294967296.0 + f32(dlo);
    if (negative) {
        return -v;
    }
    return v;
}
