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
    /// The focused node is it or inside it.
    pub focus_within: bool,
}

type Hook = Rc<dyn Fn(&ScrollLayout) -> bool>;
type Reveal = Rc<dyn Fn(ElementKey) -> bool>;

struct Inner {
    /// Bumped when a layout differs from the last: what a render read
    /// through [`ScrollHandle::layout`] renders again.
    version: crate::Signal<u64>,
    /// Bumped when the offset is set other than by a layout: the layout
    /// that placed the container reads it, so a wheel or `scroll_to`
    /// places that container again and nothing else (ADR-0013).
    moved: crate::Signal<u64>,
    /// Where the next layout puts the container, clamped then.
    offset: Cell<(f32, f32)>,
    laid: RefCell<Option<ScrollLayout>>,
    /// The wheel moved it since the last [`ScrollHandle::take_wheeled`].
    wheeled: Cell<bool>,
    /// Bumped when the program or the UI's own wheel sets the offset: the
    /// compositor drops what it scrolled before (ADR-0015).
    generation: Cell<u64>,
    hook: RefCell<Option<Hook>>,
    /// Brings the child keyed so into view (a virtual list's row, built or not).
    reveal: RefCell<Option<Reveal>>,
    /// How far the compositor may scroll it, at its speed, before the
    /// commit after this frame's shows (ADR-0015): what a virtual list
    /// builds beyond its viewport that way.
    reach: Cell<(f32, f32)>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            version: crate::reactive::detached(|| crate::Signal::labeled("scroll layout", 0)),
            moved: crate::reactive::detached(|| crate::Signal::labeled("scroll offset", 0)),
            offset: Cell::default(),
            laid: RefCell::default(),
            wheeled: Cell::default(),
            generation: Cell::default(),
            hook: RefCell::default(),
            reveal: RefCell::default(),
            reach: Cell::default(),
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.version.dispose_quietly();
        self.moved.dispose_quietly();
    }
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
        self.0.version.with(|_| ());
        self.0.offset.get()
    }

    /// Scroll to `offset`; the next layout clamps it to what the content allows.
    pub fn scroll_to(&self, offset: (f32, f32)) {
        self.0.offset.set(offset);
        self.0.generation.set(self.0.generation.get().wrapping_add(1));
        self.moved();
    }

    fn moved(&self) {
        self.0.moved.update(|m| *m = m.wrapping_add(1));
    }

    /// The offset the next layout puts it at, read by that layout: a set
    /// offset places the container again.
    pub(crate) fn wanted(&self) -> (f32, f32) {
        self.0.moved.with(|_| ());
        self.0.offset.get()
    }

    /// How many times the offset was set other than by the compositor's scroll.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.0.generation.get()
    }

    /// The container as last laid out; `None` before its first layout.
    #[must_use]
    pub fn layout(&self) -> Option<ScrollLayout> {
        self.0.version.with(|_| ());
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

    /// Let `f` bring the child keyed `key` into view when assistive
    /// technology asks for it (`ScrollIntoView`): a container whose children
    /// are not all built (a virtual list) knows where one lies. It returns
    /// whether it scrolled.
    pub fn on_reveal(&self, f: impl Fn(ElementKey) -> bool + 'static) {
        *self.0.reveal.borrow_mut() = Some(Rc::new(f));
    }

    /// Bring the child keyed `key` into view through [`Self::on_reveal`];
    /// `None` when the container has no such hook.
    pub(crate) fn reveal(&self, key: ElementKey) -> Option<bool> {
        let f = self.0.reveal.borrow().clone();
        f.map(|f| f(key))
    }

    pub(crate) fn laid_out(&self, layout: ScrollLayout) {
        self.0.offset.set(layout.offset);
        let same = self.0.laid.borrow().as_ref() == Some(&layout);
        if !same {
            *self.0.laid.borrow_mut() = Some(layout);
            self.0.version.update(|v| *v = v.wrapping_add(1));
        }
    }

    /// The UI's own wheel moved it to `offset`.
    pub(crate) fn wheel(&self, offset: (f32, f32)) {
        self.0.offset.set(offset);
        self.0.wheeled.set(true);
        self.0.generation.set(self.0.generation.get().wrapping_add(1));
        self.moved();
    }

    /// The compositor's wheel moved it to `offset` (a main frame brought the
    /// scroll it did): the reader scrolled, and nothing it did is dropped.
    /// How far the compositor may scroll it before the commit after this
    /// frame's shows, signed as an offset moves (zero when it is still).
    #[must_use]
    pub fn reach(&self) -> (f32, f32) {
        self.0.reach.get()
    }

    pub(crate) fn set_reach(&self, reach: (f32, f32)) {
        self.0.reach.set(reach);
    }

    pub(crate) fn synced(&self, offset: (f32, f32)) {
        self.0.offset.set(offset);
        self.0.wheeled.set(true);
        self.moved();
    }

    /// Run the hook on the last layout; whether the frame must be laid out again.
    pub(crate) fn after_layout(&self) -> bool {
        // Cloned out, so the hook may set a new one.
        let hook = self.0.hook.borrow().clone();
        let laid = self.0.laid.borrow().clone();
        match (hook, laid) {
            (Some(f), Some(l)) => f(&l),
            _ => false,
        }
    }

    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
