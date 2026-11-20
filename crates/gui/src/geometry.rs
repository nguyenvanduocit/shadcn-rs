//! Boxes and node identity, in logical (unscaled) window pixels.

use std::fmt;

use crate::element::ElementKey;

/// A rectangle in logical window pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Bounds {
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    #[must_use]
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The overlap of two boxes; empty boxes have zero size.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        Self {
            x,
            y,
            width: (self.right().min(other.right()) - x).max(0.0),
            height: (self.bottom().min(other.bottom()) - y).max(0.0),
        }
    }

    /// This box grown by `d` on every side (shrunk when negative).
    #[must_use]
    pub fn outset(&self, d: f32) -> Self {
        Self {
            x: self.x - d,
            y: self.y - d,
            width: (self.width + 2.0 * d).max(0.0),
            height: (self.height + 2.0 * d).max(0.0),
        }
    }

    /// This box shrunk by per-side amounts.
    #[must_use]
    pub fn inset(&self, top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            x: self.x + left,
            y: self.y + top,
            width: (self.width - left - right).max(0.0),
            height: (self.height - top - bottom).max(0.0),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// This box with each edge on the nearest of `scale` device pixels per
    /// logical one, where Blink paints a box (`PixelSnappedIntRect`): edges
    /// stay crisp, and two boxes that touch still touch.
    #[must_use]
    pub fn snapped(&self, scale: f32) -> Self {
        let snap = |v: f32| (v * scale + 0.5).floor() / scale;
        let (x, y) = (snap(self.x), snap(self.y));
        Self {
            x,
            y,
            width: snap(self.right()) - x,
            height: snap(self.bottom()) - y,
        }
    }
}

impl fmt::Display for Bounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:.0}x{:.0} at ({:.0},{:.0})",
            self.width, self.height, self.x, self.y
        )
    }
}

/// One step of a [`NodePath`]: a child's position among its siblings, or its
/// key when it has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSeg {
    Index(usize),
    Key(ElementKey),
}

/// Where a node sits in the tree: the same node across frames has the same
/// path, so keyed list items keep it when they reorder.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct NodePath(pub Vec<PathSeg>);

impl NodePath {
    /// Nesting depth: 0 for the root.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len().saturating_sub(1)
    }

    #[must_use]
    pub fn child(&self, seg: PathSeg) -> Self {
        let mut segs = self.0.clone();
        segs.push(seg);
        Self(segs)
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, seg) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            match seg {
                PathSeg::Index(n) => write!(f, "{n}")?,
                PathSeg::Key(k) => write!(f, "k{k}")?,
            }
        }
        Ok(())
    }
}
