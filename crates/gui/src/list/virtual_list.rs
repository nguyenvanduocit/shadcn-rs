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
use crate::prelude::*;
use crate::reactive::{Signal, detached};
use crate::scroll::{ScrollHandle, ScrollLayout};
use crate::style::{BackgroundImage, Calc, LinearGradient, Stop, StopColor};

/// The viewport height a list assumes before its first layout.
const FIRST_VIEWPORT: f32 = 600.0;
/// Fewest pixels the thumb is tall (a scroll container's).
const MIN_THUMB: f32 = 20.0;
/// The thumb's inset from the track's ends.
const THUMB_INSET: f32 = 2.0;
/// The thumb's inset from the viewport's right edge.
const THUMB_RIGHT: f32 = 8.0;
const THUMB_WIDTH: f32 = 6.0;
/// How long the thumb stays shown after the reader last scrolled or
/// dragged, then how long it fades (an overlay scrollbar, as the
/// compositor shows a scroll container's), milliseconds.
const THUMB_SHOWN_MS: u64 = 500;
const THUMB_FADE_MS: f32 = 200.0;
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

/// What a render built the rows from: worked out from the list's state and
/// the data the render was given, and handed with the scroll container to
/// the layout hook, which takes it in (a render writes nothing, ADR-0013;
/// a render the frame does not lay out, as the cold check's, leaves the
/// state as it was).
/// A scroll asked for, held until the viewport shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Request {
    /// Which request it is: a layout clears only the one it saw.
    rev: u64,
    /// It holds the anchored row's bottom this many pixels above the
    /// viewport's, whatever height the row is measured at; `None`: the
    /// anchor's own offset from its top.
    bottom: Option<f32>,
}

#[derive(Debug)]
struct Plan {
    /// The rows the data holds.
    rows: u64,
    /// The anchor moved to its key's row, the data having moved under it;
    /// `None`: it stays.
    followed: Option<Anchor>,
    window: Window,
    /// The keys of the window's rows, in order.
    keys: Vec<ElementKey>,
    /// The rows built outside the window, in order, and how many come before it.
    pins: Vec<(u64, ElementKey)>,
    pins_before: usize,
    /// The row holding focus, as the render found it by its key.
    focus: Option<(u64, ElementKey)>,
    /// The rows' extent and the end space, in units, over `rows` rows.
    total: u64,
    /// The scroll request revision it was built against: a request made
    /// since (an effect that ran after the render) wins over its anchor.
    rev: u64,
}

