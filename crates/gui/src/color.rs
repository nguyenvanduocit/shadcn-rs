//! Colours.
//!
//! [`Color`] is what is painted: straight-alpha sRGB, every channel in
//! `0.0..=1.0`. [`Oklch`] is a colour as CSS Color 4 computes it in OKLCH,
//! the space shadcn's tokens are written in: its components, relative colours
//! (`oklch(from c l calc(c * k) h)` is `c.with_chroma(|c| c * k)`) and
//! `color-mix(in oklch, …)` (`a.mix(b, t)`), all as Chrome computes them.
//! An `Oklch` keeps what sRGB cannot hold (chroma outside the sRGB gamut, a
//! hue at zero chroma, a `none` hue), so relative colours and mixes of
//! tokens start from the token itself; it becomes a [`Color`] only to be
//! painted, clipped into sRGB channel by channel as Chrome draws it on an
//! sRGB surface. `crates/gui/tests/chrome_oklch.rs` checks both against
//! Chrome.

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

    /// An opaque colour from CSS `oklch(l c h)`: lightness `0..1`, chroma, hue in degrees,
    /// clipped into sRGB. [`Oklch::new`] keeps it as written, for relative colours and mixes.
    #[must_use]
    pub fn oklch(l: f32, c: f32, h: f32) -> Self {
        Oklch::new(l, c, h).into()
    }

    /// A colour from OKLab `[l, a, b]` and straight `alpha`, clipped into sRGB.
    #[must_use]
    pub fn oklab(lab: [f32; 3], alpha: f32) -> Self {
        let [r, g, b] = oklab_to_srgb(lab);
        Self {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a: alpha,
        }
    }

    /// This colour in OKLCH, as a relative colour reads its origin
    /// (`oklch(from <this> l c h)`) and `color-mix(in oklch, …)` converts it.
    #[must_use]
    pub fn to_oklch(self) -> Oklch {
        Oklch::from(self)
    }

    /// CSS `color-mix(in oklch, self, other <t × 100%>)`: the colour `t` of the way
    /// from `self` to `other`, mixed in OKLCH along the shorter hue arc with
    /// premultiplied alpha. [`Oklch::mix`] mixes colours kept in OKLCH.
    #[must_use]
    pub fn mix_oklch(self, other: Self, t: f32) -> Self {
        self.to_oklch().mix(other.to_oklch(), t).into()
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

/// OKLab to sRGB, unclipped: a channel outside `0.0..=1.0` is a colour
/// outside the sRGB gamut, as CSS's `color(srgb …)` holds it.
fn oklab_to_srgb([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_.powi(3), m_.powi(3), s_.powi(3));
    [
        gamma(4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3),
        gamma(-1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3),
        gamma(-0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3),
    ]
}

/// Linear light to the sRGB transfer curve, extended to every real number
/// as CSS extends it (mirrored below zero).
fn gamma(linear: f32) -> f32 {
    let c = linear.abs();
    let encoded = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    encoded.copysign(linear)
}

/// The sRGB transfer curve undone: a channel in linear light.
fn linear(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Below this chroma a colour's hue is noise of the conversion (sRGB grey is
/// about 1e-7 from the axis in `f32`), and reads as 0, CSS's value for a
/// missing component.
const HUE_NOISE: f32 = 1e-4;

/// Chrome's threshold for an achromatic colour: a colour converted into
/// OKLCH for `color-mix` whose chroma is at most this has a powerless
/// (missing) hue, so the other colour's hue is used. Measured: chroma 0.02
/// mixes as `none`, 0.020 01 keeps its hue (`tests/chrome_oklch`).
const POWERLESS_CHROMA: f32 = 0.02;

/// How `color-mix` interpolates between two hues (CSS Color 4 §12.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HueInterpolation {
    /// The arc of at most 180°, CSS's default.
    #[default]
    Shorter,
    /// The arc of at least 180°.
    Longer,
    /// Hue only grows, wrapping past 360°.
    Increasing,
    /// Hue only shrinks, wrapping past 0°.
    Decreasing,
}

/// A component of a relative colour: a value replaces the origin's
/// (`with_chroma(0.1)`, CSS `oklch(from x l 0.1 h)`), a closure computes
/// from it (`with_chroma(|c| c * 0.5)`, CSS `calc(c * 0.5)`).
pub trait Component {
    /// The new component, from the origin's.
    fn resolve(self, origin: f32) -> f32;
}

impl Component for f32 {
    fn resolve(self, _origin: f32) -> f32 {
        self
    }
}

impl<F: FnOnce(f32) -> f32> Component for F {
    fn resolve(self, origin: f32) -> f32 {
        self(origin)
    }
}

/// A colour in OKLCH with straight alpha, as CSS Color 4 computes
/// `oklch()`, relative colours and `color-mix(in oklch, …)`.
///
/// Lightness is in `0.0..=1.0`, chroma at least 0 (with no upper bound: a
/// colour may lie outside sRGB, kept until painted), hue in degrees
/// `0.0..360.0`, alpha in `0.0..=1.0`. The hue may be missing (CSS `none`):
/// written so ([`Oklch::without_hue`]), or the powerless hue of a nearly
/// achromatic colour converted from sRGB. A mix then takes the other colour's
/// hue; a relative colour reads it as `0`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Oklch {
    l: f32,
    c: f32,
    h: f32,
    hue_missing: bool,
    alpha: f32,
}

