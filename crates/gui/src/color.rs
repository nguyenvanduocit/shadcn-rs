//! Colours: straight-alpha sRGB, built from hex or from OKLCH, the space
//! shadcn's tokens are written in.

use std::fmt;

use crate::convert::unit_to_u8;

/// An sRGB colour with straight (not premultiplied) alpha, every channel in `0.0..=1.0`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    pub const WHITE: Self = Self {
        r: 1.0,
        g: 1.0,
        b: 1.0,
        a: 1.0,
    };

    /// An opaque colour from `0xRRGGBB`.
    #[must_use]
    pub fn hex(rgb: u32) -> Self {
        let [_, r, g, b] = rgb.to_be_bytes();
        Self {
            r: f32::from(r) / 255.0,
            g: f32::from(g) / 255.0,
            b: f32::from(b) / 255.0,
            a: 1.0,
        }
    }

    /// An opaque colour from CSS `oklch(l c h)`: lightness `0..1`, chroma, hue in degrees.
    #[must_use]
    pub fn oklch(l: f32, c: f32, h: f32) -> Self {
        let (sin, cos) = h.to_radians().sin_cos();
        Self::oklab([l, c * cos, c * sin], 1.0)
    }

    /// A colour from OKLab `[l, a, b]` and straight `alpha`.
    #[must_use]
    pub fn oklab([l, a, b]: [f32; 3], alpha: f32) -> Self {
        let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
        let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
        let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
        let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
        let r = 4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3;
        let g = -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3;
        let b = -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3;
        Self {
            r: gamma(r),
            g: gamma(g),
            b: gamma(b),
            a: alpha,
        }
    }

    /// This colour in OKLab, `[l, a, b]` (alpha aside).
    #[must_use]
    pub fn to_oklab(self) -> [f32; 3] {
        let (r, g, b) = (linear(self.r), linear(self.g), linear(self.b));
        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
        let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
        [
            0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
        ]
    }

    /// The colour `t` of the way from `self` to `other`, interpolated in
    /// OKLab with premultiplied alpha, as CSS mixes and Tailwind's gradients do.
    #[must_use]
    pub fn mix_oklab(self, other: Self, t: f32) -> Self {
        let alpha = self.a + (other.a - self.a) * t;
        if alpha <= 0.0 {
            return Self::TRANSPARENT;
        }
        let (from, to) = (self.to_oklab(), other.to_oklab());
        let channel = |i: usize| {
            let (x, y) = (from.get(i).copied().unwrap_or(0.0), to.get(i).copied().unwrap_or(0.0));
            (x * self.a + (y * other.a - x * self.a) * t) / alpha
        };
        Self::oklab([channel(0), channel(1), channel(2)], alpha)
    }

    /// This colour with its alpha multiplied by `opacity`: Tailwind's `bg-primary/90` is
    /// `primary.opacity(0.9)`.
    #[must_use]
    pub fn opacity(self, opacity: f32) -> Self {
        Self {
            a: self.a * opacity.clamp(0.0, 1.0),
            ..self
        }
    }

    /// Whether painting this colour changes nothing.
    #[must_use]
    pub fn is_transparent(self) -> bool {
        self.a <= 0.0
    }

    /// `self` drawn over an opaque `below`.
    #[must_use]
    pub fn over(self, below: Self) -> Self {
        let mix = |top: f32, bottom: f32| top * self.a + bottom * (1.0 - self.a);
        Self {
            r: mix(self.r, below.r),
            g: mix(self.g, below.g),
            b: mix(self.b, below.b),
            a: 1.0,
        }
    }

    pub(crate) fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(
            self.r.clamp(0.0, 1.0),
            self.g.clamp(0.0, 1.0),
            self.b.clamp(0.0, 1.0),
            self.a.clamp(0.0, 1.0),
        )
        .unwrap_or(tiny_skia::Color::TRANSPARENT)
    }

    /// The four channels as bytes, straight alpha.
    #[must_use]
    pub fn to_rgba8(self) -> [u8; 4] {
        [
            unit_to_u8(self.r),
            unit_to_u8(self.g),
            unit_to_u8(self.b),
            unit_to_u8(self.a),
        ]
    }

    /// Relative luminance (WCAG 2.2), for contrast checks.
    #[must_use]
    pub fn luminance(self) -> f32 {
        0.2126 * linear(self.r) + 0.7152 * linear(self.g) + 0.0722 * linear(self.b)
    }

    /// WCAG contrast ratio between two opaque colours, `1.0..=21.0`.
    #[must_use]
    pub fn contrast(self, other: Self) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

/// Linear light to the sRGB transfer curve, clamped into gamut.
fn gamma(linear: f32) -> f32 {
    let c = linear.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The sRGB transfer curve undone: a channel in linear light.
fn linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b, a] = self.to_rgba8();
        match a {
            0 => f.write_str("transparent"),
            255 => write!(f, "#{r:02x}{g:02x}{b:02x}"),
            _ => write!(f, "#{r:02x}{g:02x}{b:02x}{a:02x}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklch_matches_css_for_shadcn_tokens() {
        // Values a browser computes for these tokens.
        assert_eq!(Color::oklch(1.0, 0.0, 0.0).to_string(), "#ffffff");
        assert_eq!(Color::oklch(0.145, 0.0, 0.0).to_string(), "#0a0a0a");
        assert_eq!(Color::oklch(0.205, 0.0, 0.0).to_string(), "#171717");
        assert_eq!(Color::oklch(0.922, 0.0, 0.0).to_string(), "#e5e5e5");
        assert_eq!(Color::oklch(0.577, 0.245, 27.325).to_string(), "#e7000b");
    }
}
