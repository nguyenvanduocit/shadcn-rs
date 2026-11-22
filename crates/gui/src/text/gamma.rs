//! Skia's mask gamma: the per-luminance coverage correction Chrome applies to glyph masks.
//!
//! A glyph rasterizer produces linear coverage, but the blend that draws the glyph is done in
//! gamma-encoded (sRGB) space, so dark text on a light background looks too thin and light text
//! on a dark background looks too bold. Skia corrects this before the blend: it bends the coverage
//! through a lookup table chosen by the luminance of the text colour, guessing that the
//! background is the opposite luminance. This module is a port of that table
//! (`SkTMaskGamma<3, 3, 3>`, `SkTMaskGamma_build_correcting_lut` in `src/core/SkMaskGamma.cpp`) and
//! of how Skia picks a row of it (`SkScalerContextRec::setLuminanceColor`,
//! `SkScalerContext::PreprocessRec`, `SkTMaskGamma::preBlend`).
//!
//! The table has 8 rows of 256 bytes (3 luminance bits, as Skia's `SkMaskGamma` typedef), and it
//! is uploaded as one R8 texture 256 wide and 8 high. The text colour is not part of the glyph
//! cache key: the shader reads the coverage from the atlas, picks the row from the colour at draw
//! time, and looks the coverage up in it. An A8 (grey) glyph uses one row, chosen from the
//! colour's luminance ([`MaskGamma::luminance_row`]); an LCD glyph uses one row per channel
//! ([`MaskGamma::channel_rows`]).
//!
//! Differences from the API first sketched for this module, each because Skia's model demands it:
//!
//! - There is no paint gamma. Current Skia converts the paint colour to the device colour space
//!   before text is drawn, so the paint gamma always equals the device gamma, and
//!   `SkTMaskGamma_build_correcting_lut` uses one transfer function for both source and
//!   destination. `GammaParams` therefore has one `gamma` field. Older Skia's `build_power_table`
//!   and separate paint gamma no longer exist.
//! - `GammaParams` has a `luminance_scale`. On macOS, Skia emulates CoreGraphics' font smoothing by
//!   scaling the luminance colour by 1/2 or 3/4 before the row is picked
//!   (`SkTypeface_Mac::onFilterRec`). That changes which row a colour picks, so
//!   [`MaskGamma::luminance_row`] and [`MaskGamma::channel_rows`] both take the parameters.
//! - Contrast and gamma are quantized as Skia stores them in `SkScalerContextRec`: contrast to
//!   1/255 steps (rounded), gamma to 1/64 steps (truncated). Chromium's Linux gamma 1.2 is
//!   therefore really 1.1875, and the tables match Skia's byte for byte only with this step.
//! - Skia builds no table when contrast is 0 and gamma is 1 (`linear_gamma()`) and leaves the
//!   coverage alone. [`MaskGamma::new`] returns identity rows in that case, which is the same.
//!
//! One deliberate difference in arithmetic: Skia calls `powf`, whose last bit differs between C
//! libraries, and one unit in the last place does change bytes of the table. This port computes
//! the power in `f64` and rounds it to `f32`, which is the correctly rounded `powf`. It equals
//! Skia's output with macOS' libm byte for byte (the tests check it), and gives the same table on
//! every machine.

/// A transfer function as Skia's `SkColorSpaceLuminance` models it.
///
/// `Power(1.0)` is linear. Skia stores the exponent in 1/64 steps (truncated) and reads a stored
/// 0 as sRGB, so `Power` of anything below 1/64 behaves as `Srgb`, as it does in Skia.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gamma {
    /// The sRGB transfer function (Skia's gamma 0).
    Srgb,
    /// A pure power law with this exponent, in Skia's range [0, 4).
    Power(f32),
}

/// How the luminance colour is scaled before a row is picked.
///
/// Skia does this only on macOS, to emulate the dilation CoreGraphics applies to smoothed text:
/// smoothing without subpixel coverage goes from a gamma of 2.0 for black text to 1.0 for white,
/// which Skia approximates by halving the colour; subpixel smoothing goes to about 1.4, which it
/// approximates by taking three quarters of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuminanceScale {
    /// The colour is used as it is (every platform but macOS, and macOS without smoothing).
    Full,
    /// The colour is halved (macOS smoothing without subpixel coverage, CoreText's "some").
    Half,
    /// The colour is scaled by 3/4 (macOS subpixel smoothing).
    ThreeQuarters,
}

/// The inputs to Skia's mask gamma, as `SkSurfaceProps` and the platform's `onFilterRec` set them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GammaParams {
    /// Artificial contrast in [0, 1]; it thickens dark text and tapers off to nothing for white.
    pub contrast: f32,
    /// The transfer function of the device, used for both the text colour and the background.
    pub gamma: Gamma,
    /// How the luminance colour is scaled before a row is picked.
    pub luminance_scale: LuminanceScale,
}

impl GammaParams {
    /// Chromium on Linux: its default font render params give a power gamma of 1.2 (stored by
    /// Skia as 1.1875) and a contrast of 0.2.
    pub fn linux() -> Self {
        Self {
            contrast: 0.2,
            gamma: Gamma::Power(1.2),
            luminance_scale: LuminanceScale::Full,
        }
    }