impl Oklch {
    /// CSS `oklch(l c h)`, opaque. As CSS parses it, lightness is clamped to
    /// `0.0..=1.0`, a negative chroma is 0 and the hue is taken modulo 360°.
    #[must_use]
    pub fn new(l: f32, c: f32, h: f32) -> Self {
        Self {
            l: lightness(l),
            c: chroma(c),
            h: hue(h),
            hue_missing: false,
            alpha: 1.0,
        }
    }

    /// Lightness, `0.0..=1.0`.
    #[must_use]
    pub fn lightness(self) -> f32 {
        self.l
    }

    /// Chroma, at least 0.
    #[must_use]
    pub fn chroma(self) -> f32 {
        self.c
    }

    /// Hue in degrees, `0.0..360.0`, as a relative colour reads it: `0` when missing.
    #[must_use]
    pub fn hue(self) -> f32 {
        self.h
    }

    /// Whether the hue is missing (CSS `none`), so a mix takes the other colour's.
    #[must_use]
    pub fn hue_missing(self) -> bool {
        self.hue_missing
    }

    /// Straight alpha, `0.0..=1.0`.
    #[must_use]
    pub fn alpha(self) -> f32 {
        self.alpha
    }

    /// The relative colour `oklch(from self <l> c h / alpha)`.
    #[must_use]
    pub fn with_lightness(self, l: impl Component) -> Self {
        Self {
            l: lightness(l.resolve(self.l)),
            ..self.relative()
        }
    }

    /// The relative colour `oklch(from self l <c> h / alpha)`.
    #[must_use]
    pub fn with_chroma(self, c: impl Component) -> Self {
        Self {
            c: chroma(c.resolve(self.c)),
            ..self.relative()
        }
    }

    /// The relative colour `oklch(from self l c <h> / alpha)`.
    #[must_use]
    pub fn with_hue(self, h: impl Component) -> Self {
        Self {
            h: hue(h.resolve(self.h)),
            ..self.relative()
        }
    }

    /// The relative colour `oklch(from self l c h / <alpha>)`.
    #[must_use]
    pub fn with_alpha(self, alpha: impl Component) -> Self {
        Self {
            alpha: unit(alpha.resolve(self.alpha)),
            ..self.relative()
        }
    }

    /// This colour with a missing hue: CSS `oklch(l c none)`.
    #[must_use]
    pub fn without_hue(self) -> Self {
        Self {
            h: 0.0,
            hue_missing: true,
            ..self
        }
    }

    /// CSS `color-mix(in oklch, self, other <t × 100%>)`: `t` of the way from
    /// `self` to `other` (clamped to `0.0..=1.0`), along the shorter hue arc.
    #[must_use]
    pub fn mix(self, other: Self, t: f32) -> Self {
        self.mix_hue(other, t, HueInterpolation::Shorter)
    }

