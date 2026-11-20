//! Every conversion between layout floats and pixel integers, in one place.
//! Each one saturates (Rust's float-to-int `as` clamps and maps NaN to 0),
//! which is what a pixel position wants.

#![expect(
    clippy::as_conversions,
    reason = "the crate's numeric casts live here; each saturates, which is the intent for pixel coordinates"
)]

#[must_use]
pub fn f32_to_i32(v: f32) -> i32 {
    v as i32
}

#[must_use]
pub fn f32_to_u32(v: f32) -> u32 {
    v as u32
}

#[must_use]
pub fn u32_to_f32(v: u32) -> f32 {
    v as f32
}

#[must_use]
pub fn i32_to_f32(v: i32) -> f32 {
    v as f32
}

#[must_use]
pub fn usize_to_f32(v: usize) -> f32 {
    v as f32
}

#[must_use]
pub fn u64_to_f64(v: u64) -> f64 {
    v as f64
}

#[must_use]
pub fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

/// A colour channel in `0.0..=1.0` as a byte.
#[must_use]
pub fn unit_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}
