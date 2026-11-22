//! The quad pipeline's instance records (ADR-0016): typed, compact, and
//! written as `u32` words, a plain-old-data layout `bytemuck` casts to bytes
//! with no copy and no `unsafe` of ours. A record starts with its kind and
//! flags, then its state slot (`shaders.wgsl`'s `vs_main` reads the
//! transform, clip and effect from the slot), then its colour as four
//! halves and its geometry in its chunk's paint space, physical pixels.
//!
//! | record | words | bytes |
//! | --- | --- | --- |
//! | glyph or icon from an atlas | 8 | 32 |
//! | rectangle | 8 | 32 |
//! | rounded rectangle | 12 | 48 |
//! | band (border, ring) | 20 | 80 |
//! | shadow | 21 | 84 |
//! | image | 16 | 64 |
//! | gradient | 16 + 3 a stop | |
//!
//! The 256-byte primitive it replaces held all of them in sixteen `vec4`s.

pub(crate) const KIND_SOLID: u32 = 0;
pub(crate) const KIND_SHADOW: u32 = 1;
pub(crate) const KIND_GRADIENT: u32 = 2;
pub(crate) const KIND_MONO: u32 = 3;
pub(crate) const KIND_COLOR: u32 = 4;
pub(crate) const KIND_IMAGE: u32 = 5;
pub(crate) const KIND_MASK: u32 = 6;
pub(crate) const KIND_LAYER: u32 = 7;
pub(crate) const KIND_TEXT_THROUGH: u32 = 8;
pub(crate) const KIND_LCD: u32 = 9;

/// A solid record has radii (a text-through one: a background; a layer: a mask).
pub(crate) const F_RADII: u32 = 1 << 8;
/// A solid record has an inner rectangle cut out (a text-through one: a gradient).
pub(crate) const F_INNER: u32 = 1 << 9;

/// `v` as an IEEE half, rounded to nearest, ties to even.
pub(crate) fn half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exp = (bits >> 23) & 0xff;
    let mant = bits & 0x007f_ffff;
    let h = if exp == 0xff {
        // Infinity or NaN.
        0x7c00 | if mant == 0 { 0 } else { 0x200 }
    } else if exp >= 143 {
        0x7c00
    } else if exp > 112 {
        let h = (exp.saturating_sub(112) << 10) | (mant >> 13);
        let rest = mant & 0x1fff;
        if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) {
            h.saturating_add(1)
        } else {
            h
        }
    } else if exp >= 102 {
        let m = mant | 0x0080_0000;
        let shift = 126_u32.saturating_sub(exp);
        let h = m >> shift;
        let half_bit = 1_u32 << shift.saturating_sub(1);
        let rest = m & ((1_u32 << shift).saturating_sub(1));
        if rest > half_bit || (rest == half_bit && h & 1 == 1) {
            h.saturating_add(1)
        } else {
            h
        }
    } else {
        0
    };
    u16::try_from(sign | h).unwrap_or(0)
}

/// A straight colour as two words of halves (`unpack2x16float`).
pub(crate) fn color(c: [f32; 4]) -> [u32; 2] {
    let pack = |a: f32, b: f32| u32::from(half(a)) | (u32::from(half(b)) << 16);
    [pack(c[0], c[1]), pack(c[2], c[3])]
}

fn f(v: [f32; 4]) -> [u32; 4] {
    v.map(f32::to_bits)
}

fn pair(a: u32, b: u32) -> u32 {
    (a & 0xffff) | ((b & 0xffff) << 16)
}

/// A gradient's stops: an offset and a straight colour each.
pub(crate) type Stops = Vec<(f32, [f32; 4])>;

/// A glyph or icon read from an atlas page.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Glyph {
    pub kind: u32,
    pub page: u32,
    /// Gamma table rows: the colour's luminance, then red, green, blue
    /// (subpixel text).
    pub rows: [u32; 4],
    pub slot: u32,
    pub color: [f32; 4],
    /// Its top left (physical, in its space) and size.
    pub at: (f32, f32),
    pub size: (u32, u32),
    /// Where it lies on its atlas page.
    pub texel: (u32, u32),
}