    /// CSS `color-mix(in oklch <hue> hue, self, other <t × 100%>)`.
    ///
    /// Lightness and chroma are mixed premultiplied by alpha, the hue is not;
    /// a missing hue takes the other colour's, and stays missing if both are
    /// (CSS Color 4 §12.3–12.4).
    #[must_use]
    pub fn mix_hue(self, other: Self, t: f32, hue_interpolation: HueInterpolation) -> Self {
        let t = unit(t);
        let s = 1.0 - t;
        let alpha = self.alpha * s + other.alpha * t;
        let premultiplied = |x: f32, y: f32| {
            let sum = x * self.alpha * s + y * other.alpha * t;
            if alpha == 0.0 { sum } else { sum / alpha }
        };
        let (h, hue_missing) = match (self.hue_missing, other.hue_missing) {
            (true, true) => (0.0, true),
            (false, true) => (mix_hue(self.h, self.h, t, hue_interpolation), false),
            (true, false) => (mix_hue(other.h, other.h, t, hue_interpolation), false),
            (false, false) => (mix_hue(self.h, other.h, t, hue_interpolation), false),
        };
        Self {
            l: lightness(premultiplied(self.l, other.l)),
            c: chroma(premultiplied(self.c, other.c)),
            h,
            hue_missing,
            alpha,
        }
    }

    /// In OKLab, `[l, a, b]`. A missing hue converts as 0°, as CSS converts
    /// a missing component (and Chrome paints `oklch(0.5 0.1 none)` at hue 0).
    #[must_use]
    pub fn to_oklab(self) -> [f32; 3] {
        let (sin, cos) = self.h.to_radians().sin_cos();
        [self.l, self.c * cos, self.c * sin]
    }

    /// In sRGB, `[r, g, b, alpha]`, unclipped, as CSS's `color(srgb …)` holds
    /// it: a channel outside `0.0..=1.0` means the colour is outside sRGB.
    #[must_use]
    pub fn to_srgb(self) -> [f32; 4] {
        let [r, g, b] = oklab_to_srgb(self.to_oklab());
        [r, g, b, self.alpha]
    }

    /// Whether sRGB holds this colour (within `f32` rounding), so painting it clips nothing.
    #[must_use]
    pub fn in_srgb_gamut(self) -> bool {
        let [r, g, b, _] = self.to_srgb();
        [r, g, b].iter().all(|c| (-1e-5..=1.0 + 1e-5).contains(c))
    }

    /// The result of a relative colour: every component read, so none is missing.
    fn relative(self) -> Self {
        Self {
            hue_missing: false,
            ..self
        }
    }
}

/// Painted on an sRGB surface, as Chrome paints it: each channel clipped
/// into `0.0..=1.0` (no gamut mapping).
impl From<Oklch> for Color {
    fn from(color: Oklch) -> Self {
        let [r, g, b, a] = color.to_srgb();
        Self {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a,
        }
    }
}

/// An sRGB colour in OKLCH. Its hue is the one a relative colour reads
/// (`0` within 1e-4 chroma of grey); it is missing for `color-mix` when the
/// chroma is at most 0.02, as Chrome converts.
impl From<Color> for Oklch {
    fn from(color: Color) -> Self {
        let [l, a, b] = color.to_oklab();
        let c = a.hypot(b);
        let h = if c < HUE_NOISE {
            0.0
        } else {
            hue(b.atan2(a).to_degrees())
        };
        Self {
            l: lightness(l),
            c,
            h,
            hue_missing: c <= POWERLESS_CHROMA,
            alpha: unit(color.a),
        }
    }
}

/// CSS's serialization: `oklch(l c h)`, `oklch(l c none / a)`.
impl fmt::Display for Oklch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "oklch({} {} ", self.l, self.c)?;
        if self.hue_missing {
            f.write_str("none")?;
        } else {
            write!(f, "{}", self.h)?;
        }
        if self.alpha < 1.0 {
            write!(f, " / {}", self.alpha)?;
        }
        f.write_str(")")
    }
}

