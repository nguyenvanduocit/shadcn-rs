//! A scroll container whose offset code holds: [`ScrollHandle`] says where the
//! container and its children were laid out and how far it scrolls, moves it,
//! and runs a hook after each layout that may ask for the frame to be laid out
//! again (the DOM's `scrollTop`, `getBoundingClientRect` and `ResizeObserver`
//! in one, as GPUI's `ScrollHandle`). A container without one keeps its offset
//! in the frame's interaction state, moved only by the wheel.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use crate::element::ElementKey;
use crate::geometry::Bounds;
use crate::style::Edges;

/// A scroll container as last laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollLayout {
    /// The border box in window pixels.
    pub bounds: Bounds,
    pub padding: Edges<f32>,
    /// The offset shown, within `0..=max` on each axis.
    pub offset: (f32, f32),
    /// The largest offset on each axis.
    pub max: (f32, f32),
    /// The children, in order.
    pub children: Vec<ScrollChild>,
}

/// A child of a scroll container.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollChild {
    pub key: Option<ElementKey>,
    /// Its border box from the container's top-left corner, unscrolled.
    pub bounds: Bounds,
}

type Hook = Rc<dyn Fn(&ScrollLayout) -> bool>;

#[derive(Default)]
struct Inner {
    /// Where the next layout puts the container, clamped then.
    offset: Cell<(f32, f32)>,
    laid: RefCell<Option<ScrollLayout>>,
    /// The wheel moved it since the last [`ScrollHandle::take_wheeled`].
    wheeled: Cell<bool>,
    hook: RefCell<Option<Hook>>,
}

/// Holds one scroll container's offset (see [`crate::Div::track_scroll`]).
#[derive(Clone, Default)]
pub struct ScrollHandle(Rc<Inner>);

impl fmt::Debug for ScrollHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollHandle")
            .field("offset", &self.0.offset.get())
            .finish_non_exhaustive()
    }
}

impl ScrollHandle {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The offset shown, or asked for since the last layout.
    #[must_use]
    pub fn offset(&self) -> (f32, f32) {
        self.0.offset.get()
    }

    /// Scroll to `offset`; the next layout clamps it to what the content allows.
    pub fn scroll_to(&self, offset: (f32, f32)) {
        self.0.offset.set(offset);
    }

    /// The container as last laid out; `None` before its first layout.
    #[must_use]
    pub fn layout(&self) -> Option<ScrollLayout> {
        self.0.laid.borrow().clone()
    }

    /// Whether the reader scrolled it with the wheel since the last call.
    pub fn take_wheeled(&self) -> bool {
        self.0.wheeled.replace(false)
    }

    /// Run `f` after each layout of the container. It returns whether it
    /// changed what the frame is built from (a signal the view reads, the
    /// offset), and then the frame is laid out again before it is painted.
    pub fn on_layout(&self, f: impl Fn(&ScrollLayout) -> bool + 'static) {
        *self.0.hook.borrow_mut() = Some(Rc::new(f));
    }

    pub(crate) fn laid_out(&self, layout: ScrollLayout) {
        self.0.offset.set(layout.offset);
        *self.0.laid.borrow_mut() = Some(layout);
    }

    pub(crate) fn wheel(&self, offset: (f32, f32)) {
        self.0.offset.set(offset);
        self.0.wheeled.set(true);
    }

    /// Run the hook on the last layout; whether the frame must be laid out again.
    pub(crate) fn after_layout(&self) -> bool {
        // Cloned out, so the hook may set a new one.
        let hook = self.0.hook.borrow().clone();
        let laid = self.layout();
        match (hook, laid) {
            (Some(f), Some(l)) => f(&l),
            _ => false,
        }
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