impl Glyph {
    pub(crate) fn words(&self) -> [u32; 8] {
        let [lum, r, g, b] = self.rows.map(|v| v.min(15));
        let head = (self.kind & 0xff) | ((self.page & 0xff) << 8) | (lum << 16) | (r << 20) | (g << 24) | (b << 28);
        let [c0, c1] = color(self.color);
        [
            head,
            self.slot,
            c0,
            c1,
            self.at.0.to_bits(),
            self.at.1.to_bits(),
            pair(self.size.0, self.size.1),
            pair(self.texel.0, self.texel.1),
        ]
    }
}

/// A rectangle, a rounded one, or what lies between two (a border).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Solid {
    pub slot: u32,
    pub color: [f32; 4],
    pub rect: [f32; 4],
    pub radii: Option<[f32; 4]>,
    /// The rectangle cut out of it and its radii.
    pub inner: Option<([f32; 4], [f32; 4])>,
}

impl Solid {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        let radii = self.radii.or(self.inner.map(|_| [0.0; 4]));
        let flags = radii.map_or(0, |_| F_RADII) | self.inner.map_or(0, |_| F_INNER);
        let [c0, c1] = color(self.color);
        out.extend([KIND_SOLID | flags, self.slot, c0, c1]);
        out.extend(f(self.rect));
        if let Some(r) = radii {
            out.extend(f(r));
        }
        if let Some((rect, r)) = self.inner {
            out.extend(f(rect));
            out.extend(f(r));
        }
    }
}

/// A box shadow: the shape it casts (with its radii), the box casting it,
/// its blur's sigma.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shadow {
    pub slot: u32,
    pub color: [f32; 4],
    pub shape: [f32; 4],
    pub shape_radii: [f32; 4],
    pub caster: [f32; 4],
    pub caster_radii: [f32; 4],
    pub sigma: f32,
}

impl Shadow {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        let [c0, c1] = color(self.color);
        out.extend([KIND_SHADOW, self.slot, c0, c1]);
        out.extend(f(self.shape));
        out.extend(f(self.shape_radii));
        out.extend(f(self.caster));
        out.extend(f(self.caster_radii));
        out.push(self.sigma.to_bits());
    }
}

/// Stops inline: an offset and a colour each.
fn push_stops(out: &mut Vec<u32>, stops: &[(f32, [f32; 4])]) {
    for (offset, c) in stops {
        out.push(offset.to_bits());
        out.extend(color(*c));
    }
}

/// A gradient filling a rounded rectangle, along `line` (physical).
#[derive(Clone, Debug)]
pub(crate) struct Gradient {
    pub slot: u32,
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub line: [f32; 4],
    pub stops: Stops,
}

impl Gradient {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        let n = u32::try_from(self.stops.len().min(255)).unwrap_or(0);
        out.extend([KIND_GRADIENT | (n << 8), self.slot, 0, 0]);
        out.extend(f(self.rect));
        out.extend(f(self.radii));
        out.extend(f(self.line));
        push_stops(out, self.stops.get(..255).unwrap_or(&self.stops));
    }
}

/// An image covering a rounded rectangle, the whole of it landing on `placed`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Image {
    pub slot: u32,
    pub rect: [f32; 4],
    pub radii: [f32; 4],
    pub placed: [f32; 4],
}

impl Image {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        out.extend([KIND_IMAGE, self.slot, 0, 0]);
        out.extend(f(self.rect));
        out.extend(f(self.radii));
        out.extend(f(self.placed));
    }
}

/// Text showing a fill through its glyphs: its region, the node's
/// background beneath, and where the gradient shows.
#[derive(Clone, Debug)]
pub(crate) struct TextThrough {
    pub slot: u32,
    pub region: [f32; 4],
    pub background: Option<([f32; 4], [f32; 4])>,
    /// Where it shows, its line, its stops.
    pub fill: Option<([f32; 4], [f32; 4], Stops)>,
}