    /// Chromium on Windows: sRGB with full contrast.
    pub fn windows() -> Self {
        Self {
            contrast: 1.0,
            gamma: Gamma::Srgb,
            luminance_scale: LuminanceScale::Full,
        }
    }

    /// Chromium on macOS: sRGB, no contrast (Skia zeroes it because CoreGraphics dilates smoothed
    /// text instead), and the colour halved as Skia does for smoothing without subpixel coverage,
    /// which is what macOS has used since 10.14 with the default "font smoothing" setting.
    pub fn macos() -> Self {
        Self {
            contrast: 0.0,
            gamma: Gamma::Srgb,
            luminance_scale: LuminanceScale::Half,
        }
    }
}

/// The number of rows: 2 to the power of Skia's 3 luminance bits.
pub const ROWS: usize = 8;

/// The width of a row: one entry per coverage value.
pub const ROW_WIDTH: usize = 256;

/// The colour Skia assumes when the paint is not a solid colour (a gradient or an image): mid
/// grey, `SkColor4f { 0.5, 0.5, 0.5, 1.0 }.toSkColor()`.
pub const NON_SOLID_PAINT_RGB: [u8; 3] = [128, 128, 128];

/// Skia's `sk_t_scale255<3>`: a 3-bit value spread over 0..=255 by repeating its bits
/// (`b << 5 | b << 2 | b >> 1`). Row `i` is built for this source luminance.
const SCALE_3_BITS: [u8; ROWS] = [0, 36, 73, 109, 146, 182, 219, 255];

/// Skia's 8 coverage correction tables, one per luminance of the text colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskGamma {
    rows: [[u8; ROW_WIDTH]; ROWS],
}

impl MaskGamma {
    /// Builds the tables as `SkScalerContextRec::CachedMaskGamma` does for these parameters.
    pub fn new(params: &GammaParams) -> Self {
        let contrast = internal_contrast(params.contrast);
        let gamma = internal_gamma(params.gamma);
        let mut rows = [[0_u8; ROW_WIDTH]; ROWS];
        if contrast == 0 && gamma == LINEAR_GAMMA {
            // Skia builds no table here and leaves coverage alone.
            for row in &mut rows {
                for (value, out) in (0_u8..=u8::MAX).zip(row.iter_mut()) {
                    *out = value;
                }
            }
            return Self { rows };
        }
        let contrast = f32::from(contrast) / 255.0;
        let transfer = Transfer::from_internal(gamma);
        for (row, src) in rows.iter_mut().zip(SCALE_3_BITS) {
            build_correcting_lut(row, src, contrast, transfer);
        }
        Self { rows }
    }

    /// One row of 256 entries, indexed by linear coverage, or `None` past the last row.
    pub fn row(&self, row: usize) -> Option<&[u8; ROW_WIDTH]> {
        self.rows.get(row)
    }

    /// The 2048 bytes laid out row-major, for an R8 texture 256 wide and 8 high.
    pub fn texture_bytes(&self) -> Vec<u8> {
        self.rows.as_flattened().to_vec()
    }

    /// The row an A8 (grey) glyph drawn in straight-alpha sRGB colour `rgb` uses, as Skia picks it.
    ///
    /// Skia snaps the colour to 3 bits per channel, scales it on macOS, snaps it again, takes its
    /// luminance with fixed-point BT.709 weights (`SkComputeLuminance`, applied to the encoded
    /// values as Skia does), and uses the top 3 bits of that grey. Alpha plays no part: the
    /// blitter applies it after the correction.
    pub fn luminance_row(params: &GammaParams, rgb: [u8; 3]) -> usize {
        let [r, g, b] = luminance_color(params, rgb);
        let weighted = u32::from(r)
            .saturating_mul(54)
            .saturating_add(u32::from(g).saturating_mul(183))
            .saturating_add(u32::from(b).saturating_mul(19));
        let lum = u8::try_from(weighted.wrapping_shr(8)).unwrap_or(u8::MAX);
        row_of(lum)
    }

    /// The row each channel of an LCD glyph uses: the top 3 bits of that channel of the
    /// (on macOS, scaled) colour, as Skia's `preBlend` picks `fR`, `fG` and `fB`.
    pub fn channel_rows(params: &GammaParams, rgb: [u8; 3]) -> [usize; 3] {
        luminance_color(params, rgb).map(row_of)
    }
}

/// Skia's stored gamma for linear (1.0 in 1/64 steps).
const LINEAR_GAMMA: u8 = 64;

/// Skia's `InternalContrastFromExternal`: contrast in 1/255 steps, rounded.
fn internal_contrast(contrast: f32) -> u8 {
    f32_to_u8(contrast.clamp(0.0, 1.0) * 255.0 + 0.5)
}

/// Skia's `InternalGammaFromExternal`: gamma in 1/64 steps, truncated; sRGB is stored as 0.
fn internal_gamma(gamma: Gamma) -> u8 {
    match gamma {
        Gamma::Srgb => 0,
        Gamma::Power(exponent) => f32_to_u8(exponent * 64.0),
    }
}

