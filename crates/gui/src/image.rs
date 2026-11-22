//! Raster images: [`Pixels`], `gui`'s own RGBA8 image (premultiplied, with
//! the scale it was drawn at), which decoded PNGs, screenshots and
//! read-backs all are; and [`ImageSource`], an image decoded once and shared
//! by every frame that shows it.

use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;

/// An RGBA8 image, premultiplied alpha, rows top to bottom with no padding;
/// `scale` is the physical pixels per logical pixel it was drawn at (1 for
/// a decoded file).
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    width: u32,
    height: u32,
    scale: f32,
    data: Vec<u8>,
}

/// Why an image could not be read or written.
#[derive(Debug)]
pub struct PixelsError(String);

impl std::fmt::Display for PixelsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PixelsError {}

fn error(e: impl std::fmt::Display) -> PixelsError {
    PixelsError(e.to_string())
}

/// `c × a / 255`, rounded, as premultiplying a byte does.
fn mul(c: u8, a: u8) -> u8 {
    let v = u32::from(c).saturating_mul(u32::from(a)).saturating_add(128);
    u8::try_from(v.saturating_add(v >> 8) >> 8).unwrap_or(u8::MAX)
}

impl Pixels {
    /// A transparent image `width`×`height`; `None` when either is 0 or it
    /// would not fit in memory.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Option<Self> {
        let len = Self::len(width, height)?;
        Some(Self {
            width,
            height,
            scale: 1.0,
            data: vec![0; len],
        })
    }

    fn len(width: u32, height: u32) -> Option<usize> {
        if width == 0 || height == 0 {
            return None;
        }
        usize::try_from(u64::from(width).checked_mul(u64::from(height))?.checked_mul(4)?).ok()
    }

    /// An image over `data` (premultiplied RGBA8 rows); `None` when its
    /// length is not `width × height × 4`.
    #[must_use]
    pub fn from_vec(width: u32, height: u32, data: Vec<u8>) -> Option<Self> {
        (Self::len(width, height)? == data.len()).then_some(Self {
            width,
            height,
            scale: 1.0,
            data,
        })
    }

    /// The image from straight-alpha RGBA8 rows.
    #[must_use]
    pub fn from_straight(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let mut out = Self::new(width, height)?;
        if rgba.len() != out.data.len() {
            return None;
        }
        for (px, s) in out.data.as_chunks_mut::<4>().0.iter_mut().zip(rgba.as_chunks::<4>().0) {
            let [r, g, b, a] = *s;
            *px = [mul(r, a), mul(g, a), mul(b, a), a];
        }
        Some(out)
    }

    /// The same pixels, marked as drawn at `scale`.
    #[must_use]
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Physical pixels per logical pixel.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Premultiplied RGBA8 rows.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height)
            .then(|| {
                let i = u64::from(y)
                    .saturating_mul(u64::from(self.width))
                    .saturating_add(u64::from(x));
                usize::try_from(i.saturating_mul(4)).ok()
            })
            .flatten()
    }

    /// The premultiplied pixel at `(x, y)`.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let i = self.index(x, y)?;
        let p = self.data.get(i..i.checked_add(4)?)?;
        Some([*p.first()?, *p.get(1)?, *p.get(2)?, *p.get(3)?])
    }

    /// The pixel at `(x, y)` with straight alpha.
    #[must_use]
    pub fn straight(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        self.pixel(x, y).map(demultiply)
    }

    /// Every pixel set to `rgba` (straight alpha).
    pub fn fill(&mut self, rgba: [u8; 4]) {
        let [r, g, b, a] = rgba;
        let p = [mul(r, a), mul(g, a), mul(b, a), a];
        for px in self.data.as_chunks_mut::<4>().0 {
            *px = p;
        }
    }

    /// The `width`×`height` part from `(x, y)`; `None` when it is empty or
    /// reaches past the image.
    #[must_use]
    pub fn crop(&self, x: i32, y: i32, width: u32, height: u32) -> Option<Self> {
        let (x0, y0) = (u32::try_from(x).ok()?, u32::try_from(y).ok()?);
        if x0.checked_add(width)? > self.width || y0.checked_add(height)? > self.height {
            return None;
        }
        let mut out = Self::new(width, height)?;
        out.scale = self.scale;
        let row = usize::try_from(width).ok()?.checked_mul(4)?;
        for r in 0..height {
            let from = self.index(x0, y0.checked_add(r)?)?;
            let to = out.index(0, r)?;
            out.data
                .get_mut(to..to.checked_add(row)?)?
                .copy_from_slice(self.data.get(from..from.checked_add(row)?)?);
        }
        Some(out)
    }

    /// `other` painted over this image with its top left at `(x, y)`, faded
    /// to `opacity` (source over, premultiplied).
    pub fn draw(&mut self, other: &Self, x: i32, y: i32, opacity: f32) {
        let k = crate::convert::unit_to_u8(opacity);
        for sy in 0..other.height {
            let Some(ty) = i64::from(y)
                .checked_add(i64::from(sy))
                .and_then(|v| u32::try_from(v).ok())
            else {
                continue;
            };
            for sx in 0..other.width {
                let Some(tx) = i64::from(x)
                    .checked_add(i64::from(sx))
                    .and_then(|v| u32::try_from(v).ok())
                else {
                    continue;
                };
                let (Some(s), Some(t)) = (other.pixel(sx, sy), self.index(tx, ty)) else {
                    continue;
                };
                let s = s.map(|c| mul(c, k));
                let inv = 255_u8.saturating_sub(s[3]);
                if let Some(d) = self.data.get_mut(t..t.saturating_add(4)) {
                    for (d, s) in d.iter_mut().zip(s) {
                        *d = s.saturating_add(mul(*d, inv));
                    }
                }
            }
        }
    }

    /// Decode a PNG of any colour type and depth.
    ///
    /// # Errors
    ///
    /// When the bytes are not a PNG this decoder reads.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, PixelsError> {
        Self::read_png(bytes)
    }

    /// Read and decode the PNG at `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not a PNG.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, PixelsError> {
        let mut bytes = Vec::new();
        std::fs::File::open(path.as_ref())
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(error)?;
        Self::read_png(&bytes)
    }

    fn read_png(bytes: &[u8]) -> Result<Self, PixelsError> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // Palettes and low depths become 8-bit channels, 16-bit ones 8-bit.
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(error)?;
        let size = reader.output_buffer_size().ok_or_else(|| error("PNG too large"))?;
        let mut buf = vec![0; size];
        let info = reader.next_frame(&mut buf).map_err(error)?;
        let (w, h) = (info.width, info.height);
        let count = usize::try_from(u64::from(w).saturating_mul(u64::from(h))).map_err(error)?;
        let src = buf.get(..info.buffer_size()).ok_or_else(|| error("short PNG"))?;
        let mut rgba = Vec::with_capacity(count.saturating_mul(4));
        match info.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(src),
            png::ColorType::Rgb => {
                for [r, g, b] in src.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[*r, *g, *b, 255]);
                }
            }
            png::ColorType::GrayscaleAlpha => {
                for [v, a] in src.as_chunks::<2>().0 {
                    rgba.extend_from_slice(&[*v, *v, *v, *a]);
                }
            }
            png::ColorType::Grayscale => {
                for v in src {
                    rgba.extend_from_slice(&[*v, *v, *v, 255]);
                }
            }
            png::ColorType::Indexed => return Err(error("indexed PNG left unexpanded")),
        }
        Self::from_straight(w, h, &rgba).ok_or_else(|| error("PNG of no pixels"))
    }

    /// The image as PNG bytes (straight alpha, as PNG stores it).
    ///
    /// # Errors
    ///
    /// When the encoder fails.
    pub fn encode_png(&self) -> Result<Vec<u8>, PixelsError> {
        let mut out = Vec::new();
        self.write_png(&mut out)?;
        Ok(out)
    }

    /// Write the image as a PNG to `path`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), PixelsError> {
        let file = std::fs::File::create(path.as_ref()).map_err(error)?;
        let mut w = BufWriter::new(file);
        self.write_png(&mut w)?;
        w.flush().map_err(error)
    }

    fn write_png(&self, w: impl Write) -> Result<(), PixelsError> {
        let mut encoder = png::Encoder::new(w, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(error)?;
        let straight: Vec<u8> = self
            .data
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| demultiply(*p))
            .collect();
        writer.write_image_data(&straight).map_err(error)?;
        writer.finish().map_err(error)
    }
}