impl TextThrough {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        let n = self
            .fill
            .as_ref()
            .map_or(0, |f| u32::try_from(f.2.len().min(255)).unwrap_or(0));
        let flags = self.background.map_or(0, |_| F_RADII) | self.fill.as_ref().map_or(0, |_| F_INNER) | (n << 16);
        let (c, bg) = self.background.unwrap_or(([0.0; 4], [0.0; 4]));
        let [c0, c1] = color(c);
        out.extend([KIND_TEXT_THROUGH | flags, self.slot, c0, c1]);
        out.extend(f(self.region));
        out.extend(f(bg));
        let (shown, line, stops) = self.fill.clone().unwrap_or(([0.0; 4], [0.0; 4], Vec::new()));
        out.extend(f(shown));
        out.extend(f(line));
        push_stops(out, stops.get(..255).unwrap_or(&stops));
    }
}

/// A layer composited: the quad it covers in the window, its region, the
/// inverse of its transform, two filters, its mask.
#[derive(Clone, Debug)]
pub(crate) struct Layer {
    pub slot: u32,
    pub opacity: f32,
    pub quad: [f32; 4],
    pub region: [f32; 4],
    pub inverse: [f32; 6],
    pub filters: [f32; 4],
    pub mask: Option<([f32; 4], Stops)>,
}

impl Layer {
    pub(crate) fn push(&self, out: &mut Vec<u32>) {
        let n = self
            .mask
            .as_ref()
            .map_or(0, |m| u32::try_from(m.1.len().min(255)).unwrap_or(0));
        let flags = self.mask.as_ref().map_or(0, |_| F_RADII) | (n << 16);
        let [c0, c1] = color([0.0, 0.0, 0.0, self.opacity.clamp(0.0, 1.0)]);
        out.extend([KIND_LAYER | flags, self.slot, c0, c1]);
        out.extend(f(self.quad));
        out.extend(f(self.region));
        out.extend(self.inverse.map(f32::to_bits));
        out.extend(f(self.filters));
        let (line, stops) = self.mask.clone().unwrap_or(([0.0; 4], Vec::new()));
        out.extend(f(line));
        push_stops(out, stops.get(..255).unwrap_or(&stops));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_round_to_nearest() {
        assert_eq!(half(0.0), 0);
        assert_eq!(half(1.0), 0x3c00);
        assert_eq!(half(-2.0), 0xc000);
        assert_eq!(half(0.5), 0x3800);
        assert_eq!(half(65504.0), 0x7bff);
        assert_eq!(half(1e6), 0x7c00);
        // The smallest subnormal half, and a value just past half of it.
        assert_eq!(half(5.960_464_5e-8), 1);
        assert_eq!(half(3.0e-8), 1);
        // Every byte of a colour comes back within half a step.
        for v in 0..=255_u8 {
            let x = f32::from(v) / 255.0;
            let h = half(x);
            let back = {
                let e = (h >> 10) & 0x1f;
                let m = f32::from(h & 0x3ff);
                if e == 0 {
                    m * 2f32.powi(-24)
                } else {
                    (1.0 + m / 1024.0) * 2f32.powi(i32::from(e) - 15)
                }
            };
            assert!((back - x).abs() * 255.0 < 0.5, "{v}: {x} -> {back}");
        }
    }

    #[test]
    fn a_glyph_is_eight_words_and_a_rounded_rect_twelve() {
        let g = Glyph {
            kind: KIND_MONO,
            page: 1,
            rows: [2, 0, 0, 0],
            slot: 7,
            color: [0.0, 0.0, 0.0, 1.0],
            at: (1.0, 2.0),
            size: (5, 9),
            texel: (100, 200),
        };
        assert_eq!(g.words().len(), 8);
        let mut out = Vec::new();
        Solid {
            slot: 0,
            color: [1.0; 4],
            rect: [0.0, 0.0, 10.0, 10.0],
            radii: Some([2.0; 4]),
            inner: None,
        }
        .push(&mut out);
        assert_eq!(out.len(), 12);
    }
}