/// The colour Skia keeps in `fLumBits`: snapped to 3 bits per channel, scaled for macOS, and
/// snapped again (`setLuminanceColor` in `MakeRecAndEffects`, then in `onFilterRec`).
fn luminance_color(params: &GammaParams, rgb: [u8; 3]) -> [u8; 3] {
    rgb.map(|channel| canonical(scale(params.luminance_scale, canonical(channel))))
}

/// Skia's `CanonicalColor` for one channel: its top 3 bits spread back over 0..=255.
fn canonical(channel: u8) -> u8 {
    SCALE_3_BITS.get(row_of(channel)).copied().unwrap_or(u8::MAX)
}

/// The macOS scaling of one channel, in Skia's integer arithmetic (`c * 1/2`, `c * 3/4`).
fn scale(scale: LuminanceScale, channel: u8) -> u8 {
    match scale {
        LuminanceScale::Full => channel,
        LuminanceScale::Half => channel.wrapping_shr(1),
        LuminanceScale::ThreeQuarters => {
            u8::try_from(u16::from(channel).saturating_mul(3).wrapping_shr(2)).unwrap_or(u8::MAX)
        }
    }
}

/// The row a channel value selects: its top 3 bits.
fn row_of(value: u8) -> usize {
    usize::from(value.wrapping_shr(5))
}

/// Skia's three `SkColorSpaceLuminance` implementations, as `Fetch` picks them from the stored
/// gamma: 0 is sRGB, exactly 1 is linear, anything else a power law.
#[derive(Clone, Copy, Debug)]
enum Transfer {
    Srgb,
    Linear,
    Power(f32),
}

impl Transfer {
    fn from_internal(gamma: u8) -> Self {
        match gamma {
            0 => Self::Srgb,
            LINEAR_GAMMA => Self::Linear,
            _ => Self::Power(f32::from(gamma) / 64.0),
        }
    }

    /// Encoded luminance to linear luma (Skia's `toLuma`).
    fn decode(self, luminance: f32) -> f32 {
        match self {
            Self::Linear => luminance,
            Self::Power(gamma) => pow(luminance, gamma),
            Self::Srgb => {
                if luminance <= 0.04045 {
                    luminance / 12.92
                } else {
                    pow((luminance + 0.055) / 1.055, 2.4)
                }
            }
        }
    }

    /// Linear luma to encoded luminance (Skia's `fromLuma`).
    fn encode(self, luma: f32) -> f32 {
        match self {
            Self::Linear => luma,
            Self::Power(gamma) => pow(luma, 1.0 / gamma),
            Self::Srgb => {
                if luma <= 0.003_130_8 {
                    luma * 12.92
                } else {
                    1.055 * pow(luma, 1.0 / 2.4) - 0.055
                }
            }
        }
    }
}

/// Skia's `apply_contrast`: pushes coverage up, most in the middle, none at 0 and 1.
fn apply_contrast(srca: f32, contrast: f32) -> f32 {
    srca + ((1.0 - srca) * contrast * srca)
}

/// Skia's `SkTMaskGamma_build_correcting_lut`, step for step in `f32`.
///
/// For text of encoded luminance `src_i` over a background guessed as its opposite, each entry is
/// the coverage that, blended linearly in encoded space, gives the colour a blend in linear space
/// would have given.
fn build_correcting_lut(table: &mut [u8; ROW_WIDTH], src_i: u8, contrast: f32, transfer: Transfer) {
    let src = f32::from(src_i) / 255.0;
    let lin_src = transfer.decode(src);
    // The perceptual opposite gives smaller jumps than a linear one when a slight change of a
    // desaturated colour moves a channel to the neighbouring row (Chromium issue 141425).
    let dst = 1.0 - src;
    let lin_dst = transfer.decode(dst);
    // Contrast tapers off to nothing as the text becomes white.
    let adjusted_contrast = contrast * lin_dst;
    // Skia counts in a float rather than dividing an integer, so 255 / 255 is exactly 1.
    let mut ii = 0.0_f32;
    if (src - dst).abs() < 1.0 / 256.0 {
        // Text and background are too close for the correction to be stable: contrast only.
        for out in table.iter_mut() {
            let srca = apply_contrast(ii / 255.0, adjusted_contrast);
            *out = round_to_u8(255.0 * srca);
            ii += 1.0;
        }
    } else {
        for out in table.iter_mut() {
            let srca = apply_contrast(ii / 255.0, adjusted_contrast);
            let dsta = 1.0 - srca;
            let lin_out = lin_src * srca + dsta * lin_dst;
            let encoded = transfer.encode(lin_out);
            // Undo what the encoded-space blend will do.
            let result = (encoded - dst) / (src - dst);
            *out = round_to_u8(255.0 * result);
            ii += 1.0;
        }
    }
}

/// `powf`, correctly rounded: the power is taken in `f64` and rounded once to `f32`, so the
/// tables do not depend on the C library's `powf`.
fn pow(base: f32, exponent: f32) -> f32 {
    f64_to_f32(f64::from(base).powf(f64::from(exponent)))
}

