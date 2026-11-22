//! A list of any number of rows, of any heights, that builds only the rows
//! in view and a prefetch margin around them (ADR-0014).
//!
//! - **Heights** live in an [`ExtentTree`]: rows not yet seen take an
//!   estimate (the mean of the rows measured), rows laid out keep their
//!   measured height, and offset to row is O(log n).
//! - **Scroll position** is an [`Anchor`]: a row, its key, and how far its
//!   top lies above the viewport's (below it when negative). It is never an
//!   absolute `y`: the rows built are laid out from the first of them, so
//!   every `f32` is local to the window, and a list ten million rows long
//!   scrolls as exactly as a short one. A row above the anchor measured
//!   taller does not move what is in view; data inserted above it finds it
//!   again by its key.
//! - **Going to a row** ([`ListState::scroll_to`], [`ListState::scroll_into_view`])
//!   lands exactly where asked: the anchor asked for is held until the rows
//!   around it are measured and the viewport shows it, estimates or not.
//! - **The window** (the rows built) is the viewport plus the prefetch
//!   margin above and below, decided at render from the anchor and the
//!   viewport last laid out. The rows are the children of a scroll
//!   container whose offset is local to the window; after each layout the
//!   list measures them, moves its anchor to where the wheel left the
//!   viewport, and asks for another pass when the window must move (the
//!   same frame shows it: [`ScrollHandle::on_layout`]). Its owner may then
//!   act on where the rows lie ([`ListState::on_layout`]).
//! - **Its scrollbar** is its own, drawn as a scroll container's: the thumb
//!   is placed from the anchor's offset over the whole extent (a `u64` in
//!   1/64 px), and dragging it puts the anchor at the row under that
//!   fraction, measured or not. The rows measured after that do not move
//!   the anchor, so the view does not jump.
//! - **Tail follow** ([`ListState::follow_tail`]): a list at its end stays
//!   at its end as rows are added (a chat, a log). Room below the last row
//!   ([`ListState::set_end_space`]) lets the last rows scroll to the top.
//! - **Sized by its rows** ([`ListState::fit_rows`]): a list as tall as its
//!   rows up to a limit (a menu, a command palette), measured, not built whole.
//! - **Data** changes: the caller reports splices ([`ListState::splice`]),
//!   or the list sees its row count change at the end. The keyed diff of
//!   the layout tree then runs over the window only.
//! - **Accessibility** (ADR-0021): the list carries the whole row count and
//!   each row its index, so a screen reader announces "row 4 812 of
//!   10 000 000". A row whose element names a row's role (a table's row,
//!   an option) carries them itself; one that names another (a heading
//!   among options) is left as it is; any other is wrapped in a row of the
//!   role the list's implies (a list item, a table's row, an option). The row holding focus
//!   and the row its owner calls active ([`ListState::set_active`]) stay
//!   built when they scroll out of the window, above the viewport where
//!   nothing shows them, so focus, a caret, a composition and a screen
//!   reader's place survive scrolling; a screen reader's `ScrollIntoView`
//!   on a row brings it into view by its index.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::{Rc, Weak};

use super::extent_tree::{ExtentTree, UNITS, pixels, units};
use crate::element::{Div, Element, ElementKey, IntoElement, Role, div};
use crate::events::{DragEvent, DragPhase};
use crate::reactive::{Signal, detached};
use crate::scroll::{ScrollHandle, ScrollLayout};
use crate::style::Length;
use crate::style::{BackgroundImage, Calc, Ink, LinearGradient, Stop};
use crate::styled::{Styled, px};

/// The viewport height a list assumes before its first layout.
const FIRST_VIEWPORT: f32 = 600.0;
/// Fewest pixels the thumb is tall (a scroll container's).
const MIN_THUMB: f32 = 20.0;
/// The thumb's inset from the track's ends and the viewport's right edge.
const THUMB_INSET: f32 = 2.0;
const THUMB_WIDTH: f32 = 6.0;
/// How far keys are looked for around the anchor's old row after the data changed.
const KEY_SEARCH: u64 = 512;
/// How close a placement must come to the one asked for.
const PLACED: f32 = 0.01;

/// Where a list is scrolled to: `offset` pixels of row `row` lie above the
/// viewport's top (a negative `offset`: its top lies that far below the
/// viewport's). `key` is the row's key, so the anchor follows its row when
/// rows are inserted or removed before it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Anchor {
    pub row: u64,
    pub key: Option<ElementKey>,
    pub offset: f32,
}

/// The rows built: `first..last`, and how far the viewport's top lies
/// below the first's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Window {
    first: u64,
    last: u64,
    local: f32,
}

/// One row built, as last laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RowLayout {
    pub row: u64,
    /// Its top from the viewport's top (negative above it).
    pub top: f32,
    pub height: f32,
}

/// A list as last laid out, once its window is settled: what its owner's
/// hook ([`ListState::on_layout`]) reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListLayout {
    /// The viewport's width and height.
    pub width: f32,
    pub viewport: f32,
    /// How far the viewport's top lies below the first row's top, in pixels
    /// (`f64`: exact over any extent a list reaches).
    pub top: f64,
    /// The rows' total height, measured and estimated, the end space left out.
    pub extent: f64,
    /// The rows built, in order.
    pub rows: Vec<RowLayout>,
    /// The reader moved it with the wheel since the last layout.
    pub wheeled: bool,
}

type Hook = Rc<dyn Fn(&ListLayout) -> bool>;

