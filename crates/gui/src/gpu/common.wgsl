// What every pipeline of the renderer shares: signed distances and area
// coverage of rounded rectangles, and colour (sRGB, OKLab, premultiplying).
// Prepended to each module by `gpu::module`.

const M_PI_F: f32 = 3.1415926;

fn pick_corner(center_to_point: vec2<f32>, radii: vec4<f32>) -> f32 {
    // radii: top left, top right, bottom right, bottom left.
    if (center_to_point.x < 0.0) {
        if (center_to_point.y < 0.0) {
            return radii.x;
        }
        return radii.w;
    }
    if (center_to_point.y < 0.0) {
        return radii.y;
    }
    return radii.z;
}

// Signed distance from `p` to a rounded rectangle, negative inside.
fn rrect_sdf(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    let half_size = rect.zw / 2.0;
    let center = rect.xy + half_size;
    let center_to_point = p - center;
    let r = min(pick_corner(center_to_point, radii), min(half_size.x, half_size.y));
    let corner_to_point = abs(center_to_point) - half_size;
    let q = corner_to_point + r;
    if (r <= 0.0) {
        return max(q.x, q.y);
    }
    return length(max(vec2<f32>(0.0), q)) + min(0.0, max(q.x, q.y)) - r;
}

// How much of the pixel centred on `p` a rounded rectangle covers.
fn rrect_cov(p: vec2<f32>, rect: vec4<f32>, radii: vec4<f32>) -> f32 {
    if (rect.z <= 0.0 || rect.w <= 0.0) {
        return 0.0;
    }
    let corners = max(max(radii.x, radii.y), max(radii.z, radii.w));
    if (corners <= 0.0) {
        // An axis-aligned rectangle covers the pixel's area exactly.
        let lo = clamp(p - 0.5, rect.xy, rect.xy + rect.zw);
        let hi = clamp(p + 0.5, rect.xy, rect.xy + rect.zw);
        let size = max(hi - lo, vec2<f32>(0.0));
        return size.x * size.y;
    }
    return saturate(0.5 - rrect_sdf(p, rect, radii));
}

fn premul(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn to_gamma(linear: f32) -> f32 {
    let c = clamp(linear, 0.0, 1.0);
    if (c <= 0.0031308) {
        return 12.92 * c;
    }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

fn cbrt(x: f32) -> f32 {
    return sign(x) * pow(abs(x), 1.0 / 3.0);
}

fn to_oklab(c: vec3<f32>) -> vec3<f32> {
    let r = to_linear(c.r);
    let g = to_linear(c.g);
    let b = to_linear(c.b);
    let l = cbrt(0.41222146 * r + 0.53633255 * g + 0.051445995 * b);
    let m = cbrt(0.2119035 * r + 0.6806995 * g + 0.10739696 * b);
    let s = cbrt(0.08830246 * r + 0.28171885 * g + 0.6299787 * b);
    return vec3<f32>(
        0.21045426 * l + 0.7936178 * m - 0.004072047 * s,
        1.9779985 * l - 2.4285922 * m + 0.4505937 * s,
        0.025904037 * l + 0.78277177 * m - 0.80867577 * s,
    );
}

fn from_oklab(lab: vec3<f32>) -> vec3<f32> {
    let l_ = lab.x + 0.39633778 * lab.y + 0.21580376 * lab.z;
    let m_ = lab.x - 0.105561346 * lab.y - 0.06385417 * lab.z;
    let s_ = lab.x - 0.08948418 * lab.y - 1.2914855 * lab.z;
    let l3 = l_ * l_ * l_;
    let m3 = m_ * m_ * m_;
    let s3 = s_ * s_ * s_;
    return vec3<f32>(
        to_gamma(4.0767417 * l3 - 3.3077116 * m3 + 0.23096994 * s3),
        to_gamma(-1.268438 * l3 + 2.6097574 * m3 - 0.34131938 * s3),
        to_gamma(-0.0041960863 * l3 - 0.7034186 * m3 + 1.7076147 * s3),
    );
}

// `a` to `b` by `t` in OKLab with premultiplied alpha (`Color::mix_oklab`).
fn mix_oklab(a: vec4<f32>, b: vec4<f32>, t: f32) -> vec4<f32> {
    let alpha = a.a + (b.a - a.a) * t;
    if (alpha <= 0.0) {
        return vec4<f32>(0.0);
    }
    let lab_a = to_oklab(a.rgb);
    let lab_b = to_oklab(b.rgb);
    let lab = (lab_a * a.a + (lab_b * b.a - lab_a * a.a) * t) / alpha;
    return vec4<f32>(from_oklab(lab), alpha);
}