/// `h` and `to` interpolated `t` of the way, after CSS Color 4's hue fix-up.
fn mix_hue(from: f32, to: f32, t: f32, method: HueInterpolation) -> f32 {
    let (mut from, mut to) = (hue(from), hue(to));
    let delta = to - from;
    match method {
        HueInterpolation::Shorter => {
            if delta > 180.0 {
                from += 360.0;
            } else if delta < -180.0 {
                to += 360.0;
            }
        }
        HueInterpolation::Longer => {
            if 0.0 < delta && delta < 180.0 {
                from += 360.0;
            } else if -180.0 < delta && delta <= 0.0 {
                to += 360.0;
            }
        }
        HueInterpolation::Increasing => {
            if to < from {
                to += 360.0;
            }
        }
        HueInterpolation::Decreasing => {
            if from < to {
                from += 360.0;
            }
        }
    }
    hue(from * (1.0 - t) + to * t)
}

/// A value in `0.0..=1.0`; NaN is 0.
fn unit(v: f32) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) }
}

/// OKLCH lightness as CSS clamps it.
fn lightness(l: f32) -> f32 {
    unit(l)
}

/// OKLCH chroma as CSS clamps it: never negative; NaN is 0.
fn chroma(c: f32) -> f32 {
    if c.is_nan() { 0.0 } else { c.max(0.0) }
}

/// A hue in `0.0..360.0`; NaN and infinities are 0.
fn hue(h: f32) -> f32 {
    if !h.is_finite() {
        return 0.0;
    }
    let h = h.rem_euclid(360.0);
    // `rem_euclid` of a tiny negative rounds up to exactly 360.
    if h >= 360.0 { 0.0 } else { h }
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

    #[test]
    fn a_component_is_a_value_or_computed_from_the_origin() {
        let primary = Oklch::new(0.577, 0.245, 27.325);
        assert_eq!(primary.with_chroma(0.1).chroma(), 0.1);
        assert!((primary.with_chroma(|c| c * 0.5).chroma() - 0.1225).abs() < 1e-6);
        assert!((primary.with_hue(|h| h - 30.0).hue() - 357.325).abs() < 1e-4);
        assert_eq!(primary.with_lightness(|l| l + 1.0).lightness(), 1.0);
        assert_eq!(primary.with_alpha(2.0).alpha(), 1.0);
        assert_eq!(primary.with_chroma(-1.0).chroma(), 0.0);
    }

    #[test]
    fn a_missing_hue_is_kept_by_mixes_and_read_as_zero_by_relative_colours() {
        let grey = Oklch::new(0.5, 0.0, 200.0).without_hue();
        assert!(grey.hue_missing());
        assert!(grey.mix(grey, 0.5).hue_missing());
        assert_eq!(grey.mix(Oklch::new(0.7, 0.2, 120.0), 0.5).hue(), 120.0);
        let relative = grey.with_lightness(|l| l);
        assert!(!relative.hue_missing());
        assert_eq!(relative.hue(), 0.0);
    }

    #[test]
    fn srgb_round_trips_through_oklch() {
        for hex in [0x00_0000, 0xff_ffff, 0xe7_000b, 0x3b_82f6, 0x10_b981, 0x80_8080] {
            let c = Color::hex(hex);
            assert_eq!(Color::from(c.to_oklch()).to_string(), c.to_string());
        }
    }

    #[test]
    fn a_colour_outside_srgb_is_kept_until_painted() {
        let green = Oklch::new(0.7, 0.4, 150.0);
        assert!(!green.in_srgb_gamut());
        assert!(Oklch::new(0.6, 0.1, 150.0).in_srgb_gamut());
        let [r, _, b, _] = green.to_srgb();
        assert!(r < 0.0 && b < 0.0);
        // A quarter of the chroma is a quarter of 0.4, not of the clipped colour's.
        let muted = green.with_chroma(|c| c * 0.25);
        assert_eq!(muted.chroma(), 0.1);
        assert!(muted.in_srgb_gamut());
        let painted = Color::from(green);
        assert_eq!(painted.to_rgba8(), [0, 214, 0, 255]);
    }

    #[test]
    fn oklch_serializes_as_css() {
        assert_eq!(Oklch::new(0.5, 0.1, 30.0).to_string(), "oklch(0.5 0.1 30)");
        assert_eq!(
            Oklch::new(0.5, 0.0, 0.0).without_hue().with_alpha(0.5).to_string(),
            "oklch(0.5 0 0 / 0.5)"
        );
        assert_eq!(
            Oklch::new(0.5, 0.0, 0.0).with_alpha(0.5).without_hue().to_string(),
            "oklch(0.5 0 none / 0.5)"
        );
    }
}