/// Heights `ext` matched to `rows` rows: rows added or removed at the end.
fn resize(ext: &mut ExtentTree, rows: u64) {
    let len = ext.len();
    if rows > len {
        ext.splice(len, 0, rows.saturating_sub(len));
    } else if rows < len {
        ext.splice(rows, len.saturating_sub(rows), 0);
    }
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
/// What tells the row its owner treats as current ([`ListState::active_from`]).
type ActiveFrom = Rc<dyn Fn() -> Option<u64>>;
/// A row and its key.
type KeyedRow = (u64, ElementKey);

struct Inner {
    /// Bumped when the window moves: the list renders again.
    version: Signal<u64>,
    extents: RefCell<ExtentTree>,
    anchor: Cell<Anchor>,
    /// The scroll asked for, the viewport not showing it yet: the anchor is
    /// held through the layouts that measure the rows around it. One slot:
    /// each request replaces the one before, whole, so none comes back.
    request: Cell<Option<Request>>,
    /// The revision the next request takes.
    requests: Cell<u64>,
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
    /// Its height when sized by its rows, as last laid out; `None` before.
    height: Signal<Option<f32>>,
    /// The thumb is not drawn (`scrollbar-width: none`): read as the list
    /// renders, so a scroller that hides it as it scrolls itself (an
    /// effect on its own state) builds the list again.
    no_scrollbar: Signal<bool>,
    /// Where on the thumb a drag took hold, from its top.
    grab: Cell<f32>,
    /// The thumb's top and height in the viewport; `None` when all rows fit.
    thumb: Signal<Option<(f32, f32)>>,
    /// The thumb shows: the reader scrolled or dragged it lately. At rest
    /// it is hidden, an overlay scrollbar.
    thumb_shown: Signal<bool>,
    /// The scroll handle's [`ScrollHandle::wheels`] as last seen, and the
    /// timer hiding the thumb again (only the latest one hides it).
    wheels_seen: Cell<u64>,
    hide_run: Cell<u64>,
    /// The thumb is held: it stays shown until let go.
    dragging: Cell<bool>,
    /// The owner's hook, run once the window is settled.
    hook: RefCell<Option<Hook>>,
    /// The row holding focus, as last laid out, and its key.
    focus_row: Cell<Option<(u64, ElementKey)>>,
    /// Where the scroll container sits, as last laid out: its rows are
    /// its children.
    viewport_path: RefCell<Option<crate::geometry::NodePath>>,
    /// The row its owner treats as current (an active option).
    active: Cell<Option<u64>>,
    /// Or what tells it, read as the list renders ([`ListState::active_from`]).
    /// Held by the owner's instance while it is mounted: a list state that
    /// outlives it (reused by another list) never calls it once it left.
    active_from: crate::Holder<ActiveFrom>,
    /// Rows on each side of the focused one kept built with it.
    around_focus: Cell<u64>,
    /// The rows built outside the window (pinned), in order, and how many
    /// of them come before it.
    pins: RefCell<Vec<(u64, ElementKey)>>,
    pins_before: Cell<usize>,
    /// The last render's plan, taken in: taken once.
    taken: RefCell<Weak<dyn std::any::Any>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.version.dispose_quietly();
        self.thumb.dispose_quietly();
        self.thumb_shown.dispose_quietly();
        self.height.dispose_quietly();
        self.no_scrollbar.dispose_quietly();
        self.active_from.dispose_quietly();
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
            request: Cell::new(None),
            requests: Cell::new(0),
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
            height: detached(|| Signal::labeled("virtual list height", None)),
            no_scrollbar: detached(|| Signal::labeled("virtual list no scrollbar", false)),
            grab: Cell::new(0.0),
            thumb: detached(|| Signal::labeled("virtual list thumb", None)),
            thumb_shown: detached(|| Signal::labeled("virtual list thumb shown", false)),
            wheels_seen: Cell::new(0),
            hide_run: Cell::new(0),
            dragging: Cell::new(false),
            hook: RefCell::new(None),
            focus_row: Cell::new(None),
            active: Cell::new(None),
            active_from: detached(crate::Holder::new),
            around_focus: Cell::new(0),
            pins: RefCell::default(),
            viewport_path: RefCell::default(),
            pins_before: Cell::new(0),
            taken: RefCell::new(Weak::<Plan>::new()),
        });
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        inner
            .scroll
            .on_layout_built(move |l, built| weak.upgrade().is_some_and(|i| Self(i).laid_out(l, built)));
        let weak: Weak<Inner> = Rc::downgrade(&inner);
        inner
            .scroll
            .on_built(move |built| weak.upgrade().is_some_and(|i| Self(i).take(built)));
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
        self.moved();
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
        self.moved();
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
            self.moved();
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

    /// The row its owner treats as current is what `f` gives, read (tracked)
    /// as the list renders: an owner whose highlight lives in its own state
    /// says so once, in its setup, rather than setting it as it renders
    /// (where a list built before the call would read the last one).
    /// It stands in for [`Self::set_active`] while the calling setup's
    /// instance is mounted (from its mount, [`crate::while_mounted`]): its
    /// unmounting lets go of `f` unless another owner gave one since.
    #[track_caller]
    pub fn active_from(&self, f: impl Fn() -> Option<u64> + 'static) {
        let f: ActiveFrom = Rc::new(f);
        self.0.active_from.hold(f);
    }

    /// Keep `rows` rows on each side of the focused one built with it (0 by
    /// default), so Tab and Shift+Tab from a row scrolled away reach its
    /// neighbours, not the rows in view. Read when the list renders.
    pub fn keep_around_focus(&self, rows: u64) {
        self.0.around_focus.set(rows);
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

    /// The row holding focus now, if built in the last frame or in window
    /// `w` (its rows keyed `keys`): found as the list renders, not at the
    /// next layout, so a window that jumps before then (a scroll asked in
    /// the same turn) keeps it built.
    fn noted_focus(&self, w: Window, keys: &[ElementKey]) -> Option<(u64, ElementKey)> {
        let path = crate::untracked(crate::focus::focused).and_then(crate::focus::path_of)?;
        // A row is a child of this list's scroll container: the segment
        // past the container's in the focused node's path, and no other.
        let seg = self.0.viewport_path.borrow().as_ref().and_then(|v| {
            path.0
                .starts_with(&v.0)
                .then(|| path.0.get(v.0.len()).copied())
                .flatten()
        });
        // A row the list keyed: as a node, or as a component's root.
        let Some(crate::geometry::PathSeg::Key(k) | crate::geometry::PathSeg::Component(k, _)) = seg else {
            return None;
        };
        // Built in the last frame: in its window or pinned.
        let last = self.0.window.get();
        let in_window = self
            .0
            .keys
            .borrow()
            .iter()
            .position(|key| *key == k)
            .map(|j| last.first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)));
        let row = in_window
            .or_else(|| self.0.pins.borrow().iter().find(|(_, key)| *key == k).map(|(r, _)| *r))
            // Or in the window this render builds.
            .or_else(|| {
                keys.iter()
                    .position(|key| *key == k)
                    .map(|j| w.first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)))
            });
        row.map(|r| (r, k))
    }

    /// The pinned rows for this render: focus (and the rows kept around it)
    /// and the active row, outside the window `w`, found again by key after
    /// the data moved.
    fn pins_for(
        &self,
        focus: Option<(u64, ElementKey)>,
        w: Window,
        rows: u64,
        key: &dyn Fn(u64) -> ElementKey,
    ) -> (Option<KeyedRow>, Vec<KeyedRow>) {
        let focus = focus.and_then(|(row, k)| {
            if row < rows && key(row) == k {
                return Some((row, k));
            }
            let from = row.saturating_sub(KEY_SEARCH);
            let to = row.saturating_add(KEY_SEARCH).min(rows);
            (from..to).find(|&r| key(r) == k).map(|r| (r, k))
        });
        let from = self.0.active_from.get();
        let active = from
            .map_or_else(|| self.0.active.get(), |f| f())
            .filter(|&r| r < rows)
            .map(|r| (r, key(r)));
        let around = self.0.around_focus.get();
        let neighbours = focus.into_iter().flat_map(|(row, _)| {
            (row.saturating_sub(around)..row.saturating_add(around).saturating_add(1).min(rows))
                .filter(move |&r| r != row)
                .map(|r| (r, key(r)))
        });
        let mut pins: Vec<(u64, ElementKey)> = focus
            .into_iter()
            .chain(neighbours)
            .chain(active)
            .filter(|(r, _)| !(w.first..w.last).contains(r))
            .collect();
        pins.sort_unstable_by_key(|(r, _)| *r);
        pins.dedup_by_key(|(r, _)| *r);
        (focus, pins)
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
        self.ask(None);
        self.0.anchor.set(Anchor {
            row: anchor.row.min(self.len().saturating_sub(1)),
            ..anchor
        });
        self.moved();
    }

    /// Scroll so `row`'s bottom lies `above` pixels above the viewport's,
    /// at whatever height the row is measured.
    pub fn scroll_to_row_bottom(&self, row: u64, above: f32) {
        self.scroll_to(Anchor {
            row,
            key: None,
            offset: 0.0,
        });
        self.ask(Some(above));
        self.moved();
    }

    /// Ask for a scroll, in place of any asked before.
    fn ask(&self, bottom: Option<f32>) {
        let rev = self.0.requests.get().wrapping_add(1);
        self.0.requests.set(rev);
        self.0.request.set(Some(Request { rev, bottom }));
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
        self.0.request.set(None);
        self.0.pinned.set(true);
        self.0.to_end.set(true);
        self.moved();
    }

    /// Scroll so a `fraction` of the whole extent lies above the viewport.
    pub fn scroll_to_fraction(&self, fraction: f64) {
        let ext = self.0.extents.borrow();
        let vp = u64::from(units(self.viewport_height()));
        let room = self.total(&ext).saturating_sub(vp);
        let top = crate::convert::f64_to_u64(fraction.clamp(0.0, 1.0) * crate::convert::u64_to_f64(room));
        let (row, into) = ext.row_at(top);
        drop(ext);
        self.0.request.set(None);
        self.0.pinned.set(fraction >= 1.0 && self.0.follow.get());
        self.0.to_end.set(fraction >= 1.0);
        self.0.anchor.set(Anchor {
            row,
            key: None,
            offset: crate::convert::f64_to_f32(pixels(into)),
        });
        self.moved();
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
        self.moved();
    }

    /// The rows were reordered or filtered (a sort, a new
    /// [`View`](super::View)): it holds `rows` rows now, and the row at each
    /// old position `p` is at `to(p)` (`None`: left out;
    /// [`View::moved_to`](super::View::moved_to) gives it). The heights
    /// measured follow their rows, the rows new to it take the estimate,
    /// and the anchor and the focused row follow theirs (a row left out
    /// leaves the anchor at its old index, its top). O(m log m) for the
    /// `m` rows measured, whatever the rows.
    pub fn reorder(&self, rows: u64, to: impl Fn(u64) -> Option<u64>) {
        self.0.extents.borrow_mut().reorder(rows, &to);
        let a = self.0.anchor.get();
        let anchor = match to(a.row).filter(|&r| r < rows) {
            Some(row) => Anchor { row, ..a },
            None => Anchor {
                row: a.row.min(rows.saturating_sub(1)),
                key: None,
                offset: 0.0,
            },
        };
        self.0.anchor.set(anchor);
        let focus = self.0.focus_row.get();
        self.0
            .focus_row
            .set(focus.and_then(|(r, k)| to(r).filter(|&r| r < rows).map(|r| (r, k))));
        // The rows built, and those pinned, were found by position: the
        // window is decided anew.
        self.0.window.set(Window::default());
        self.0.keys.borrow_mut().clear();
        self.0.pins.borrow_mut().clear();
        self.0.pins_before.set(0);
        self.moved();
    }

    fn bump(&self) {
        self.0.version.update(|v| *v = v.wrapping_add(1));
    }

    /// Where the view is to be changed (a scroll asked for, rows moved
    /// under it): the scroll container is put where the window the next
    /// render builds has it, and the list renders again.
    fn moved(&self) {
        self.place(self.compute());
        self.bump();
    }

    /// Put the scroll container where window `w` shows the viewport (at
    /// the end, as far as the rows reach once measured: the container
    /// clamps it).
    fn place(&self, w: Window) {
        let local = if self.wants_end() { f32::MAX } else { w.local };
        self.0.scroll.keep_at((0.0, local));
    }

    /// The rows' extent and the end space, in units.
    fn total(&self, ext: &ExtentTree) -> u64 {
        ext.extent().saturating_add(u64::from(units(self.0.end_space.get())))
    }

    /// The viewport's top in units as the anchor puts it, within the extent.
    fn top_units(&self, ext: &ExtentTree) -> u64 {
        self.top_units_at(ext, self.0.anchor.get())
    }

    /// [`Self::top_units`] with the anchor at `a`.
    fn top_units_at(&self, ext: &ExtentTree, a: Anchor) -> u64 {
        let len = ext.len();
        if len == 0 {
            return 0;
        }
        let vp = u64::from(units(self.viewport_height()));
        let room = self.total(ext).saturating_sub(vp);
        if self.wants_end() {
            return room;
        }
        let row = a.row.min(len.saturating_sub(1));
        let base = crate::convert::u64_to_f64(ext.offset_of(row));
        let offset = match self.0.request.get().and_then(|r| r.bottom) {
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
        self.compute_at(&self.0.extents.borrow(), self.0.anchor.get())
    }

    /// [`Self::compute`] over heights `ext`, the anchor at `anchor`.
    fn compute_at(&self, ext: &ExtentTree, anchor: Anchor) -> Window {
        let len = ext.len();
        if len == 0 {
            return Window::default();
        }
        let vp_px = self.viewport_height();
        let vp = u64::from(units(vp_px));
        let rows_end = ext.extent();
        let top = self.top_units_at(ext, anchor);
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
    fn laid_out(&self, l: &ScrollLayout, built: Option<&crate::scroll::Built>) -> bool {
        let inner = &self.0;
        if let Some(b) = built {
            let _moved = self.take(b);
        }
        let width = l.bounds.width;
        let height = (l.bounds.height - l.padding.top - l.padding.bottom).max(0.0);
        let (old_width, _) = inner.viewport.get();
        inner.viewport.set((width, height));
        if inner.viewport_path.borrow().as_ref() != Some(&l.path) {
            inner.viewport_path.replace(Some(l.path.clone()));
        }
        inner.top.set(l.bounds.y);
        if old_width > 0.0 && old_width != width {
            // Heights measured at another width: estimates again.
            inner.extents.borrow_mut().reset();
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
        let held = inner.focus_row.get();
        let unpinned = held.is_some_and(|(r, _)| !(w.first..w.last).contains(&r)) && focus != held;
        inner.focus_row.set(focus);
        if unpinned {
            // Focus left the row kept built for it: it goes.
            self.bump();
            return true;
        }
        {
            // The rows not built take the mean of those measured, which the
            // heights keep themselves ([`ExtentTree::estimate`]).
            let mut ext = inner.extents.borrow_mut();
            for (row, c) in (w.first..w.last).zip(children) {
                ext.set(row, units(c.bounds.height));
            }
        }
        // The anchor: the row at the viewport's top, unless one asked for is
        // not shown yet.
        let y = l.offset.1 + l.padding.top;
        let asked = inner.request.get();
        if asked.is_none() {
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
        let wheels = inner.scroll.wheels();
        if wheels != inner.wheels_seen.replace(wheels) {
            self.show_thumb();
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
            // The next render builds this window, put where it shows the
            // viewport.
            inner.window.set(next);
            self.place(next);
            self.bump();
            return true;
        }
        // The rows built still cover it: nothing to build.
        inner.window.set(next);
        if inner.request.get().map(|r| r.rev) == asked.map(|r| r.rev) {
            inner.request.set(None);
        }
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
        if self.0.height.get_untracked() == Some(h) {
            return false;
        }
        self.0.height.set(Some(h));
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

    /// The anchor kept on its key after the data moved under it, over
    /// `len` rows; `None` when it stays where it is.
    fn followed(&self, len: u64, key: &dyn Fn(u64) -> ElementKey) -> Option<Anchor> {
        let a = self.0.anchor.get();
        let k = a.key?;
        if a.row < len && key(a.row) == k {
            return None;
        }
        let from = a.row.saturating_sub(KEY_SEARCH);
        let to = a.row.saturating_add(KEY_SEARCH).min(len);
        (from..to).find(|&r| key(r) == k).map(|row| Anchor { row, ..a })
    }

    /// Match the heights to `rows` rows: rows added or removed at the end.
    fn sync_len(&self, rows: u64) {
        resize(&mut self.0.extents.borrow_mut(), rows);
    }

    /// What a render builds from the state as it stands and the data it is
    /// given, `rows` rows keyed `key`: worked out, nothing written (the
    /// layout hook takes it in, [`Self::take`]).
    fn plan(&self, rows: u64, key: &dyn Fn(u64) -> ElementKey) -> Plan {
        let held = self.0.extents.borrow();
        // The heights as the data has them: rows added or removed at the end.
        let resized = (held.len() != rows).then(|| {
            let mut ext = held.clone();
            resize(&mut ext, rows);
            ext
        });
        let ext = resized.as_ref().unwrap_or(&held);
        let followed = self.followed(rows, key);
        let window = self.compute_at(ext, followed.unwrap_or_else(|| self.0.anchor.get()));
        let total = self.total(ext);
        drop(held);
        let keys: Vec<ElementKey> = (window.first..window.last).map(key).collect();
        let focus = self.noted_focus(window, &keys).or_else(|| self.0.focus_row.get());
        let (focus, pins) = self.pins_for(focus, window, rows, key);
        let pins_before = pins.iter().take_while(|(r, _)| *r < window.first).count();
        Plan {
            rows,
            followed,
            window,
            keys,
            pins,
            pins_before,
            focus,
            total,
            rev: self.0.requests.get(),
        }
    }

    /// Take in what the render built from, once (the plan is known by its
    /// own allocation, which the `Weak` held keeps from being reused, so a
    /// plan taken again changes nothing): the heights matched to its data,
    /// the anchor kept on its key, the rows it built. Taken after every
    /// layout pass that laid them out, the frame's last too. Whether it put
    /// the container elsewhere.
    fn take(&self, built: &crate::scroll::Built) -> bool {
        let inner = &self.0;
        if Weak::ptr_eq(&inner.taken.borrow(), &Rc::downgrade(built)) {
            return false;
        }
        *inner.taken.borrow_mut() = Rc::downgrade(built);
        let Some(p) = built.downcast_ref::<Plan>() else {
            return false;
        };
        let was = inner.scroll.offset();
        self.sync_len(p.rows);
        // A scroll asked for since the render (a later effect's) is newer
        // than the anchor the plan followed: it wins, and the list renders
        // again for it.
        let current = inner.requests.get() == p.rev;
        if let Some(a) = p.followed.filter(|_| current) {
            // The reader's row moved to another index under them: the
            // container goes where the plan put that row, and the anchor
            // holds until the viewport shows it there (the hook reads no
            // other row from a layout at the old offset).
            inner.anchor.set(a);
            self.ask(None);
            self.place(p.window);
        }
        inner.window.set(p.window);
        inner.keys.borrow_mut().clone_from(&p.keys);
        inner.pins.borrow_mut().clone_from(&p.pins);
        inner.pins_before.set(p.pins_before);
        inner.focus_row.set(p.focus);
        inner.scroll.offset() != was
    }

    /// Show the thumb, and hide it [`THUMB_SHOWN_MS`] after the last call.
    fn show_thumb(&self) {
        let inner = &self.0;
        if !inner.thumb_shown.get_untracked() {
            inner.thumb_shown.set(true);
        }
        let run = inner.hide_run.get().wrapping_add(1);
        inner.hide_run.set(run);
        let weak = Rc::downgrade(&self.0);
        crate::clock::after(std::time::Duration::from_millis(THUMB_SHOWN_MS), move || {
            if let Some(inner) = weak.upgrade()
                && inner.hide_run.get() == run
                && !inner.dragging.get()
            {
                inner.thumb_shown.set(false);
            }
        });
    }

    /// Move the thumb to put its top at `y` (window pixels).
    fn drag_thumb(&self, e: &DragEvent) {
        self.0.dragging.set(!e.phase.is_last());
        self.show_thumb();
        match e.phase {
            DragPhase::Start => self.0.grab.set(e.y - e.bounds.y),
            // Cancelled, the list stays where the moves put it.
            DragPhase::Cancel => {}
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
    let ink = StopColor::Current {
        lightness_floor: 0.0,
        lightness_lift: 0.0,
        alpha_scale: 0.25,
        alpha_add: 0.0,
    };
    LinearGradient {
        angle: 180.0,
        stops: [
            Stop {
                color: ink,
                at: Calc::fraction(0.0),
            },
            Stop {
                color: ink,
                at: Calc::fraction(1.0),
            },
        ]
        .into(),
    }
    .into()
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
    let plan = state.plan(rows, &key);
    let w = plan.window;
    let total = plan.total;
    let row_at = |i: u64, k: ElementKey| {
        row_wrapper(i, k, row(i))
            .flex(Flex::NONE)
            .width(pct(100.0))
            .into_element()
    };
    let before = plan.pins_before;
    // Kept built where nothing shows them: their bottom on the viewport's
    // top, out of the scroll range.
    let pinned = |&(i, k): &(u64, ElementKey)| {
        row_wrapper(i, k, row(i))
            .position(Position::Absolute)
            .left(0.0)
            .width(pct(100.0))
            .bottom(Length::Fraction(1.0))
            .into_element()
    };
    let pins_before: Vec<Element> = plan.pins.iter().take(before).map(pinned).collect();
    let pins_after: Vec<Element> = plan.pins.iter().skip(before).map(pinned).collect();
    let built = pins_before
        .into_iter()
        .chain((w.first..w.last).zip(&plan.keys).map(|(i, &k)| row_at(i, k)))
        .chain(pins_after);
    let end_space = state.0.end_space.get();
    let room =
        (w.last >= rows && end_space > 0.0).then(|| div().named("virtual-list-end").flex(Flex::NONE).height(end_space));
    let scroller = viewport(
        div()
            .named("virtual-list-viewport")
            .position(Position::Absolute)
            .inset(0.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden)
            .scrollbar_hidden(true),
    )
    .children(built.collect::<Vec<_>>())
    .child(room);
    let scroller = scroller.track_scroll_built(&state.0.scroll, Rc::new(plan));
    // The thumb's place and size are bound to the layout (ADR-0013): set
    // after each layout from the anchor over the whole extent, they move the
    // thumb without building the list again.
    let geometry = state.0.thumb;
    let shown = state.0.thumb_shown;
    let s = state.clone();
    let thumb = (!state.0.no_scrollbar.get()).then(|| {
        div()
            .named("virtual-list-thumb")
            .position(Position::Absolute)
            .right(THUMB_RIGHT)
            .width(THUMB_WIDTH)
            .radius(THUMB_WIDTH / 2.0)
            .background_image(thumb_ink())
            .top(move || geometry.get().map_or(0.0, |(y, _)| y))
            .height(move || geometry.get().map_or(0.0, |(_, h)| h))
            // Shown at once, faded out.
            .opacity(move || if shown.get() { 1.0 } else { 0.0 })
            .duration_ms(move || if shown.get() { 0.0 } else { THUMB_FADE_MS })
            .ease(Easing::Linear)
            .transition(crate::motion::Properties::OPACITY)
            .display(move || {
                if geometry.get().is_some() {
                    Display::Block
                } else {
                    Display::None
                }
            })
            .on_drag(move |e| s.drag_thumb(e))
    });
    let list = div()
        .named("virtual-list")
        .position(Position::Relative)
        .overflow(Overflow::Hidden)
        .role(Role::List)
        .aria_row_count(usize::try_from(rows).unwrap_or(usize::MAX))
        .child(scroller)
        .child(thumb);
    match state.0.fit.get() {
        Some(max) => {
            // Its rows' height so far; measured after layout, bound there.
            // Before its first layout, from the heights it starts with.
            let rows_h = crate::convert::f64_to_f32(pixels(total)).min(max);
            let height = state.0.height;
            list.flex(Flex::NONE).height(move || height.get().unwrap_or(rows_h))
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
            .width(300.0)
            .child(
                virtual_list(
                    state,
                    ROWS,
                    |i| i,
                    |i| {
                        div()
                            .height(if i % 3 == 0 { 40.0 } else { 24.0 })
                            .child(text(format!("row {i}")))
                            .into_element()
                    },
                )
                .height(400.0),
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

    /// Focus moved to a row and a scroll far away asked in the same turn:
    /// the row is kept built from when focus moved, so focus stays on it.
    #[test]
    fn a_row_focused_then_scrolled_away_at_once_keeps_focus() {
        let state = ListState::new(30.0);
        let h = crate::FocusHandle::new();
        let st = state.clone();
        let ui = move || {
            div()
                .width(300.0)
                .child(
                    virtual_list(
                        &st,
                        ROWS,
                        |i| i,
                        move |i| {
                            let d = div().height(24.0).child(text(format!("row {i}")));
                            let d = if i == 3 { d.track_focus(h) } else { d };
                            d.into_element()
                        },
                    )
                    .height(400.0),
                )
                .into_element()
        };
        let mut s = Scenario::measuring(ui, 300, 400, 1.0).unwrap();
        s.settle().unwrap();
        s.act("focus row 3 and jump away", || {
            h.focus();
            state.scroll_to_row(5_000);
        });
        s.settle().unwrap();
        assert!(top(&s) > 4_000, "the window moved");
        assert_eq!(crate::focus::focused(), Some(h.id()), "row 3 keeps focus");
        assert_eq!(state.focused_row(), Some(3), "kept built, pinned");
    }

    /// Focus on a row of another list keyed as one of this list's rows
    /// pins nothing here: only a node under this list's container is its.
    #[test]
    fn focus_in_a_sibling_list_pins_no_row_here() {
        let b = ListState::new(30.0);
        let h = crate::FocusHandle::new();
        let (sa, sb) = (ListState::new(30.0), b.clone());
        let ui = move || {
            let rows = |focus: Option<crate::FocusHandle>| {
                move |i: u64| {
                    let d = div().height(24.0).child(text(format!("row {i}")));
                    match focus {
                        Some(h) if i == 3 => d.track_focus(h).into_element(),
                        _ => d.into_element(),
                    }
                }
            };
            div()
                .width(300.0)
                .child(virtual_list(&sa, ROWS, |i| i, rows(Some(h))).height(200.0))
                .child(virtual_list(&sb, ROWS, |i| i, rows(None)).height(200.0))
                .into_element()
        };
        let mut s = Scenario::measuring(ui, 300, 400, 1.0).unwrap();
        s.settle().unwrap();
        s.act("focus the first list's row 3, and scroll the second", || {
            h.focus();
            b.scroll_to_row(5_000);
        });
        s.settle().unwrap();
        assert_eq!(crate::focus::focused(), Some(h.id()));
        assert_eq!(b.focused_row(), None, "the second list holds no focus");
        assert!(b.pinned().is_empty(), "and pins none of its rows");
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

    /// Whether its thumb is shown.
    fn thumb_shown(state: &ListState) -> bool {
        state.0.thumb_shown.get_untracked()
    }

    #[test]
    fn the_thumb_shows_while_the_reader_scrolls_and_hides_at_rest() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        assert!(!thumb_shown(&state), "hidden at rest, as an overlay scrollbar");
        s.act("to the middle", || state.scroll_to_fraction(0.5));
        s.settle().unwrap();
        assert!(!thumb_shown(&state), "a program's scroll shows none");
        s.hover(100.0, 200.0);
        s.wheel(100.0);
        s.settle().unwrap();
        assert!(thumb_shown(&state), "the reader scrolled it");
        s.wait(std::time::Duration::from_millis(THUMB_SHOWN_MS.saturating_sub(100)));
        s.wheel(100.0);
        s.settle().unwrap();
        s.wait(std::time::Duration::from_millis(THUMB_SHOWN_MS.saturating_sub(100)));
        s.settle().unwrap();
        assert!(thumb_shown(&state), "shown from the last scroll on");
        s.wait(std::time::Duration::from_millis(200));
        s.settle().unwrap();
        assert!(!thumb_shown(&state), "hidden once the reader stopped");
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
                    .width(300.0)
                    .child(virtual_list(&st, n.get(), |i| i, |i| text(format!("row {i}")).into_element()).height(400.0))
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

    /// A million rows of 24 px in a 300 px viewport.
    fn million(state: &ListState) -> Scenario {
        let st = state.clone();
        Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            1_000_000,
                            |i| i,
                            |i| div().height(24.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap()
    }

    /// A fling under way, its scroll and its reach in flight to the UI,
    /// when the program jumps far: the delta, scrolled from where the list
    /// was, is dropped, and its reach with it, so the rows built are those
    /// around where the list lands, as a jump from rest builds them.
    #[test]
    fn a_jump_during_a_fling_builds_no_rows_for_the_flings_reach() {
        let built = |s: &Scenario| rows(s).into_iter().map(|r| r.0).collect::<Vec<_>>();
        let calm = ListState::new(30.0);
        let mut r = million(&calm);
        r.settle().unwrap();
        r.act("far", || calm.scroll_to_row(5_000));
        r.frame("jumped").unwrap();
        let want = built(&r);
        let state = ListState::new(30.0);
        let mut s = million(&state);
        s.settle().unwrap();
        s.hover(150.0, 150.0);
        s.frame("hovered").unwrap();
        let dt = 1.0 / 60.0;
        let mut t = crate::clock::current_seconds();
        let mut wheel = |s: &mut Scenario| {
            t += dt;
            crate::clock::advance_to(std::time::Duration::from_secs_f32(t));
            let (_, headless) = s.parts();
            assert!(headless.wheel((150.0, 150.0), (0.0, 80.0), t), "the list scrolls");
        };
        for _ in 0..6 {
            wheel(&mut s);
            s.frame("flung").unwrap();
        }
        assert!(
            built(&s).len() > want.len(),
            "a fling's reach builds rows past the viewport's margin"
        );
        wheel(&mut s);
        s.act("far", || state.scroll_to_row(5_000));
        s.frame("jumped").unwrap();
        assert_eq!(built(&s), want, "around where it landed, as from rest");
    }

    /// The list's render writes nothing: the window it builds is taken in
    /// by its layout hook, so the cold check's render of every instance
    /// (which asserts no live write) leaves it as it was, through a scroll
    /// asked for, the wheel, rows added and a followed tail.
    #[test]
    fn a_list_renders_purely_under_the_cold_check() {
        let state = ListState::new(30.0);
        let n = Signal::new(1_000_000_u64);
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            n.get(),
                            |i| i,
                            |i| div().height(24.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.check_cold(true);
        s.settle().unwrap();
        // A fling the compositor scrolls between the UI's frames: the
        // window reaches as far as it may go before the next commit.
        s.hover(150.0, 150.0);
        s.frame("hovered").unwrap();
        // A UI busy for six frames: the compositor scrolls on meanwhile.
        let dt = 1.0 / 60.0;
        s.parts().1.set_ui_late(std::time::Duration::from_secs_f32(dt * 6.0));
        let mut t = crate::clock::current_seconds();
        for _ in 0..12 {
            for _ in 0..7 {
                t += dt;
                crate::clock::advance_to(std::time::Duration::from_secs_f32(t));
                let (_, headless) = s.parts();
                assert!(headless.wheel((150.0, 150.0), (0.0, 50.0), t), "the list scrolls");
            }
            s.frame("the UI's commit").unwrap();
        }
        s.settle().unwrap();
        s.act("far", || state.scroll_to_row(30));
        s.settle().unwrap();
        assert_eq!(top(&s), 30, "where it was asked to go");
        s.act("more", || n.set(2_000_000));
        s.settle().unwrap();
        s.act("tail", || {
            state.follow_tail(true);
            state.scroll_to_end();
        });
        s.settle().unwrap();
        s.act("more again", || n.set(2_000_010));
        s.settle().unwrap();
        assert!(rows(&s).iter().any(|r| r.0 == 2_000_009), "the tail followed");
    }

    /// The top of `row`'s box, if built.
    fn top_of(s: &Scenario, row: u64) -> Option<f32> {
        rows(s).into_iter().find(|r| r.0 == row).map(|r| r.1)
    }

    /// A list of 200 rows 24 px tall in a 300 px viewport, each showing
    /// the id `ids` holds at its index, keyed by it.
    fn swappable(state: &ListState, ids: Signal<Vec<u64>>) -> Scenario {
        let st = state.clone();
        Scenario::measuring(
            move || {
                let v = Rc::new(ids.get());
                let (k, r) = (Rc::clone(&v), Rc::clone(&v));
                let at = move |v: &[u64], i: u64| usize::try_from(i).ok().and_then(|i| v.get(i)).copied().unwrap_or(0);
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            u64::try_from(v.len()).unwrap_or(0),
                            move |i| at(&k, i),
                            move |i| {
                                div()
                                    .height(24.0)
                                    .child(text(format!("row {}", at(&r, i))))
                                    .into_element()
                            },
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap()
    }

    /// A bottom alignment asked for once, then the reader wheels to anchor
    /// a row at the top, and the data swaps that row with the next: the
    /// plan's anchor replaces every earlier request, so the old bottom
    /// alignment does not come back and the reader's row stays at the top.
    #[test]
    fn a_reorder_after_the_wheel_brings_back_no_earlier_request() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..200).collect());
        let mut s = swappable(&state, ids);
        s.settle().unwrap();
        s.act("row 40's bottom at the viewport's", || {
            state.scroll_to_row_bottom(40, 0.0)
        });
        s.settle().unwrap();
        s.hover(150.0, 150.0);
        s.wheel(120.0);
        s.settle().unwrap();
        let reader = top(&s);
        let shown = top_of(&s, reader);
        let at = usize::try_from(state.anchor().row).unwrap();
        s.act("swap it with the next", || ids.update(|v| v.swap(at, at + 1)));
        s.settle().unwrap();
        assert_eq!(top(&s), reader, "the reader's row stays at the top");
        assert_eq!(top_of(&s, reader), shown, "where it was on screen");
        assert_eq!(state.anchor().row, u64::try_from(at + 1).unwrap());
    }

    /// The data reorders under the reader: the row at the viewport's top
    /// swaps places with the next one, of the same height. The reader's row
    /// (by key) stays at the top, now at the next index: the plan moves the
    /// container with it, and the hook does not take the row now at the old
    /// offset for the anchor.
    #[test]
    fn a_row_moved_to_another_index_stays_where_the_reader_sees_it() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..200).collect());
        let mut s = swappable(&state, ids);
        s.settle().unwrap();
        s.act("to row 30", || state.scroll_to_row(30));
        s.settle().unwrap();
        assert_eq!(top(&s), 30);
        s.act("swap 30 and 31", || ids.update(|v| v.swap(30, 31)));
        s.settle().unwrap();
        assert_eq!(top(&s), 30, "the reader's row stays at the top");
        assert_eq!(state.anchor().row, 31, "at its new index");
        assert_eq!(state.len(), 200, "what the list holds is what it shows");
    }

    /// The data reorders under the reader, and in the same resolve a later
    /// sibling, mounting, asks (from its setup's effect, which runs after
    /// the list rendered) for row 100: the plan the list built follows the
    /// reader's row against an older request, so the newer one wins and
    /// row 100 is at the top.
    #[test]
    fn a_scroll_asked_after_the_render_wins_over_the_plans_anchor() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..200).collect());
        let go = Signal::new(false);
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                let v = Rc::new(ids.get());
                let (k, r) = (Rc::clone(&v), Rc::clone(&v));
                let at = move |v: &[u64], i: u64| usize::try_from(i).ok().and_then(|i| v.get(i)).copied().unwrap_or(0);
                let asker = go.get().then(|| {
                    let st = st.clone();
                    crate::component((), move |_| {
                        let _ask = crate::Effect::new(move || st.scroll_to_row(100));
                        || Element::Empty
                    })
                    .into_element()
                });
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            u64::try_from(v.len()).unwrap_or(0),
                            move |i| at(&k, i),
                            move |i| {
                                div()
                                    .height(24.0)
                                    .child(text(format!("row {}", at(&r, i))))
                                    .into_element()
                            },
                        )
                        .height(300.0),
                    )
                    .child(asker)
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.act("to row 30", || state.scroll_to_row(30));
        s.settle().unwrap();
        assert_eq!(top(&s), 30);
        s.act("swap 30 and 31, and ask for row 100 after", || {
            ids.update(|v| v.swap(30, 31));
            go.set(true);
        });
        s.settle().unwrap();
        assert_eq!(state.anchor().row, 100, "the newer request holds");
        assert_eq!(top(&s), 100, "row 100 at the top");
    }

    /// A frame whose pass lays out again (a plan moved the container): what
    /// the frame reports of its layout counts both layouts, the first of
    /// which rebuilt the frame for the rows that moved among their
    /// siblings, not only the repeat (which only moved the container).
    #[test]
    fn a_pass_laid_out_again_reports_both_layouts() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..200).collect());
        let mut s = swappable(&state, ids);
        s.settle().unwrap();
        s.act("to row 30", || state.scroll_to_row(30));
        s.settle().unwrap();
        s.act("swap 30 and 31", || ids.update(|v| v.swap(30, 31)));
        let report = s.frame("swapped").unwrap();
        assert!(
            report.visits.rebuilt >= 1,
            "the first layout's rebuild is counted: {}",
            report.visits
        );
    }

    /// A binding read in a pass's repeat layout: what the renders built is
    /// taken in between each pass's two layouts, and the taking moves
    /// `gate` on, which the binding reads, so the repeat runs it again and
    /// it reads the signal `gate` now picks. The frame ends with the
    /// repeat's read, and that read is tracked: the signal it picked
    /// changing lays the node out again.
    #[test]
    fn a_binding_read_in_the_repeat_layout_is_tracked() {
        let gate = Signal::new(0_u32);
        let picks = [Signal::new(30.0_f32), Signal::new(60.0_f32)];
        let handle = ScrollHandle::new();
        let armed = Rc::new(Cell::new(true));
        let a = Rc::clone(&armed);
        handle.on_built(move |_| {
            if !a.get() {
                return false;
            }
            gate.set(gate.get_untracked() + 1);
            // The container moved: the pass lays out again.
            true
        });
        let mut s = Scenario::measuring(
            move || {
                let built: Rc<dyn std::any::Any> = Rc::new(());
                div()
                    .width(300.0)
                    .child(
                        div()
                            .height(100.0)
                            .overflow_y(crate::Overflow::Scroll)
                            .track_scroll_built(&handle, built)
                            .child(div().height(400.0)),
                    )
                    .child(div().named("gauge").height(10.0).width(move || {
                        let g = gate.get();
                        if g == 0 {
                            10.0
                        } else {
                            picks[usize::from(g % 2 == 1)].get()
                        }
                    }))
                    .into_element()
            },
            300,
            400,
            1.0,
        )
        .unwrap();
        let width = |s: &Scenario| {
            s.last()
                .nodes
                .iter()
                .find(|n| n.name == Some("gauge"))
                .map(|n| n.bounds.width)
        };
        s.frame("armed").unwrap();
        armed.set(false);
        let g = gate.get_untracked();
        assert!(g >= 1, "the frame took in what was built");
        let picked = picks[usize::from(g % 2 == 1)];
        assert_eq!(
            width(&s),
            Some(picked.get_untracked()),
            "the frame ends with the repeat's layout of the gauge"
        );
        s.act("change the pick", || picked.set(90.0));
        s.settle().unwrap();
        assert_eq!(width(&s), Some(90.0), "what the repeat read lays the gauge out again");
    }

    /// A frame whose passes run out: a hook prepends a row after every
    /// pass, so the reader's row moves down an index each time, the last
    /// pass too. The plan the last pass built is taken in all the same, and
    /// the container placed for it within that pass: what the list holds is
    /// what it shows, and the reader's row is painted where it was.
    #[test]
    fn the_last_layout_pass_takes_in_its_plan() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((1000..1200).collect());
        let armed = Rc::new(Cell::new(false));
        let other = ScrollHandle::new();
        let (a, next) = (Rc::clone(&armed), Rc::new(Cell::new(0_u64)));
        other.on_layout(move |_| {
            if !a.get() {
                return false;
            }
            let id = next.get();
            next.set(id + 1);
            ids.update(|v| v.insert(0, id));
            true
        });
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                let v = Rc::new(ids.get());
                let (k, r) = (Rc::clone(&v), Rc::clone(&v));
                let at = move |v: &[u64], i: u64| usize::try_from(i).ok().and_then(|i| v.get(i)).copied().unwrap_or(0);
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            u64::try_from(v.len()).unwrap_or(0),
                            move |i| at(&k, i),
                            move |i| {
                                div()
                                    .height(24.0)
                                    .child(text(format!("row {}", at(&r, i))))
                                    .into_element()
                            },
                        )
                        .height(288.0),
                    )
                    .child(
                        div()
                            .height(10.0)
                            .overflow_y(Overflow::Scroll)
                            .track_scroll(&other)
                            .child(div().height(20.0)),
                    )
                    .into_element()
            },
            300,
            298,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.act("to id 1050", || state.scroll_to_row(50));
        s.settle().unwrap();
        assert_eq!(top(&s), 1050);
        let shown = top_of(&s, 1050);
        armed.set(true);
        s.frame("passes run out").unwrap();
        armed.set(false);
        let held = ids.with_untracked(Vec::len);
        assert!(held > 200, "rows came in during the frame");
        assert_eq!(
            state.len(),
            u64::try_from(held).unwrap(),
            "the list holds the rows the frame built"
        );
        assert_eq!(top(&s), 1050, "the reader's row is painted at the top");
        assert_eq!(top_of(&s, 1050), shown, "at the offset it was");
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
                    .width(300.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child(virtual_list(
                        &state,
                        n.get(),
                        |i| i,
                        |i| div().height(25.0).child(text(format!("row {i}"))).into_element(),
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
                .find(|n| matches!(&n.content, Content::Text { content, .. } if &**content == "below"))
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
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            30,
                            |i| i,
                            |i| div().height(25.0).child(text(format!("row {i}"))).into_element(),
                        )
                        .height(400.0),
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
        let far = ROWS - 10;
        let (h, extent, mean) = (state.row_height(3), state.extent(), state.row_height(far));
        state.adjust_row_height(3, 32.0);
        assert_eq!(state.row_height(3), h + 32.0);
        // A measured height moved, so did the mean the rows not built take.
        assert!(state.row_height(far) > mean);
        state.adjust_row_height(3, -32.0);
        assert_eq!(state.row_height(3), h);
        assert_eq!(state.extent(), extent, "back where it was, the mean too");
        // A row not measured keeps its estimate.
        let estimate = state.row_height(far);
        state.adjust_row_height(far, 32.0);
        assert_eq!(state.row_height(far), estimate);
    }

    /// Rows measured once and laid out again at another height: the
    /// estimate the rows not yet built take is the mean of the heights the
    /// rows measured have now, not of the ones they had when first seen.
    #[test]
    fn the_estimate_follows_measured_rows_that_change_height() {
        let state = ListState::new(24.0);
        let height = Signal::new(24.0_f32);
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                let h = height.get();
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            1_000,
                            |i| i,
                            move |i| div().height(h).child(text(format!("row {i}"))).into_element(),
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        assert_eq!(state.row_height(999), 24.0);
        s.act("every row twice as tall", || height.set(48.0));
        s.settle().unwrap();
        assert!(state.is_measured(0) && state.row_height(0) == 48.0);
        assert_eq!(state.row_height(999), 48.0, "the rows not built take the new mean");
    }

    /// Frontiers 7.3 item 5: rows measured, then the data reordered so they
    /// lie far from the viewport. Their heights went with them: a row off
    /// screen has its own height, not the one measured at its new index.
    #[test]
    fn heights_measured_follow_their_rows_through_a_sort() {
        let state = ListState::new(30.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..200).collect());
        let tall = |id: u64| if id.is_multiple_of(2) { 40.0 } else { 24.0 };
        let st = state.clone();
        let mut s = Scenario::measuring(
            move || {
                let v = Rc::new(ids.get());
                let (k, r) = (Rc::clone(&v), Rc::clone(&v));
                let at = move |v: &[u64], i: u64| usize::try_from(i).ok().and_then(|i| v.get(i)).copied().unwrap_or(0);
                div()
                    .width(300.0)
                    .child(
                        virtual_list(
                            &st,
                            200,
                            move |i| at(&k, i),
                            move |i| {
                                let id = at(&r, i);
                                div().height(tall(id)).child(text(format!("row {id}"))).into_element()
                            },
                        )
                        .height(300.0),
                    )
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        let measured: Vec<u64> = (0..200).filter(|&p| state.is_measured(p)).collect();
        assert!(measured.len() > 4 && measured.len() < 60, "{measured:?}");
        // Rotated by an odd count: each row's new neighbour has the other height.
        let to = |p: u64| Some((p + 101) % 200);
        s.act("rotate the data, back to the top", || {
            ids.update(|v| v.rotate_right(101));
            state.reorder(200, to);
            state.scroll_to_row(0);
        });
        s.settle().unwrap();
        for &p in &measured {
            let q = to(p).unwrap();
            assert!(state.is_measured(q), "row {p} measured, now at {q}");
            assert_eq!(state.row_height(q), tall(p), "row {p}'s height, now at {q}");
        }
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