/// A premultiplied pixel with straight alpha.
#[must_use]
pub fn demultiply([r, g, b, a]: [u8; 4]) -> [u8; 4] {
    if a == 0 {
        return [0, 0, 0, 0];
    }
    let un = |c: u8| {
        let v = u32::from(c).saturating_mul(255).saturating_add(u32::from(a) / 2);
        u8::try_from(v.checked_div(u32::from(a)).unwrap_or(0)).unwrap_or(u8::MAX)
    };
    [un(r), un(g), un(b), a]
}

/// A decoded image; cloning shares the pixels.
#[derive(Clone, Debug)]
pub struct ImageSource(pub(crate) Arc<Pixels>);

impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl ImageSource {
    /// Decode PNG bytes; `None` when they are not a PNG.
    #[must_use]
    pub fn from_png(bytes: &[u8]) -> Option<Self> {
        Pixels::decode_png(bytes).ok().map(|p| Self(Arc::new(p)))
    }

    /// An image from straight-alpha RGBA bytes, `width`×`height`.
    #[must_use]
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        Pixels::from_straight(width, height, rgba).map(|p| Self(Arc::new(p)))
    }

    /// Size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.0.width(), self.0.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_round_trips_through_straight_alpha() {
        let mut p = Pixels::new(3, 2).expect("an image");
        p.fill([200, 100, 50, 128]);
        let bytes = p.encode_png().expect("encoded");
        let back = Pixels::decode_png(&bytes).expect("decoded");
        assert_eq!((back.width(), back.height()), (3, 2));
        for (a, b) in p.data().iter().zip(back.data()) {
            assert!(a.abs_diff(*b) <= 1, "{a} against {b}");
        }
    }

    #[test]
    fn drawing_over_blends_source_over() {
        let mut p = Pixels::new(2, 1).expect("an image");
        p.fill([255, 255, 255, 255]);
        let mut red = Pixels::new(1, 1).expect("an image");
        red.fill([255, 0, 0, 255]);
        p.draw(&red, 1, 0, 0.5);
        assert_eq!(p.straight(0, 0), Some([255, 255, 255, 255]));
        assert_eq!(p.straight(1, 0), Some([255, 127, 127, 255]));
        let c = p.crop(1, 0, 1, 1).expect("in the image");
        assert_eq!(c.straight(0, 0), Some([255, 127, 127, 255]));
    }
}
