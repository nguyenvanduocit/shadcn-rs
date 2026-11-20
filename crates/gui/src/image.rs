//! Raster images, decoded once and shared by every frame that shows them.

use std::rc::Rc;

use tiny_skia::Pixmap;

/// A decoded image; cloning shares the pixels.
#[derive(Clone, Debug)]
pub struct ImageSource(pub(crate) Rc<Pixmap>);

impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl ImageSource {
    /// Decode PNG bytes; `None` when they are not a PNG.
    #[must_use]
    pub fn from_png(bytes: &[u8]) -> Option<Self> {
        Pixmap::decode_png(bytes).ok().map(|p| Self(Rc::new(p)))
    }

    /// An image from straight-alpha RGBA bytes, `width`×`height`.
    #[must_use]
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<Self> {
        let mut pixmap = Pixmap::new(width, height)?;
        let (chunks, _) = rgba.as_chunks::<4>();
        for (px, &[r, g, b, a]) in pixmap.pixels_mut().iter_mut().zip(chunks) {
            *px = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
        }
        Some(Self(Rc::new(pixmap)))
    }

    /// Size in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.0.width(), self.0.height())
    }
}