struct Inner {
    /// Bumped when the window moves: the list renders again.
    version: Signal<u64>,
    extents: RefCell<ExtentTree>,
    anchor: Cell<Anchor>,
    /// The anchor was asked for and the viewport does not show it yet: it is
    /// held through the layouts that measure the rows around it.
    requested: Cell<bool>,
    /// The anchor asked for holds the row's bottom this many pixels above
    /// the viewport's, whatever height the row is measured at.
    bottom: Cell<Option<f32>>,
    window: Cell<Window>,
    /// The keys of the window's rows, in order.
    keys: RefCell<Vec<ElementKey>>,
    /// The scroll container's content box as last laid out: width, height.
    viewport: Cell<(f32, f32)>,
    /// Its top in window pixels (where a thumb drag is measured from).
    top: Cell<f32>,
    scroll: ScrollHandle,
    /// Pixels built beyond the viewport, above and below; `None`: one viewport.
    prefetch: Cell<Option<f32>>,
    /// Keep the end in view as rows are added.
    follow: Cell<bool>,
    /// The view is at the end (where a following list stays).
    pinned: Cell<bool>,
    /// Asked to show the end, once.
    to_end: Cell<bool>,
    /// Room below the last row, in pixels.
    end_space: Cell<f32>,
    /// As tall as its rows, up to this many pixels; `None`: sized by its parent.
    fit: Cell<Option<f32>>,
    /// Its height when sized by its rows.
    height: Signal<f32>,
    /// The thumb is not drawn (`scrollbar-width: none`).
    no_scrollbar: Cell<bool>,
    /// Rows measured and their total height, for the estimate.
    measured: Cell<(u64, f64)>,
    /// Where on the thumb a drag took hold, from its top.
    grab: Cell<f32>,
    /// The thumb's top and height in the viewport; `None` when all rows fit.
    thumb: Signal<Option<(f32, f32)>>,
    /// The owner's hook, run once the window is settled.
    hook: RefCell<Option<Hook>>,
    /// The row holding focus, as last laid out, and its key.
    focus_row: Cell<Option<(u64, ElementKey)>>,
    /// The row its owner treats as current (an active option).
    active: Cell<Option<u64>>,
    /// The rows built outside the window (pinned), in order, and how many
    /// of them come before it.
    pins: RefCell<Vec<(u64, ElementKey)>>,
    pins_before: Cell<usize>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.version.dispose_quietly();
        self.thumb.dispose_quietly();
        self.height.dispose_quietly();
    }
}

/// A virtual list's state: heights, anchor, window. Clones share it.
#[derive(Clone)]
pub struct ListState(Rc<Inner>);

impl fmt::Debug for ListState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListState")
            .field("rows", &self.len())
            .field("anchor", &self.0.anchor.get())
            .field("window", &self.0.window.get())
            .finish_non_exhaustive()
    }
}

impl Default for ListState {
    fn default() -> Self {
        Self::new(32.0)
    }
}