/// Skia's `SkToU8(sk_float_round2int(x))`: round half up through `double`, truncate to `int`,
/// keep the low byte.
fn round_to_u8(x: f32) -> u8 {
    let [low, ..] = f64_to_i32((f64::from(x) + 0.5).floor()).to_le_bytes();
    low
}

#[expect(
    clippy::as_conversions,
    reason = "f64 to f32 rounds to nearest, as C's (float) cast does"
)]
fn f64_to_f32(x: f64) -> f32 {
    x as f32
}

#[expect(
    clippy::as_conversions,
    reason = "the value is an integer; `as` saturates as Skia's saturate2int"
)]
fn f64_to_i32(x: f64) -> i32 {
    x as i32
}

#[expect(
    clippy::as_conversions,
    reason = "truncates toward zero and saturates, as Skia's static_cast<uint8_t> in range"
)]
fn f32_to_u8(x: f32) -> u8 {
    x as u8
}

/// Expected values come from Skia's own code, compiled as a standalone C++ program: the functions
/// `SkTMaskGamma_build_correcting_lut`, `apply_contrast`, the three `SkColorSpaceLuminance`
/// classes, `sk_t_scale255`, `CanonicalColor`, `SkComputeLuminance`, the `Internal*`/`External*`
/// quantizers of `SkScalerContextRec` and the macOS colour scaling of `onFilterRec` were copied
/// verbatim from Skia (`src/core/SkMaskGamma.{h,cpp}`, `src/core/SkScalerContext.{h,cpp}`,
/// `src/core/SkColorData.h`, `src/ports/SkTypeface_mac_ct.cpp`), with `SkScalarPow` as
/// `std::pow(float, float)` and `sk_float_round2int(x)` as `(int)(float)floor((double)x + 0.5)`.
/// It was built with `clang++ -std=c++17 -O1 -ffp-contract=off` on macOS (arm64, Apple libm; the
/// output is the same with `-O2 -ffp-contract=on`). For each parameter set it ran
///
/// ```cpp
/// uint8_t c = InternalContrastFromExternal(contrast), g = InternalGammaFromExternal(gamma);
/// float ec = ExternalContrastFromInternal(c), eg = ExternalGammaFromInternal(g);
/// const SkColorSpaceLuminance& conv = SkColorSpaceLuminance::Fetch(eg);
/// for (U8CPU i = 0; i < 8; ++i)
///     SkTMaskGamma_build_correcting_lut(&tables[i * 256], sk_t_scale255<3>(i), ec, conv, eg);
/// // then FNV-1a 64 over the 2048 bytes and every row in hex
/// ```
///
/// and for each colour and macOS scale mode
///
/// ```cpp
/// SkColor c = CanonicalColor(paint);                       // MakeRecAndEffects
/// c = SkColorSetRGB(R(c) * 1/2, ...) or (R(c) * 3/4, ...)  // onFilterRec on macOS
/// c = CanonicalColor(c);
/// U8CPU l = SkComputeLuminance(R(c), G(c), B(c));          // PreprocessRec, A8 masks
/// a8_row = G(CanonicalColor(SkColorSetRGB(l, l, l))) >> 5; // preBlend's fG
/// lcd_rows = { R(c) >> 5, G(c) >> 5, B(c) >> 5 };          // preBlend's fR, fG, fB
/// ```
#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    /// Expected tables from the C++ reference: (name, params, FNV-1a 64 of all 2048 bytes, rows 0, 3 and 7 in hex).
    fn expected() -> Vec<(&'static str, GammaParams, u64, [&'static str; 3])> {
        vec![
            (
                "linux",
                GammaParams::linux(),
                0x4b0396f48bb8d8e9_u64,
                [
                    concat!(
                        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                        "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                        "40414242434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e",
                        "5f60616262636465666768696a6b6c6d6e6f70717273747576777879797a7b7c",
                        "7d7e7f808182838485868788898a8b8c8d8e8e8f909192939495969798999a9b",
                        "9c9d9e9fa0a1a2a3a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9ba",
                        "bbbbbcbdbebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9",
                        "dadbdcdddedfe0e2e3e4e5e6e7e8e9eaebeceeeff0f1f2f3f5f6f7f9fafbfdff",
                    ),
                    concat!(
                        "0001020304050608090a0b0c0d0e0f1011121314151618191a1b1c1d1e1f2021",
                        "22232425262728292b2c2d2e2f303132333435363738393a3b3c3d3e3f404243",
                        "4445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f60616264",
                        "65666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384",
                        "85868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4",
                        "a5a6a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3",
                        "c4c5c6c7c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdbdcdddedfe0e1",
                        "e2e3e4e5e6e7e8e9eaebebecedeeeff0f1f2f3f4f5f6f7f8f9f9fafbfcfdfeff",
                    ),
                    concat!(
                        "0002040608090b0c0e0f111213151617191a1b1d1e1f20222324252628292a2b",
                        "2c2e2f3031323334363738393a3b3c3d3e404142434445464748494a4b4c4e4f",
                        "505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f",
                        "707172737475767778797a7b7c7d7e7f80808182838485868788898a8b8c8d8e",
                        "8f90919292939495969798999a9b9c9d9e9f9fa0a1a2a3a4a5a6a7a8a9a9aaab",
                        "acadaeafb0b1b2b3b3b4b5b6b7b8b9babbbcbcbdbebfc0c1c2c3c4c4c5c6c7c8",
                        "c9cacbcbcccdcecfd0d1d2d2d3d4d5d6d7d8d9d9dadbdcdddedfdfe0e1e2e3e4",
                        "e5e5e6e7e8e9eaebebecedeeeff0f1f1f2f3f4f5f6f7f7f8f9fafbfcfcfdfeff",
                    ),
                ],
            ),
            (
                "windows",
                GammaParams::windows(),
                0x126da49e80e8c6fd_u64,
                [
                    concat!(
                        "00010203040405060708090a0b0b0c0d0e0f10111213131415161718191a1b1c",
                        "1c1d1e1f20212223242525262728292a2b2c2d2e2f2f30313233343536373839",
                        "3a3a3b3c3d3e3f40414243444546474848494a4b4c4d4e4f5051525354555657",
                        "5858595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f70717273747576",
                        "7778797a7a7b7c7d7e7f8081828384858788898a8b8c8d8e8f90919293949596",
                        "9798999a9b9c9d9e9fa0a1a2a3a5a6a7a8a9aaabacadaeafb0b1b3b4b5b6b7b8",
                        "b9babbbcbebfc0c1c2c3c4c6c7c8c9cacbcccecfd0d1d2d4d5d6d7d8dadbdcdd",
                        "dfe0e1e2e4e5e6e7e9eaebedeeeff1f2f4f5f6f8f9fafbfcfdfdfefeffffffff",
                    ),
                    concat!(
                        "0001020304050708090a0b0c0d0e0f1011121315161718191a1b1c1d1e1f2021",
                        "232425262728292a2b2c2d2e2f3031333435363738393a3b3c3d3e3f40414243",
                        "45464748494a4b4c4d4e4f5051525354555657585a5b5c5d5e5f606162636465",
                        "666768696a6b6c6d6e6f7071727374757677787a7b7c7d7e7f80818283848586",
                        "8788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6",
                        "a7a8a9aaabacadaeaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebfc0c1c2c3c4c5",
                        "c6c7c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d7d8d9dadbdcdddedfe0e1e2e2",
                        "e3e4e5e6e7e8e9eaebececedeeeff0f1f2f3f4f4f5f6f7f8f9fafbfcfcfdfeff",
                    ),
                    concat!(
                        "000d161c22262a2e3235383b3d40424547494b4d4f51535556585a5c5d5f6062",
                        "63656668696a6c6d6e7071727375767778797a7c7d7e7f808182838485868788",
                        "898a8b8c8d8e8f90919293949495969798999a9b9b9c9d9e9f9fa0a1a2a3a3a4",
                        "a5a6a7a7a8a9aaaaabacadadaeafafb0b1b2b2b3b4b4b5b6b6b7b8b9b9babbbb",
                        "bcbdbdbebebfc0c0c1c2c2c3c4c4c5c5c6c7c7c8c8c9cacacbcbcccdcdcececf",
                        "d0d0d1d1d2d2d3d4d4d5d5d6d6d7d7d8d8d9dadadbdbdcdcdddddededfdfe0e0",
                        "e1e2e2e3e3e4e4e5e5e6e6e7e7e8e8e9e9eaeaebebececededeeeeeeefeff0f0",
                        "f1f1f2f2f3f3f4f4f5f5f6f6f6f7f7f8f8f9f9fafafbfbfbfcfcfdfdfefeffff",
                    ),
                ],
            ),
            (
                "macos",
                GammaParams::macos(),
                0x85528d34f620e3c5_u64,
                [
                    concat!(
                        "000001010202030304040405050606070708080909090a0a0b0b0c0c0d0d0e0e",
                        "0f0f1010111111121213131414151516161717181819191a1a1b1b1c1c1d1d1e",
                        "1f1f202021212222232324242525262727282829292a2a2b2b2c2d2d2e2e2f2f",
                        "303131323233343435353637373838393a3a3b3b3c3d3d3e3f3f404141424243",
                        "4444454646474849494a4b4b4c4d4d4e4f50505152525354555556575858595a",
                        "5b5c5c5d5e5f6060616263646465666768696a6b6b6c6d6e6f70717273747576",
                        "7778797a7b7c7d7e7f8081828385868788898a8c8d8e8f919293959697999a9c",
                        "9d9fa0a2a3a5a7a9aaacaeb0b2b4b6b8babdbfc2c4c7cacdd1d5d9dde3e9f2ff",
                    ),
                    concat!(
                        "0001020303040506070808090a0b0c0d0e0e0f10111213141515161718191a1b",
                        "1b1c1d1e1f20212222232425262728292a2a2b2c2d2e2f303132333334353637",
                        "38393a3b3c3c3d3e3f40414243444546474848494a4b4c4d4e4f505152535455",
                        "56575758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f7070717273",
                        "7475767778797a7b7c7d7e7f808182838485868788898a8b8c8e8f9091929394",
                        "95969798999a9b9c9d9e9fa0a1a2a3a4a5a7a8a9aaabacadaeafb0b1b2b3b4b6",
                        "b7b8b9babbbcbdbebfc0c2c3c4c5c6c7c8c9cacccdcecfd0d1d2d3d5d6d7d8d9",
                        "dadbdddedfe0e1e2e4e5e6e7e8e9ebecedeeeff1f2f3f4f5f7f8f9fafbfdfeff",
                    ),
                    concat!(
                        "000d161c22262a2e3235383b3d40424547494b4d4f51535556585a5c5d5f6062",
                        "63656668696a6c6d6e7071727375767778797a7c7d7e7f808182838485868788",
                        "898a8b8c8d8e8f90919293949495969798999a9b9b9c9d9e9f9fa0a1a2a3a3a4",
                        "a5a6a7a7a8a9aaaaabacadadaeafafb0b1b2b2b3b4b4b5b6b6b7b8b9b9babbbb",
                        "bcbdbdbebebfc0c0c1c2c2c3c4c4c5c5c6c7c7c8c8c9cacacbcbcccdcdcececf",
                        "d0d0d1d1d2d2d3d4d4d5d5d6d6d7d7d8d8d9dadadbdbdcdcdddddededfdfe0e0",
                        "e1e2e2e3e3e4e4e5e5e6e6e7e7e8e8e9e9eaeaebebececededeeeeeeefeff0f0",
                        "f1f1f2f2f3f3f4f4f5f5f6f6f6f7f7f8f8f9f9fafafbfbfbfcfcfdfdfefeffff",
                    ),
                ],
            ),
            (
                "skia_default",
                GammaParams {
                    contrast: 0.5,
                    gamma: Gamma::Srgb,
                    luminance_scale: LuminanceScale::Full,
                },
                0xff587c833d8ee705_u64,
                [
                    concat!(
                        "0001010203030405050607070809090a0b0b0c0d0d0e0f0f1011111213131415",
                        "1516171718191a1a1b1c1c1d1e1e1f2020212222232425252627272829292a2b",
                        "2c2c2d2e2e2f30303132333334353536373838393a3b3b3c3d3d3e3f40404142",
                        "434344454546474848494a4b4b4c4d4e4e4f505151525354555556575858595a",
                        "5b5c5c5d5e5f5f6061626363646566676768696a6b6c6c6d6e6f707171727374",
                        "7576767778797a7b7c7d7d7e7f80818283848585868788898a8b8c8d8e8f9091",
                        "92939495969798999a9b9c9d9e9fa0a1a2a3a4a6a7a8a9aaabadaeafb0b1b3b4",
                        "b5b7b8b9bbbcbebfc0c2c4c5c7c8caccced0d2d4d6d8dadde0e2e6e9edf2f9ff",
                    ),
                    concat!(
                        "000102030405060708090a0b0c0d0e0f0f101112131415161718191a1b1c1d1e",
                        "1f202122232425262728292a2b2c2d2e2f30313232333435363738393a3b3c3d",
                        "3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d",
                        "5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d",
                        "7e7f808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d",
                        "9e9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbd",
                        "bebfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d2d3d4d5d6d7d8d9dadbdcddde",
                        "dfe0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f3f4f5f6f7f8f9fafbfcfdfeff",
                    ),
                    concat!(
                        "000d161c22262a2e3235383b3d40424547494b4d4f51535556585a5c5d5f6062",
                        "63656668696a6c6d6e7071727375767778797a7c7d7e7f808182838485868788",
                        "898a8b8c8d8e8f90919293949495969798999a9b9b9c9d9e9f9fa0a1a2a3a3a4",
                        "a5a6a7a7a8a9aaaaabacadadaeafafb0b1b2b2b3b4b4b5b6b6b7b8b9b9babbbb",
                        "bcbdbdbebebfc0c0c1c2c2c3c4c4c5c5c6c7c7c8c8c9cacacbcbcccdcdcececf",
                        "d0d0d1d1d2d2d3d4d4d5d5d6d6d7d7d8d8d9dadadbdbdcdcdddddededfdfe0e0",
                        "e1e2e2e3e3e4e4e5e5e6e6e7e7e8e8e9e9eaeaebebececededeeeeeeefeff0f0",
                        "f1f1f2f2f3f3f4f4f5f5f6f6f6f7f7f8f8f9f9fafafbfbfbfcfcfdfdfefeffff",
                    ),
                ],
            ),
            (
                "c05_pow22",
                GammaParams {
                    contrast: 0.5,
                    gamma: Gamma::Power(2.2),
                    luminance_scale: LuminanceScale::Full,
                },
                0x05a23b708c11b3cf_u64,
                [
                    concat!(
                        "00010102030304050606070808090a0a0b0c0c0d0e0f0f101111121313141515",
                        "16171818191a1a1b1c1d1d1e1f1f20212122232424252626272829292a2b2b2c",
                        "2d2e2e2f30313132333334353636373838393a3b3b3c3d3e3e3f404141424344",
                        "444546474748494a4a4b4c4d4d4e4f505051525353545556565758595a5a5b5c",
                        "5d5d5e5f606161626364646566676868696a6b6c6d6d6e6f7071717273747576",
                        "767778797a7b7b7c7d7e7f8081828283848586878889898a8b8c8d8e8f909192",
                        "93949596969798999a9b9c9d9e9fa0a1a2a4a5a6a7a8a9aaabacadaeb0b1b2b3",
                        "b4b6b7b8b9bbbcbdbfc0c2c3c5c6c8c9cbcdced0d2d4d6d8dbdde0e3e7ebf0ff",
                    ),
                    concat!(
                        "000102030405060708090a0b0c0d0e0f10111213141516171819191a1b1c1d1e",
                        "1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e",
                        "3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e",
                        "5f606162636465666768696a6b6c6d6e6f70717273737475767778797a7b7c7d",
                        "7e7f808182838485868788898a8b8c8d8e8f9091929394959798999a9b9c9d9e",
                        "9fa0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbe",
                        "bfc0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcddde",
                        "dfe0e1e2e3e4e5e6e7e8e9ebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff",
                    ),
                    concat!(
                        "00141c21262a2e3134373a3d3f414446484a4c4e5051535557585a5b5d5e6061",
                        "63646667686a6b6c6d6f7071727375767778797a7b7c7d7e8081828384858687",
                        "8889898a8b8c8d8e8f9091929394949596979899999a9b9c9d9e9e9fa0a1a2a2",
                        "a3a4a5a5a6a7a8a8a9aaababacadaeaeafb0b0b1b2b3b3b4b5b5b6b7b7b8b9b9",
                        "babbbbbcbdbdbebfbfc0c1c1c2c2c3c4c4c5c6c6c7c7c8c9c9cacacbcccccdcd",
                        "cecfcfd0d0d1d2d2d3d3d4d4d5d6d6d7d7d8d8d9d9dadbdbdcdcdddddededfdf",
                        "e0e1e1e2e2e3e3e4e4e5e5e6e6e7e7e8e8e9e9eaeaebebececededeeeeefeff0",
                        "f0f1f1f2f2f3f3f4f4f5f5f6f6f7f7f8f8f9f9f9fafafbfbfcfcfdfdfefeffff",
                    ),
                ],
            ),
            (
                "linear_c03",
                GammaParams {
                    contrast: 0.3,
                    gamma: Gamma::Power(1.0),
                    luminance_scale: LuminanceScale::Full,
                },
                0xc36336ed981d4c2c_u64,
                [
                    concat!(
                        "00010304050608090a0c0d0e0f111213151617181a1b1c1d1f20212224252627",
                        "282a2b2c2d2f3031323335363738393b3c3d3e3f40424344454648494a4b4c4d",
                        "4e5051525354555658595a5b5c5d5e5f6162636465666768696a6c6d6e6f7071",
                        "727374757677787a7b7c7d7e7f808182838485868788898a8b8c8d8e8f909192",
                        "939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a7a8a9aaababacadaeafb0b1",
                        "b2b3b4b5b6b7b7b8b9babbbcbdbebfc0c0c1c2c3c4c5c6c7c7c8c9cacbcccdcd",
                        "cecfd0d1d2d3d3d4d5d6d7d8d8d9dadbdcdcdddedfe0e0e1e2e3e4e4e5e6e7e7",
                        "e8e9eaebebecedeeeeeff0f1f1f2f3f4f4f5f6f6f7f8f9f9fafbfbfcfdfefeff",
                    ),
                    concat!(
                        "0001020405060708090b0c0d0e0f1011131415161718191b1c1d1e1f20212324",
                        "25262728292a2c2d2e2f3031323335363738393a3b3c3d3e4041424344454647",
                        "48494a4c4d4e4f505152535455565758595b5c5d5e5f60616263646566676869",
                        "6a6b6c6d6f707172737475767778797a7b7c7d7e7f808182838485868788898a",
                        "8b8c8d8e8f909192939495969798999a9b9c9d9e9fa0a1a2a3a4a5a6a6a7a8a9",
                        "aaabacadaeafb0b1b2b3b4b5b6b7b8b8b9babbbcbdbebfc0c1c2c3c4c5c5c6c7",
                        "c8c9cacbcccdcecfcfd0d1d2d3d4d5d6d7d8d8d9dadbdcdddedfdfe0e1e2e3e4",
                        "e5e6e6e7e8e9eaebececedeeeff0f1f2f2f3f4f5f6f7f8f8f9fafbfcfdfdfeff",
                    ),
                    concat!(
                        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                        "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
                        "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
                        "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
                        "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
                        "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf",
                        "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
                        "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff",
                    ),
                ],
            ),
        ]
    }

    #[test]
    fn tables_equal_skia_byte_for_byte() {
        for (name, params, checksum, rows) in expected() {
            let gamma = MaskGamma::new(&params);
            let bytes = gamma.texture_bytes();
            assert_eq!(bytes.len(), ROWS * ROW_WIDTH, "{name}");
            for (row, hex_row) in [0, 3, 7].into_iter().zip(rows) {
                assert_eq!(gamma.row(row).unwrap().to_vec(), hex(hex_row), "{name} row {row}");
                assert_eq!(
                    &bytes[row * ROW_WIDTH..(row + 1) * ROW_WIDTH],
                    &hex(hex_row)[..],
                    "{name} row {row}"
                );
            }
            assert_eq!(fnv1a64(&bytes), checksum, "{name} checksum");
        }
    }

    #[test]
    fn rows_past_the_last_are_none() {
        assert!(MaskGamma::new(&GammaParams::linux()).row(ROWS).is_none());
    }

    #[test]
    fn linear_without_contrast_is_identity() {
        let params = GammaParams {
            contrast: 0.0,
            gamma: Gamma::Power(1.0),
            luminance_scale: LuminanceScale::Full,
        };
        let gamma = MaskGamma::new(&params);
        let identity: Vec<u8> = (0..=u8::MAX).collect();
        for row in 0..ROWS {
            assert_eq!(gamma.row(row).unwrap().to_vec(), identity);
        }
    }

    #[test]
    fn parameters_are_quantized_as_skia_stores_them() {
        assert_eq!(internal_gamma(Gamma::Power(1.2)), 76);
        assert_eq!(internal_gamma(Gamma::Power(2.2)), 140);
        assert_eq!(internal_gamma(Gamma::Srgb), 0);
        assert_eq!(internal_contrast(0.2), 51);
        assert_eq!(internal_contrast(0.5), 128);
        assert_eq!(internal_contrast(1.0), 255);
        let stored = GammaParams {
            gamma: Gamma::Power(1.1875),
            ..GammaParams::linux()
        };
        assert_eq!(MaskGamma::new(&GammaParams::linux()), MaskGamma::new(&stored));
    }

    #[test]
    fn scale_matches_sk_t_scale255() {
        for (b, &expected) in SCALE_3_BITS.iter().enumerate() {
            let b = u32::try_from(b).unwrap();
            assert_eq!(u32::from(expected), (b << 5) | (b << 2) | (b >> 1));
        }
    }

    /// The A8 row and the LCD rows Skia picks for one colour and one scale.
    type Picked = (usize, [usize; 3]);

    /// (name, colour, rows picked for Full, Half and ThreeQuarters), from the C++ program.
    const ROWS_BY_COLOR: [(&str, [u8; 3], [Picked; 3]); 8] = [
        (
            "black",
            [0x00, 0x00, 0x00],
            [(0, [0, 0, 0]), (0, [0, 0, 0]), (0, [0, 0, 0])],
        ),
        (
            "white",
            [0xff, 0xff, 0xff],
            [(7, [7, 7, 7]), (3, [3, 3, 3]), (5, [5, 5, 5])],
        ),
        (
            "grey80",
            [0x80, 0x80, 0x80],
            [(4, [4, 4, 4]), (2, [2, 2, 2]), (3, [3, 3, 3])],
        ),
        (
            "red",
            [0xff, 0x00, 0x00],
            [(1, [7, 0, 0]), (0, [3, 0, 0]), (1, [5, 0, 0])],
        ),
        (
            "green",
            [0x00, 0xff, 0x00],
            [(5, [0, 7, 0]), (2, [0, 3, 0]), (4, [0, 5, 0])],
        ),
        (
            "blue",
            [0x00, 0x00, 0xff],
            [(0, [0, 0, 7]), (0, [0, 0, 3]), (0, [0, 0, 5])],
        ),
        (
            "slate-500",
            [0x64, 0x74, 0x8b],
            [(3, [3, 3, 4]), (1, [1, 1, 2]), (2, [2, 2, 3])],
        ),
        (
            "sky-400",
            [0x38, 0xbd, 0xf8],
            [(4, [1, 5, 7]), (1, [0, 2, 3]), (3, [0, 4, 5])],
        ),
    ];

    #[test]
    fn rows_are_picked_as_skia_picks_them() {
        let scales = [
            LuminanceScale::Full,
            LuminanceScale::Half,
            LuminanceScale::ThreeQuarters,
        ];
        for (name, rgb, by_scale) in ROWS_BY_COLOR {
            for (luminance_scale, (a8, lcd)) in scales.into_iter().zip(by_scale) {
                let params = GammaParams {
                    luminance_scale,
                    ..GammaParams::windows()
                };
                assert_eq!(MaskGamma::luminance_row(&params, rgb), a8, "{name} {luminance_scale:?}");
                assert_eq!(MaskGamma::channel_rows(&params, rgb), lcd, "{name} {luminance_scale:?}");
            }
        }
    }

    #[test]
    fn presets_pick_their_scale() {
        let slate = [0x64, 0x74, 0x8b];
        assert_eq!(MaskGamma::luminance_row(&GammaParams::linux(), slate), 3);
        assert_eq!(MaskGamma::luminance_row(&GammaParams::windows(), slate), 3);
        assert_eq!(MaskGamma::luminance_row(&GammaParams::macos(), slate), 1);
        assert_eq!(MaskGamma::luminance_row(&GammaParams::linux(), NON_SOLID_PAINT_RGB), 4);
    }
}
