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

    /// This box moved by `(dx, dy)`.
    #[must_use]
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }

    /// The smallest box holding both.
    #[must_use]
    pub fn union(&self, other: &Self) -> Self {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        Self {
            x,
            y,
            width: self.right().max(other.right()) - x,
            height: self.bottom().max(other.bottom()) - y,
        }
    }

    /// Whether `other` lies wholly inside this box.
    #[must_use]
    pub fn holds(&self, other: &Self) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
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
/// key when it has one; the root of a keyed component's output is placed by
/// the component's key, with its own key if it has one (a segment no
/// element's key can make, so a component keyed 7 and a sibling node keyed
/// 7 never share a path, and the root keeps its path as its keyed siblings
/// are reordered).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathSeg {
    Index(usize),
    Key(ElementKey),
    Component(ElementKey, Option<ElementKey>),
}

impl PathSeg {
    /// The node's own key (an element's, not its component's).
    #[must_use]
    pub fn element_key(self) -> Option<ElementKey> {
        match self {
            Self::Key(k) => Some(k),
            Self::Component(_, own) => own,
            Self::Index(_) => None,
        }
    }
}

/// Where a node sits in the tree: the same node across frames has the same
/// path, so keyed list items keep it when they reorder. Shared: the frame,
/// the layout tree and what moves hold the same one.
///
/// It carries a 64-bit key of its segments, made from its parent's key and
/// its last segment when it is made ([`NodePath::child`]): hashing a path,
/// comparing two that differ and naming a node to the compositor
/// ([`crate::compositor::NodeKey`]) cost one word, not the path's depth.
#[derive(Clone, Default)]
pub struct NodePath(pub std::rc::Rc<[PathSeg]>, u64);

impl NodePath {
    /// The path of these segments, root first.
    #[must_use]
    pub fn new(segs: std::rc::Rc<[PathSeg]>) -> Self {
        let key = segs.iter().fold(0, |k, seg| Self::step(k, *seg));
        Self(segs, key)
    }

    /// Nesting depth: 0 for the root.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len().saturating_sub(1)
    }

    #[must_use]
    pub fn child(&self, seg: PathSeg) -> Self {
        Self(
            self.0.iter().copied().chain(std::iter::once(seg)).collect(),
            Self::step(self.1, seg),
        )
    }

    /// Its key: the same for the same segments, in every frame and thread
    /// of the process (a fixed seed).
    #[must_use]
    pub fn key(&self) -> u64 {
        self.1
    }

    /// The key of a path one segment below the one keyed `parent`.
    fn step(parent: u64, seg: PathSeg) -> u64 {
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut h = foldhash::fast::FixedState::with_seed(0x5eed_9a74).build_hasher();
        parent.hash(&mut h);
        seg.hash(&mut h);
        h.finish()
    }
}

impl PartialEq for NodePath {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1 && (std::rc::Rc::ptr_eq(&self.0, &other.0) || self.0 == other.0)
    }
}

impl Eq for NodePath {}

impl std::hash::Hash for NodePath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u64(self.1);
    }
}

impl fmt::Debug for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NodePath").field(&self.0).finish()
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
                PathSeg::Component(c, None) => write!(f, "c{c}")?,
                PathSeg::Component(c, Some(k)) => write!(f, "c{c}k{k}")?,
            }
        }
        Ok(())
    }
}

/// `offset` rounded to the nearest whole device pixel at `scale` device
/// pixels per logical one: a moving translation, applied as a transform, so
/// what it moves stays on the pixel grid (ADR-0020). Left as it is when
/// `scale` is not positive.
#[must_use]
pub fn device_rounded(offset: (f32, f32), scale: f32) -> (f32, f32) {
    if scale.is_nan() || scale <= 0.0 {
        return offset;
    }
    let round = |v: f32| (v * scale).round() / scale;
    (round(offset.0), round(offset.1))
}

/// `offset` down to whole device pixels at `scale` (device pixels per
/// logical pixel), so what it moves keeps its pixels (ADR-0020); as it is
/// when the scale is unknown. Floored, as Blink floors the scroll offset it
/// paints at (`PaintLayerScrollableArea`'s `FlooredIntSize`); a thousandth
/// of a device pixel short of a whole one counts as it.
#[must_use]
pub fn device_snapped(offset: (f32, f32), scale: f32) -> (f32, f32) {
    if scale.is_nan() || scale <= 0.0 {
        return offset;
    }
    let snap = |v: f32| (v * scale + 1e-3).floor() / scale;
    (snap(offset.0), snap(offset.1))
}