impl PartialEq for ListState {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Pixels as units, signed (an anchor's offset).
fn signed_units(px: f64) -> f64 {
    (px * f64::from(UNITS)).round()
}

impl ListState {
    /// A list whose rows are about `estimate` pixels tall until measured.
    #[must_use]
    pub fn new(estimate: f32) -> Self {
        let inner = Rc::new(Inner {
            version: detached(|| Signal::labeled("virtual list", 0)),
            extents: RefCell::new(ExtentTree::new(0, units(estimate))),
            anchor: Cell::default(),
            requested: Cell::new(false),
            bottom: Cell::new(None),
            window: Cell::default(),
            keys: RefCell::default(),
            viewport: Cell::default(),
            top: Cell::default(),
            scroll: ScrollHandle::new(),
            prefetch: Cell::new(None),
            follow: Cell::new(false),
            pinned: Cell::new(false),
            to_end: Cell::new(false),
            end_space: Cell::new(0.0),
            fit: Cell::new(None),
            height: detached(|| Signal::labeled("virtual list height", 0.0)),
            no_scrollbar: Cell::new(false),
            measured: Cell::new((0, 0.0)),
            grab: Cell::new(0.0),
            thumb: detached(|| Signal::labeled("virtual list thumb", None)),
            hook: RefCell::new(None),
            focus_row: Cell::new(None),
            active: Cell::new(None),
            pins: RefCell::default(),
            pins_before: Cell::new(0),
        });
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        inner
            .scroll
            .on_layout(move |l| weak.upgrade().is_some_and(|i| Self(i).laid_out(l)));
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        inner
            .scroll
            .on_reveal(move |k| weak.upgrade().is_some_and(|i| Self(i).reveal(k)));
        Self(inner)
    }

    /// Rows it holds.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.0.extents.borrow().len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where it is scrolled to.
    #[must_use]
    pub fn anchor(&self) -> Anchor {
        self.0.version.with(|_| ());
        self.0.anchor.get()
    }

    /// The rows built (in view or prefetched).
    #[must_use]
    pub fn window(&self) -> std::ops::Range<u64> {
        self.0.version.with(|_| ());
        let w = self.0.window.get();
        w.first..w.last
    }

    /// The rows' total height in pixels, measured and estimated.
    #[must_use]
    pub fn extent(&self) -> f64 {
        pixels(self.0.extents.borrow().extent())
    }

    /// The top of `row` in pixels from the first row's top (the extent for `row == len`).
    #[must_use]
    pub fn offset_of(&self, row: u64) -> f64 {
        pixels(self.0.extents.borrow().offset_of(row))
    }

    /// `row`'s height in pixels, measured or estimated; 0 past the end.
    #[must_use]
    pub fn row_height(&self, row: u64) -> f32 {
        let h = self.0.extents.borrow().height(row).unwrap_or(0);
        crate::convert::f64_to_f32(pixels(u64::from(h)))
    }

    /// Change measured `row`'s height by `delta` pixels without laying it
    /// out: its owner changed the room it holds (a gap added below it as a
    /// row comes in after it), and a row not built is not measured again
    /// until it is. A row not measured keeps its estimate.
    pub fn adjust_row_height(&self, row: u64, delta: f32) {
        {
            let mut ext = self.0.extents.borrow_mut();
            if !ext.is_measured(row) {
                return;
            }
            let h = ext.height(row).unwrap_or(0);
            let d = units(delta.abs());
            let next = if delta >= 0.0 {
                h.saturating_add(d)
            } else {
                h.saturating_sub(d)
            };
            if !ext.set(row, next) {
                return;
            }
        }
        let (n, sum) = self.0.measured.get();
        self.0.measured.set((n, sum + f64::from(delta)));
        self.bump();
    }

    /// Whether `row`'s height was measured.
    #[must_use]
    pub fn is_measured(&self, row: u64) -> bool {
        self.0.extents.borrow().is_measured(row)
    }

    /// The viewport's height as last laid out (a first guess before that).
    #[must_use]
    pub fn viewport_height(&self) -> f32 {
        match self.0.viewport.get().1 {
            h if h > 0.0 => h,
            _ => FIRST_VIEWPORT,
        }
    }

    /// Runs its height tree holds (rows measured apart, not rows).
    #[must_use]
    pub fn runs(&self) -> usize {
        self.0.extents.borrow().runs()
    }

    /// Build `px` pixels of rows beyond the viewport on each side (one
    /// viewport by default).
    pub fn set_prefetch(&self, px: f32) {
        self.0.prefetch.set(Some(px.max(0.0)));
    }

    /// Keep the last row in view as rows are added while the view is at the end.
    pub fn follow_tail(&self, follow: bool) {
        if self.0.follow.replace(follow) == follow {
            return;
        }
        if follow {
            self.0.pinned.set(true);
        }
        self.bump();
    }

    /// Whether it follows its tail.
    #[must_use]
    pub fn follows_tail(&self) -> bool {
        self.0.follow.get()
    }

    /// Whether the view is at the end.
    #[must_use]
    pub fn is_at_end(&self) -> bool {
        self.0.version.with(|_| ());
        self.0.pinned.get()
    }

    /// Leave `px` pixels of room below the last row (so the last rows can
    /// scroll up to the top); 0 by default.
    pub fn set_end_space(&self, px: f32) {
        let px = px.max(0.0);
        if self.0.end_space.replace(px) != px {
            self.bump();
        }
    }

    /// The room below the last row, in pixels.
    #[must_use]
    pub fn end_space(&self) -> f32 {
        self.0.end_space.get()
    }

    /// Size the list by its rows (measured, the rows not built estimated),
    /// up to `max` pixels, as a box of `max-height` and `overflow: auto`
    /// does; `None`: it fills the height its parent gives it.
    pub fn fit_rows(&self, max: Option<f32>) {
        self.0.fit.set(max);
    }

    /// Draw no scrollbar (`scrollbar-width: none`), or draw it again.
    pub fn hide_scrollbar(&self, hide: bool) {
        self.0.no_scrollbar.set(hide);
    }

    /// Run `f` after each layout that leaves the window where it is (the
    /// rows built are the rows needed, placed as asked): what the owner of
    /// the list reads of where its rows lie. It returns whether it changed
    /// what the frame is built from, and then the frame is laid out again.
    pub fn on_layout(&self, f: impl Fn(&ListLayout) -> bool + 'static) {
        *self.0.hook.borrow_mut() = Some(Rc::new(f));
    }

    /// The row its owner treats as current, kept built (and in what a
    /// screen reader holds) while it is scrolled out of view: an option
    /// highlighted by the keyboard (`aria-activedescendant`). Read when the
    /// list renders.
    pub fn set_active(&self, row: Option<u64>) {
        self.0.active.set(row);
    }

    /// The row holding focus, as last laid out.
    #[must_use]
    pub fn focused_row(&self) -> Option<u64> {
        self.0.focus_row.get().map(|(row, _)| row)
    }

    /// The rows built outside the window, kept for focus and the active row.
    #[must_use]
    pub fn pinned(&self) -> Vec<u64> {
        self.0.version.with(|_| ());
        self.0.pins.borrow().iter().map(|(row, _)| *row).collect()
    }

    /// Bring the row keyed `key` into view (a screen reader asked); whether
    /// it is a row of this list.
    fn reveal(&self, key: ElementKey) -> bool {
        let w = self.0.window.get();
        let in_window = self
            .0
            .keys
            .borrow()
            .iter()
            .position(|k| *k == key)
            .map(|j| w.first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)));
        let row = in_window.or_else(|| self.0.pins.borrow().iter().find(|(_, k)| *k == key).map(|(r, _)| *r));
        match row {
            Some(r) => {
                self.scroll_into_view(r);
                true
            }
            None => false,
        }
    }

    /// The pinned rows for this render: focus and the active row, outside
    /// the window `w`, found again by key after the data moved.
    fn pins_for(&self, w: Window, rows: u64, key: &dyn Fn(u64) -> ElementKey) -> Vec<(u64, ElementKey)> {
        let focus = self.0.focus_row.get().and_then(|(row, k)| {
            if row < rows && key(row) == k {
                return Some((row, k));
            }
            let from = row.saturating_sub(KEY_SEARCH);
            let to = row.saturating_add(KEY_SEARCH).min(rows);
            (from..to).find(|&r| key(r) == k).map(|r| (r, k))
        });
        self.0.focus_row.set(focus);
        let active = self.0.active.get().filter(|&r| r < rows).map(|r| (r, key(r)));
        let mut pins: Vec<(u64, ElementKey)> = focus
            .into_iter()
            .chain(active)
            .filter(|(r, _)| !(w.first..w.last).contains(r))
            .collect();
        pins.sort_unstable_by_key(|(r, _)| *r);
        pins.dedup_by_key(|(r, _)| *r);
        pins
    }

    /// Scroll so `row`'s top is the viewport's.
    pub fn scroll_to_row(&self, row: u64) {
        self.scroll_to(Anchor {
            row,
            key: None,
            offset: 0.0,
        });
    }

    /// Scroll to `anchor`: the viewport shows it exactly once the rows
    /// around it are measured (within the extent: the start and the end
    /// clamp it).
    pub fn scroll_to(&self, anchor: Anchor) {
        self.0.pinned.set(false);
        self.0.to_end.set(false);
        self.0.requested.set(true);
        self.0.bottom.set(None);
        self.0.anchor.set(Anchor {
            row: anchor.row.min(self.len().saturating_sub(1)),
            ..anchor
        });
        self.bump();
    }

    /// Scroll so `row`'s bottom lies `above` pixels above the viewport's,
    /// at whatever height the row is measured.
    pub fn scroll_to_row_bottom(&self, row: u64, above: f32) {
        self.scroll_to(Anchor {
            row,
            key: None,
            offset: 0.0,
        });
        self.0.bottom.set(Some(above));
    }

    /// Scroll as little as shows `row` whole (its top when it is taller than
    /// the viewport); nothing when it shows already.
    pub fn scroll_into_view(&self, row: u64) {
        if row >= self.len() {
            return;
        }
        let vp = f64::from(self.viewport_height());
        let top = self.offset_of(row);
        let height = f64::from(self.row_height(row));
        let view = self.view_top();
        if top < view || height > vp {
            self.scroll_to_row(row);
        } else if top + height > view + vp {
            self.scroll_to_row_bottom(row, 0.0);
        }
    }

    /// The viewport's top in pixels from the first row's, as the anchor puts it.
    #[must_use]
    pub fn view_top(&self) -> f64 {
        let ext = self.0.extents.borrow();
        let top = self.top_units(&ext);
        pixels(top)
    }

    /// Scroll to the end (and follow it if [`Self::follow_tail`] is on).
    pub fn scroll_to_end(&self) {
        self.0.requested.set(false);
        self.0.pinned.set(true);
        self.0.to_end.set(true);
        self.bump();
    }

    /// Scroll so a `fraction` of the whole extent lies above the viewport.
    pub fn scroll_to_fraction(&self, fraction: f64) {
        let ext = self.0.extents.borrow();
        let vp = u64::from(units(self.viewport_height()));
        let room = self.total(&ext).saturating_sub(vp);
        let top = crate::convert::f64_to_u64(fraction.clamp(0.0, 1.0) * crate::convert::u64_to_f64(room));
        let (row, into) = ext.row_at(top);
        drop(ext);
        self.0.requested.set(false);
        self.0.pinned.set(fraction >= 1.0 && self.0.follow.get());
        self.0.to_end.set(fraction >= 1.0);
        self.0.anchor.set(Anchor {
            row,
            key: None,
            offset: crate::convert::f64_to_f32(pixels(into)),
        });
        self.bump();
    }

    /// Rows `at..at + removed` were replaced by `inserted` new ones (not
    /// yet measured); the anchor keeps its row.
    pub fn splice(&self, at: u64, removed: u64, inserted: u64) {
        self.0.extents.borrow_mut().splice(at, removed, inserted);
        let mut a = self.0.anchor.get();
        if a.row >= at.saturating_add(removed) {
            a.row = a.row.saturating_sub(removed).saturating_add(inserted);
        } else if a.row >= at {
            a.row = at;
            a.offset = 0.0;
        }
        self.0.anchor.set(a);
        let w = self.0.window.get();
        if w.last > at {
            // The rows built moved: the window is decided anew.
            self.0.window.set(Window::default());
        }
        self.bump();
    }

    fn bump(&self) {
        self.0.version.update(|v| *v = v.wrapping_add(1));
    }

    /// The rows' extent and the end space, in units.
    fn total(&self, ext: &ExtentTree) -> u64 {
        ext.extent().saturating_add(u64::from(units(self.0.end_space.get())))
    }

    /// The viewport's top in units as the anchor puts it, within the extent.
    fn top_units(&self, ext: &ExtentTree) -> u64 {
        let len = ext.len();
        if len == 0 {
            return 0;
        }
        let vp = u64::from(units(self.viewport_height()));
        let room = self.total(ext).saturating_sub(vp);
        if self.wants_end() {
            return room;
        }
        let a = self.0.anchor.get();
        let row = a.row.min(len.saturating_sub(1));
        let base = crate::convert::u64_to_f64(ext.offset_of(row));
        let offset = match self.0.bottom.get().filter(|_| self.0.requested.get()) {
            Some(above) => {
                let height = crate::convert::u64_to_f64(u64::from(ext.height(row).unwrap_or(0)));
                height - crate::convert::u64_to_f64(vp) + signed_units(f64::from(above))
            }
            None => signed_units(f64::from(a.offset)),
        };
        let top = (base + offset).max(0.0);
        crate::convert::f64_to_u64(top).min(room)
    }

    /// The window for the anchor as it is. The rows built stay built while
    /// they cover the viewport and half the prefetch margin on each side,
    /// so a scroll back and forth builds nothing (rows that left the
    /// viewport are kept as a pool of their own: their nodes, their caches
    /// and their text, ADR-0008's two-frame rule counting them in use).
    ///
    /// While the compositor scrolls the list, the window also covers how
    /// far it may scroll before the commit after this frame's shows (the
    /// scroll handle's reach, ADR-0015): the compositor scrolls only the
    /// rows committed, and a fling or a busy UI must not reach past them.
    /// Chromium's interest rect and Flutter's `cacheExtent` are the same
    /// margin; this one is sized by the scroll's speed and the UI's latency.
    fn compute(&self) -> Window {
        let ext = self.0.extents.borrow();
        let len = ext.len();
        if len == 0 {
            return Window::default();
        }
        let vp_px = self.viewport_height();
        let vp = u64::from(units(vp_px));
        let rows_end = ext.extent();
        let top = self.top_units(&ext);
        let over = u64::from(units(self.0.prefetch.get().unwrap_or(vp_px)));
        let reach = self.0.scroll.reach().1;
        let up = u64::from(units((-reach).max(0.0)));
        let down = u64::from(units(reach.max(0.0)));
        let span = |above: u64, below: u64| {
            let first = ext.row_at(top.saturating_sub(above)).0;
            let end = top.saturating_add(vp).saturating_add(below);
            let last = if end >= rows_end {
                len
            } else {
                ext.row_at(end).0.saturating_add(1).min(len)
            };
            (first, last)
        };
        let (need_first, need_last) = span((over / 2).saturating_add(up), (over / 2).saturating_add(down));
        let w = self.0.window.get();
        let (first, last) = if w.first < w.last && w.last <= len && w.first <= need_first && w.last >= need_last {
            (w.first, w.last)
        } else {
            span(over.saturating_add(up), over.saturating_add(down))
        };
        let origin = ext.offset_of(first);
        Window {
            first,
            last,
            local: crate::convert::f64_to_f32(pixels(top.saturating_sub(origin))),
        }
    }

    /// After a layout: measure the rows built, move the anchor to where the
    /// viewport is, and whether the window must move (another pass).
    fn laid_out(&self, l: &ScrollLayout) -> bool {
        let inner = &self.0;
        let width = l.bounds.width;
        let height = (l.bounds.height - l.padding.top - l.padding.bottom).max(0.0);
        let (old_width, _) = inner.viewport.get();
        inner.viewport.set((width, height));
        inner.top.set(l.bounds.y);
        if old_width > 0.0 && old_width != width {
            // Heights measured at another width: estimates again.
            let estimate = inner.extents.borrow().estimate();
            inner.extents.borrow_mut().reset(estimate);
            inner.measured.set((0, 0.0));
        }
        let w = inner.window.get();
        let built = usize::try_from(w.last.saturating_sub(w.first)).unwrap_or(usize::MAX);
        // The rows, not the pinned rows around them or the room below them.
        let before = inner.pins_before.get().min(l.children.len());
        let children = l
            .children
            .get(before..before.saturating_add(built).min(l.children.len()))
            .unwrap_or_default();
        let keys = inner.keys.borrow().clone();
        // The row holding focus, by its key, built in the window or pinned.
        let focus = l
            .children
            .iter()
            .find(|c| c.focus_within)
            .and_then(|c| c.key)
            .and_then(|k| {
                keys.iter()
                    .position(|key| *key == k)
                    .map(|j| w.first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)))
                    .or_else(|| inner.pins.borrow().iter().find(|(_, key)| *key == k).map(|(r, _)| *r))
                    .map(|r| (r, k))
            });
        let unpinned = inner
            .focus_row
            .get()
            .is_some_and(|(r, _)| !(w.first..w.last).contains(&r))
            && focus.is_none();
        inner.focus_row.set(focus);
        if unpinned {
            // Focus left the row kept built for it: it goes.
            self.bump();
            return true;
        }
        {
            let mut ext = inner.extents.borrow_mut();
            let (mut n, mut sum) = inner.measured.get();
            for (row, c) in (w.first..w.last).zip(children) {
                let fresh = !ext.is_measured(row);
                ext.set(row, units(c.bounds.height));
                if fresh {
                    n = n.saturating_add(1);
                    sum += f64::from(c.bounds.height);
                }
            }
            inner.measured.set((n, sum));
            if n > 0 {
                let mean = crate::convert::f64_to_f32(sum / crate::convert::u64_to_f64(n));
                ext.set_estimate(units(mean));
            }
        }
        // The anchor: the row at the viewport's top, unless one asked for is
        // not shown yet.
        let y = l.offset.1 + l.padding.top;
        if !inner.requested.get() {
            let at = children
                .iter()
                .enumerate()
                .find(|(_, c)| c.bounds.y + c.bounds.height > y)
                .or_else(|| children.iter().enumerate().next_back());
            if let Some((j, c)) = at {
                let j = u64::try_from(j).unwrap_or(u64::MAX);
                inner.anchor.set(Anchor {
                    row: w.first.saturating_add(j),
                    key: keys.get(usize::try_from(j).unwrap_or(usize::MAX)).copied(),
                    offset: (y - c.bounds.y).max(0.0),
                });
            }
        }
        let thumb_moved = self.place_thumb(l.offset.1, height);
        let fitted = self.fit_height();
        let wants_end = self.wants_end();
        let at_end = l.max.1 - l.offset.1 <= 0.5 && w.last >= self.len();
        inner.pinned.set(at_end);
        if at_end {
            inner.to_end.set(false);
        }
        let next = self.compute();
        let placed = if wants_end {
            at_end
        } else {
            (next.local - l.offset.1).abs() <= PLACED
        };
        if next.first != w.first || next.last != w.last || !placed {
            self.bump();
            return true;
        }
        // The rows built still cover it: nothing to build.
        inner.window.set(next);
        inner.requested.set(false);
        let hook = inner.hook.borrow().clone();
        let owner = hook.is_some_and(|f| {
            let origin = self.offset_of(w.first);
            f(&ListLayout {
                width,
                viewport: height,
                top: origin + f64::from(l.offset.1),
                extent: self.extent(),
                rows: (w.first..w.last)
                    .zip(children)
                    .map(|(row, c)| RowLayout {
                        row,
                        top: c.bounds.y - y,
                        height: c.bounds.height,
                    })
                    .collect(),
                wheeled: inner.scroll.take_wheeled(),
            })
        });
        thumb_moved | fitted | owner
    }

    /// The height of a list sized by its rows; whether it changed (the
    /// frame is laid out again to show it).
    fn fit_height(&self) -> bool {
        let Some(max) = self.0.fit.get() else {
            return false;
        };
        let rows = pixels(self.total(&self.0.extents.borrow()));
        let h = crate::convert::f64_to_f32(rows).min(max);
        if self.0.height.get_untracked() == h {
            return false;
        }
        self.0.height.set(h);
        true
    }

    /// Put the thumb where the viewport's top lies over the whole extent:
    /// `local` pixels below the first row built, as a scroll container
    /// places its own. Whether it moved (the frame is laid out again to show it).
    fn place_thumb(&self, local: f32, viewport: f32) -> bool {
        let w = self.0.window.get();
        let ext = self.0.extents.borrow();
        let total = self.total(&ext);
        let vp = u64::from(units(viewport));
        let geometry = (viewport > 0.0 && total > vp.saturating_add(32)).then(|| {
            let height = thumb_height(viewport, pixels(total));
            let room = total.saturating_sub(vp);
            let top = ext.offset_of(w.first).saturating_add(u64::from(units(local))).min(room);
            let fraction = if room == 0 {
                0.0
            } else {
                crate::convert::u64_to_f64(top) / crate::convert::u64_to_f64(room)
            };
            let travel = (viewport - height - 2.0 * THUMB_INSET).max(0.0);
            (THUMB_INSET + crate::convert::f64_to_f32(fraction) * travel, height)
        });
        drop(ext);
        if self.0.thumb.get_untracked() == geometry {
            return false;
        }
        self.0.thumb.set(geometry);
        true
    }

    /// Whether the view is to show the end, wherever the rows' measured
    /// heights put it.
    fn wants_end(&self) -> bool {
        (self.0.follow.get() && self.0.pinned.get()) || self.0.to_end.get()
    }

    /// Keep the anchor on its key after the data moved under it.
    fn follow_key(&self, key: &dyn Fn(u64) -> ElementKey) {
        let mut a = self.0.anchor.get();
        let len = self.len();
        let Some(k) = a.key else { return };
        if a.row < len && key(a.row) == k {
            return;
        }
        let from = a.row.saturating_sub(KEY_SEARCH);
        let to = a.row.saturating_add(KEY_SEARCH).min(len);
        if let Some(row) = (from..to).find(|&r| key(r) == k) {
            a.row = row;
            self.0.anchor.set(a);
        }
    }

    /// Match the heights to `rows` rows: rows added or removed at the end.
    fn sync_len(&self, rows: u64) {
        let len = self.len();
        if rows != len {
            let mut ext = self.0.extents.borrow_mut();
            if rows > len {
                ext.splice(len, 0, rows.saturating_sub(len));
            } else {
                ext.splice(rows, len.saturating_sub(rows), 0);
            }
        }
    }

    /// Move the thumb to put its top at `y` (window pixels).
    fn drag_thumb(&self, e: &DragEvent) {
        match e.phase {
            DragPhase::Start => self.0.grab.set(e.y - e.bounds.y),
            DragPhase::Move | DragPhase::End => {
                let (_, vp) = self.0.viewport.get();
                let total = pixels(self.total(&self.0.extents.borrow()));
                let thumb = thumb_height(vp, total);
                let track = (vp - thumb - 2.0 * THUMB_INSET).max(1.0);
                let top = e.y - self.0.grab.get() - self.0.top.get() - THUMB_INSET;
                self.scroll_to_fraction(f64::from((top / track).clamp(0.0, 1.0)));
            }
        }
    }
}

/// A scroll container's thumb height over `extent` pixels of content.
fn thumb_height(viewport: f32, extent: f64) -> f32 {
    if extent <= 0.0 {
        return viewport;
    }
    crate::convert::f64_to_f32(f64::from(viewport) * f64::from(viewport) / extent).max(MIN_THUMB)
}

/// The thumb's paint: the text colour at a quarter, as a scroll container's.
fn thumb_ink() -> BackgroundImage {
    let ink = Ink::Current {
        lightness_floor: 0.0,
        lightness_lift: 0.0,
        alpha_scale: 0.25,
        alpha_add: 0.0,
    };
    BackgroundImage {
        gradient: LinearGradient {
            angle: 180.0,
            stops: vec![
                Stop {
                    ink,
                    at: Calc::fraction(0.0),
                },
                Stop {
                    ink,
                    at: Calc::fraction(1.0),
                },
            ],
        },
        size: [Calc::fraction(1.0), Calc::fraction(1.0)],
        ..BackgroundImage::default()
    }
}

/// Row `i` (keyed `k`) of a list, built as `el`: when it names a row's
/// role (a table's row, an option) it is the row and carries the index;
/// when it names another (a heading among options) it is left as it is;
/// else it is wrapped in a row whose role its list's implies.
fn row_wrapper(i: u64, k: ElementKey, mut el: Element) -> Div {
    let index = usize::try_from(i).unwrap_or(usize::MAX);
    let named = el.common_mut().and_then(|c| match c.semantics.role {
        Some(role) if crate::access::is_row_role(role) => {
            c.semantics.row_index = Some(index);
            Some(())
        }
        Some(_) => Some(()),
        None => None,
    });
    let d = div().key(k);
    let d = if named.is_some() { d } else { d.aria_row_index(index) };
    d.child(el)
}

/// A list of `rows` rows, `row(i)` built for those in view and prefetched,
/// each keyed `key(i)`. Size it as any box (it fills what it is given: give
/// it a height, or [`ListState::fit_rows`]); it scrolls by itself.
pub fn virtual_list(
    state: &ListState,
    rows: u64,
    key: impl Fn(u64) -> ElementKey,
    row: impl Fn(u64) -> Element,
) -> Div {
    virtual_list_in(state, rows, key, row, |viewport| viewport)
}

/// [`virtual_list`] with its scroll container (`virtual-list-viewport`)
/// styled by `viewport`: a fade over its edge, a gap's worth of padding.
pub fn virtual_list_in(
    state: &ListState,
    rows: u64,
    key: impl Fn(u64) -> ElementKey,
    row: impl Fn(u64) -> Element,
    viewport: impl FnOnce(Div) -> Div,
) -> Div {
    // Read so a moved window renders again.
    state.0.version.with(|_| ());
    state.sync_len(rows);
    state.follow_key(&key);
    let w = state.compute();
    state.0.window.set(w);
    let keys: Vec<ElementKey> = (w.first..w.last).map(&key).collect();
    // At the end, as far as the rows reach once measured (the container
    // clamps it).
    let local = if state.wants_end() { f32::MAX } else { w.local };
    state.0.scroll.scroll_to((0.0, local));
    let row_at = |i: u64, k: ElementKey| row_wrapper(i, k, row(i)).flex_none().w_full().into_element();
    let pins = state.pins_for(w, rows, &key);
    let before = pins.iter().take_while(|(r, _)| *r < w.first).count();
    // Kept built where nothing shows them: their bottom on the viewport's
    // top, out of the scroll range.
    let pinned = |&(i, k): &(u64, ElementKey)| {
        row_wrapper(i, k, row(i))
            .absolute()
            .left(0.0)
            .w_full()
            .bottom_len(Length::Fraction(1.0))
            .into_element()
    };
    let pins_before: Vec<Element> = pins.iter().take(before).map(pinned).collect();
    let pins_after: Vec<Element> = pins.iter().skip(before).map(pinned).collect();
    state.0.pins_before.set(before);
    *state.0.pins.borrow_mut() = pins;
    let built = pins_before
        .into_iter()
        .chain((w.first..w.last).zip(&keys).map(|(i, &k)| row_at(i, k)))
        .chain(pins_after);
    let end_space = state.0.end_space.get();
    let room = (w.last >= rows && end_space > 0.0).then(|| div().named("virtual-list-end").flex_none().h_px(end_space));
    let scroller = viewport(
        div()
            .named("virtual-list-viewport")
            .absolute()
            .inset(0.0)
            .flex_col()
            .overflow_y_scroll()
            .scrollbar_none()
            .track_scroll(&state.0.scroll),
    )
    .children(built.collect::<Vec<_>>())
    .child(room);
    *state.0.keys.borrow_mut() = keys;
    // The thumb's place and size are bound to the layout (ADR-0013): set
    // after each layout from the anchor over the whole extent, they move the
    // thumb without building the list again.
    let geometry = state.0.thumb;
    let s = state.clone();
    let thumb = (!state.0.no_scrollbar.get()).then(|| {
        div()
            .named("virtual-list-thumb")
            .absolute()
            .right(THUMB_INSET)
            .w_px(THUMB_WIDTH)
            .rounded(THUMB_WIDTH / 2.0)
            .background_image(thumb_ink())
            .bind(move |st| match geometry.get() {
                Some((y, h)) => st.top_len(px(y)).h_px(h),
                None => st.hidden(),
            })
            .on_drag(move |e| s.drag_thumb(e))
    });
    let list = div()
        .named("virtual-list")
        .relative()
        .overflow_hidden()
        .role(Role::List)
        .aria_row_count(usize::try_from(rows).unwrap_or(usize::MAX))
        .child(scroller)
        .child(thumb);
    match state.0.fit.get() {
        Some(max) => {
            // Its rows' height so far; measured after layout, bound there.
            let rows_h = crate::convert::f64_to_f32(pixels(state.total(&state.0.extents.borrow()))).min(max);
            if state.0.height.get_untracked() != rows_h {
                state.0.height.set(rows_h);
            }
            let height = state.0.height;
            list.flex_none().bind(move |st| st.h_px(height.get()))
        }
        None => list,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Scenario;
    use crate::element::text;
    use crate::layout::Content;

    const ROWS: u64 = 10_000_000;

    fn list(state: &ListState) -> Element {
        div()
            .w_px(300.0)
            .child(
                virtual_list(
                    state,
                    ROWS,
                    |i| i,
                    |i| {
                        div()
                            .h_px(if i % 3 == 0 { 40.0 } else { 24.0 })
                            .child(text(format!("row {i}")))
                            .into_element()
                    },
                )
                .h_px(400.0),
            )
            .into_element()
    }

    /// The rows laid out: their number and their box's top and bottom.
    fn rows(s: &Scenario) -> Vec<(u64, f32, f32)> {
        s.last()
            .nodes
            .iter()
            .filter_map(|n| match &n.content {
                Content::Text { content, .. } => {
                    let row = content.strip_prefix("row ")?.parse().ok()?;
                    Some((row, n.bounds.y, n.bounds.y + n.bounds.height))
                }
                _ => None,
            })
            .collect()
    }

    /// The row at the viewport's top (y = 0).
    fn top(s: &Scenario) -> u64 {
        rows(s)
            .into_iter()
            .find(|&(_, y, bottom)| y <= 0.0 && bottom > 0.0)
            .map_or(u64::MAX, |r| r.0)
    }

    fn scenario(state: &ListState) -> Scenario {
        let st = state.clone();
        let mut s = Scenario::measuring(move || list(&st), 300, 400, 1.0).unwrap();
        s.settle().unwrap();
        s
    }

    #[test]
    fn ten_million_rows_build_only_the_window() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        let built = rows(&s);
        assert!(built.len() < 80, "{} rows built", built.len());
        assert_eq!(top(&s), 0);
        assert!(
            state.extent() > f64::from(1_u32 << 24),
            "the extent is past f32's exact range"
        );
        // Measured rows are exact: row 3 lies under rows 0..3 (40 + 24 + 24).
        let r3 = built.iter().find(|r| r.0 == 3).unwrap();
        assert_eq!(r3.1, 88.0);
        // The wheel moves through the window and the window follows.
        s.hover(100.0, 200.0);
        for _ in 0..30 {
            s.wheel(100.0);
            s.settle().unwrap();
        }
        let a = state.anchor();
        assert!(a.row > 50, "{a:?}");
        assert_eq!(top(&s), a.row);
        assert!(rows(&s).len() < 80);
    }

    #[test]
    fn the_far_end_and_the_middle_land_where_asked_and_stay() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        s.act("to the middle", || state.scroll_to_fraction(0.5));
        s.settle().unwrap();
        let landed = top(&s);
        assert!(landed.abs_diff(ROWS / 2) < ROWS / 50, "landed on {landed}");
        // Rows measured after landing do not move what is in view.
        for _ in 0..3 {
            s.settle().unwrap();
            assert!(top(&s).abs_diff(landed) <= 1, "{} after {landed}", top(&s));
        }
        s.act("to the end", || state.scroll_to_end());
        s.settle().unwrap();
        let last = rows(&s)
            .into_iter()
            .find(|r| r.0 == ROWS - 1)
            .expect("the last row is built");
        // Its box (40 px tall, row 9 999 999 being a multiple of 3) ends at the viewport's end.
        assert!((last.1 + 40.0 - 400.0).abs() < 0.5, "the last row starts at {}", last.1);
    }

    #[test]
    fn a_followed_tail_stays_in_view_as_rows_arrive() {
        let state = ListState::new(30.0);
        state.follow_tail(true);
        let n = Signal::new(50_u64);
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .w_px(300.0)
                    .child(virtual_list(&st, n.get(), |i| i, |i| text(format!("row {i}")).into_element()).h_px(400.0))
                    .into_element()
            },
            300,
            400,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        assert!(rows(&s).iter().any(|r| r.0 == 49 && r.2 <= 400.5 && r.2 > 390.0));
        s.act("more", || n.set(80));
        s.settle().unwrap();
        assert!(rows(&s).iter().any(|r| r.0 == 79 && r.2 <= 400.5 && r.2 > 390.0));
        assert!(state.is_at_end());
    }

    /// The top of `row`'s box, if built.
    fn top_of(s: &Scenario, row: u64) -> Option<f32> {
        rows(s).into_iter().find(|r| r.0 == row).map(|r| r.1)
    }

    #[test]
    fn a_row_asked_for_lands_exactly_though_its_neighbours_were_estimates() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        // Below the viewport's top by 50 px: the rows above it in view are
        // estimates (30 px) until laid out, then 24 or 40.
        s.act("to a far row", || {
            state.scroll_to(Anchor {
                row: 5_000_001,
                key: None,
                offset: -50.0,
            });
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, 5_000_001), Some(50.0));
        // It stays there as more frames measure more rows.
        s.settle().unwrap();
        assert_eq!(top_of(&s, 5_000_001), Some(50.0));
    }

    #[test]
    fn scroll_into_view_moves_only_as_far_as_shows_the_row() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        // Row 2 shows already: nothing moves.
        s.act("row 2", || state.scroll_into_view(2));
        s.settle().unwrap();
        assert_eq!(top(&s), 0);
        // A row far below comes in at the bottom edge.
        s.act("row 1000", || state.scroll_into_view(1000));
        s.settle().unwrap();
        let r = rows(&s).into_iter().find(|r| r.0 == 1000).unwrap();
        assert!((r.2 - 400.0).abs() < 0.01, "{r:?}");
        // One above comes in at the top edge.
        s.act("row 900", || state.scroll_into_view(900));
        s.settle().unwrap();
        assert_eq!(top_of(&s, 900), Some(0.0));
    }

    #[test]
    fn a_list_fitted_to_its_rows_is_as_tall_as_they_are_up_to_its_limit() {
        let state = ListState::new(30.0);
        state.fit_rows(Some(300.0));
        let n = Signal::new(4_u64);
        let mut s = Scenario::measuring(
            move || {
                div()
                    .w_px(300.0)
                    .flex_col()
                    .child(virtual_list(
                        &state,
                        n.get(),
                        |i| i,
                        |i| div().h_px(25.0).child(text(format!("row {i}"))).into_element(),
                    ))
                    .child(text("below"))
                    .into_element()
            },
            300,
            400,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        let below = |s: &Scenario| {
            s.last()
                .nodes
                .iter()
                .find(|n| matches!(&n.content, Content::Text { content, .. } if content.as_str() == "below"))
                .map(|n| n.bounds.y)
        };
        assert_eq!(below(&s), Some(100.0));
        s.act("more", || n.set(1000));
        s.settle().unwrap();
        assert_eq!(below(&s), Some(300.0));
        assert!(rows(&s).len() < 40);
    }

    #[test]
    fn room_below_the_last_row_lets_it_scroll_to_the_top() {
        let state = ListState::new(30.0);
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .w_px(300.0)
                    .child(
                        virtual_list(
                            &st,
                            30,
                            |i| i,
                            |i| div().h_px(25.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .h_px(400.0),
                    )
                    .into_element()
            },
            300,
            400,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.act("room and the last row", || {
            state.set_end_space(375.0);
            state.scroll_to_row(29);
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, 29), Some(0.0));
    }

    #[test]
    fn a_measured_row_adjusted_keeps_its_new_height_until_laid_out_again() {
        let state = ListState::new(30.0);
        let _s = scenario(&state);
        assert!(state.is_measured(3));
        let (h, extent) = (state.row_height(3), state.extent());
        state.adjust_row_height(3, 32.0);
        assert_eq!(state.row_height(3), h + 32.0);
        assert_eq!(state.extent(), extent + 32.0);
        state.adjust_row_height(3, -32.0);
        assert_eq!(state.row_height(3), h);
        // A row not measured keeps its estimate.
        let far = ROWS - 10;
        let estimate = state.row_height(far);
        state.adjust_row_height(far, 32.0);
        assert_eq!(state.row_height(far), estimate);
    }

    #[test]
    fn scrolling_back_and_forth_shapes_nothing_again() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        s.hover(100.0, 200.0);
        let mut shaped = Vec::new();
        for _ in 0..8 {
            for dy in [150.0, -150.0] {
                s.wheel(dy);
                shaped.push(s.frame("wheel").unwrap().shaped);
                s.settle().unwrap();
            }
        }
        // Rows leaving the viewport stay prefetched, their text kept shaped.
        assert_eq!(shaped.iter().skip(4).sum::<u64>(), 0, "{shaped:?}");
    }
}
