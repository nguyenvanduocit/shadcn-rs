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

use std::fmt;
use std::rc::{Rc, Weak};

use super::extent_tree::{ExtentTree, UNITS, pixels, units};
use crate::element::{Div, Element, ElementKey, IntoElement, Role, div};
use crate::engine::{ForkSlab, SlabId};
use crate::events::{DragEvent, DragPhase};
use crate::prelude::*;
use crate::reactive::{Signal, detached};
use crate::scroll::{Move, ScrollHandle, ScrollLayout};
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

/// A list's heights lent to be reordered off the UI thread
/// ([`ListState::lend_heights`]): `Send`, a snapshot sharing the list's.
#[derive(Debug)]
pub struct Heights {
    extents: ExtentTree,
    /// Which loan it is: only the latest is taken back.
    loan: u64,
    /// Which row was where when lent.
    shape: u64,
    /// Whose loan it is.
    holder: u64,
}

/// The loans of heights made, numbered.
static LOANS: crate::engine::IdKind = crate::engine::IdKind::new("heights loan");
/// The holders of lists' heights ([`ListState::lend_heights_for`]), numbered.
pub(crate) static HOLDERS: crate::engine::IdKind = crate::engine::IdKind::new("heights holder");

impl Heights {
    /// Which loan it is ([`ListState::end_loan`] ends only its own).
    pub(crate) fn loan(&self) -> u64 {
        self.loan
    }

    /// The heights, as [`ListState::reorder`] moves them: `rows` rows now,
    /// the row at each old position `p` at `to(p)`. O(m log m) for the `m`
    /// rows measured: work for the thread that sorts.
    pub fn reorder(&mut self, rows: u64, to: impl Fn(u64) -> Option<u64>) {
        self.extents.reorder(rows, to);
    }

    /// [`Self::reorder`], given up (`false`, the heights as they were) once
    /// `stale` says it is no longer wanted: asked every few thousand rows.
    pub(crate) fn reorder_until(
        &mut self,
        rows: u64,
        to: impl Fn(u64) -> Option<u64>,
        stale: &dyn Fn() -> bool,
    ) -> bool {
        self.extents.reorder_until(rows, to, stale)
    }

    /// `rows` rows, none measured, at the estimate they have: the heights
    /// for rows whose old places are not known.
    pub(crate) fn forget(&mut self, rows: u64) {
        self.extents = ExtentTree::new(rows, self.extents.estimate());
    }

    #[must_use]
    pub fn extents(&self) -> &ExtentTree {
        &self.extents
    }

    /// The heights to change before they come back (a measurement the
    /// worker knows of); what [`ListState::reorder_with`] puts in place.
    pub fn extents_mut(&mut self) -> &mut ExtentTree {
        &mut self.extents
    }
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
/// What asks a refused holder's request again ([`ListState::wait_for_heights`]).
pub(crate) type Waiter = std::rc::Weak<dyn Fn()>;

/// Ends `loan` of `list` if dropped while unwinding: the loan a panicking
/// caller's closure took down with it ([`ListState::reorder_with`]).
struct EndLoanOnUnwind {
    list: ListState,
    loan: u64,
}

impl Drop for EndLoanOnUnwind {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.list.end_loan(self.loan);
        }
    }
}

/// A loan of a list's heights: which, at what shape, and whose. A loan
/// belongs to its holder: only it, or the loan's own end, takes it back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Lent {
    loan: u64,
    shape: u64,
    holder: u64,
}

/// A list's state, in [`LISTS`].
struct State {
    /// Bumped when the window moves: the list renders again.
    version: Signal<u64>,
    /// The heights: a persistent tree, so the fork's copy of the state is
    /// O(1) in them and its writes copy only the paths they walk.
    extents: ExtentTree,
    /// Bumped by every change to which row is where (a splice, rows added
    /// or removed at the end, a reorder, a reset): heights lent before one
    /// no longer match ([`ListState::reorder_with`]).
    shape: u64,
    /// The live loan of the heights, if any ([`ListState::lend_heights_for`]):
    /// its id, the shape it was made at and the holder it belongs to. A
    /// newer loan voids it.
    lent: Option<Lent>,
    /// Who holds the heights ([`ListState::claim_heights`]): one holder at
    /// a time, so two never void each other's loans in turn for ever.
    holder: Option<u64>,
    /// Holders refused the heights, each woken once to ask again when they
    /// are let go ([`ListState::release_heights`]); one entry per holder.
    /// Rarely written: a check's copy shares it, copied on its first write
    /// there (each waiter by identity, a `Weak`).
    waiting: Rc<Vec<(u64, Waiter)>>,
    /// The rows measured since the live loan, and at what height: what its
    /// heights lack. Kept, cleared, for the next loan.
    since_lent: Noted,
    anchor: Anchor,
    /// The scroll asked for, the viewport not showing it yet: the anchor is
    /// held through the layouts that measure the rows around it. One slot:
    /// each request replaces the one before, whole, so none comes back.
    request: Option<Request>,
    /// The revision the next request takes.
    requests: u64,
    window: Window,
    /// The keys of the window's rows and the rows pinned, as the last
    /// render built them: a check's copy shares them ([`Built::set`]).
    built: Rc<Built>,
    /// The scroll container's content box as last laid out: width, height.
    viewport: (f32, f32),
    /// Its top in window pixels (where a thumb drag is measured from).
    top: f32,
    scroll: ScrollHandle,
    /// Pixels built beyond the viewport, above and below; `None`: one viewport.
    prefetch: Option<f32>,
    /// Keep the end in view as rows are added.
    follow: bool,
    /// The view is at the end (where a following list stays).
    pinned: bool,
    /// Asked to show the end, once.
    to_end: bool,
    /// Room below the last row, in pixels.
    end_space: f32,
    /// As tall as its rows, up to this many pixels; `None`: sized by its parent.
    fit: Option<f32>,
    /// Its height when sized by its rows, as last laid out; `None` before.
    height: Signal<Option<f32>>,
    /// The thumb is not drawn (`scrollbar-width: none`): read as the list
    /// renders, so a scroller that hides it as it scrolls itself (an
    /// effect on its own state) builds the list again.
    no_scrollbar: Signal<bool>,
    /// Where on the thumb a drag took hold, from its top.
    grab: f32,
    /// The thumb's top and height in the viewport; `None` when all rows fit.
    thumb: Signal<Option<(f32, f32)>>,
    /// The thumb shows: the reader scrolled or dragged it lately. At rest
    /// it is hidden, an overlay scrollbar.
    thumb_shown: Signal<bool>,
    /// When the thumb hides again (on the clock), the latest showing's
    /// deadline; and whether a timer is out for it. One timer at most: it
    /// hides the thumb at the deadline, or, moved since, waits again for
    /// the latest.
    hide_at: std::time::Duration,
    hiding: bool,
    /// The thumb is held: it stays shown until let go.
    dragging: bool,
    /// The owner's hook, run once the window is settled.
    hook: Option<Hook>,
    /// The row holding focus, as last laid out, and its key.
    focus_row: Option<(u64, ElementKey)>,
    /// Where the scroll container sits, as last laid out: its rows are
    /// its children.
    viewport_path: Option<crate::geometry::NodePath>,
    /// The row its owner treats as current (an active option).
    active: Option<u64>,
    /// Or what tells it, read as the list renders ([`ListState::active_from`]).
    /// Held by the owner's instance while it is mounted: a list state that
    /// outlives it (reused by another list) never calls it once it left.
    active_from: crate::Holder<ActiveFrom>,
    /// Rows on each side of the focused one kept built with it.
    around_focus: u64,
    /// The last render's plan, taken in: taken once.
    taken: Weak<dyn std::any::Any>,
}

/// The rows the last render built ([`State::built`]): replaced whole when
/// a render's plan is taken in, never written in part.
#[derive(Default)]
struct Built {
    /// The keys of the window's rows, in order.
    keys: Vec<ElementKey>,
    /// The rows built outside the window (pinned), in order, and how many
    /// of them come before it.
    pins: Vec<(u64, ElementKey)>,
    pins_before: usize,
}

impl Built {
    /// `built` made these rows: in place while live holds it alone (its
    /// room kept, no allocation once it has enough); a new one where a
    /// check's copy shares it, which costs what it holds, not what it
    /// replaces, and leaves live's as it was.
    fn set(built: &mut Rc<Self>, keys: &[ElementKey], pins: &[(u64, ElementKey)], pins_before: usize) {
        match Rc::get_mut(built) {
            Some(b) => {
                b.keys.clear();
                b.keys.extend_from_slice(keys);
                b.pins.clear();
                b.pins.extend_from_slice(pins);
                b.pins_before = pins_before;
            }
            None => {
                *built = Rc::new(Self {
                    keys: keys.to_vec(),
                    pins: pins.to_vec(),
                    pins_before,
                });
            }
        }
    }
}

// A logged value (ADR-0034 S7, audited): `&State` changes nothing.
crate::engine::fork_value_audited!(State);
impl crate::engine::parts_seal::Sealed for State {}
impl crate::engine::Parts for State {
    /// Each field that drops anything on its own box (S9, review r17): the
    /// shared ones let go of at once if live holds them too. Every field is
    /// named, none dropped by the pattern's rest: an app callback (`hook`)
    /// left to drop with this frame would panic outside the drain's catch,
    /// into the unwind of the rest (the drain lint, `obligations.rs`).
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        use crate::engine::whole;
        let Self {
            version,
            extents,
            shape,
            lent,
            holder,
            waiting,
            since_lent,
            anchor,
            request,
            requests,
            window,
            built,
            viewport,
            top,
            scroll,
            prefetch,
            follow,
            pinned,
            to_end,
            end_space,
            fit,
            height,
            no_scrollbar,
            grab,
            thumb,
            thumb_shown,
            hide_at,
            hiding,
            dragging,
            hook,
            focus_row,
            viewport_path,
            active,
            active_from,
            around_focus,
            taken,
        } = self;
        // The app's callback, its own box (or let go of at once, live
        // sharing it).
        hook.parts(out);
        waiting.parts(out);
        built.parts(out);
        taken.parts(out);
        // The engine's own: memory, a box each where it has drop glue.
        whole(extents, out);
        whole(since_lent, out);
        whole(viewport_path, out);
        // Data: nothing to drop.
        let _data = (
            version,
            shape,
            lent,
            holder,
            anchor,
            request,
            requests,
            window,
            viewport,
            top,
            scroll,
            prefetch,
            follow,
            pinned,
            to_end,
            end_space,
            fit,
            height,
            no_scrollbar,
            grab,
            thumb,
            thumb_shown,
            hide_at,
            hiding,
            dragging,
            focus_row,
            active,
            active_from,
            around_focus,
        );
    }
}
impl crate::engine::sealed::Sealed for State {}
impl crate::engine::ForkClone for State {
    /// O(1) in the rows and the holders: the heights are a persistent tree
    /// (a count bumped); the rows built and the holders waiting are shared,
    /// copied only by a write to them there ([`Built::set`], copy-on-write);
    /// the rows noted since a loan share their entries ([`Noted`]: O(its
    /// own tail), empty live). Callbacks are held by identity: the hook an
    /// `Rc`, each waiter and the plan taken a `Weak`, what tells the active
    /// row a [`crate::Holder`] (an id), so the copy holds no callback live
    /// does not. The rest is `Copy`. Every field is named, so one added
    /// states its cost.
    fn fork_clone(&self) -> Self {
        let Self {
            version,
            extents,
            shape,
            lent,
            holder,
            waiting,
            since_lent,
            anchor,
            request,
            requests,
            window,
            built,
            viewport,
            top,
            scroll,
            prefetch,
            follow,
            pinned,
            to_end,
            end_space,
            fit,
            height,
            no_scrollbar,
            grab,
            thumb,
            thumb_shown,
            hide_at,
            hiding,
            dragging,
            hook,
            focus_row,
            viewport_path,
            active,
            active_from,
            around_focus,
            taken,
        } = self;
        Self {
            version: *version,
            extents: extents.clone(),
            shape: *shape,
            lent: *lent,
            holder: *holder,
            waiting: Rc::clone(waiting),
            since_lent: since_lent.clone(),
            anchor: *anchor,
            request: *request,
            requests: *requests,
            window: *window,
            built: Rc::clone(built),
            viewport: *viewport,
            top: *top,
            scroll: *scroll,
            prefetch: *prefetch,
            follow: *follow,
            pinned: *pinned,
            to_end: *to_end,
            end_space: *end_space,
            fit: *fit,
            height: *height,
            no_scrollbar: *no_scrollbar,
            grab: *grab,
            thumb: *thumb,
            thumb_shown: *thumb_shown,
            hide_at: *hide_at,
            hiding: *hiding,
            dragging: *dragging,
            hook: hook.as_ref().map(Rc::clone),
            focus_row: *focus_row,
            viewport_path: viewport_path.clone(),
            active: *active,
            active_from: *active_from,
            around_focus: *around_focus,
            taken: Weak::clone(taken),
        }
    }
}

crate::engine::engine_local! {
    /// Every list's state, by its id (ADR-0028, rule 4: a handle is an
    /// id): what a cold layout writes through a list (its anchor, its
    /// window, the heights it measures) is the fork's, put back after.
    [Fork(log)] static LISTS: std::cell::RefCell<ForkSlab<State>> = std::cell::RefCell::default();
}

/// A virtual list's state: heights, anchor, window. A `Copy` handle, owned
/// by the scope it is made in (made outside any, it lives until
/// [`Self::dispose`]). Once its owner ended it is inert: it holds no rows,
/// builds none and writes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListState(SlabId);

impl fmt::Debug for ListState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Quiet: a disposed list prints as one, it is no bug to print it.
        match self.peek(|s| (s.extents.len(), s.anchor, s.window)) {
            Some((rows, anchor, window)) => f
                .debug_struct("ListState")
                .field("rows", &rows)
                .field("anchor", &anchor)
                .field("window", &window)
                .finish_non_exhaustive(),
            None => f.write_str("ListState(disposed)"),
        }
    }
}

impl Default for ListState {
    fn default() -> Self {
        Self::new(32.0)
    }
}

/// Pixels as units, signed (an anchor's offset).
fn signed_units(px: f64) -> f64 {
    (px * f64::from(UNITS)).round()
}

/// The rows measured since a loan, at their heights, appended only. A copy
/// is O(1): it shares the entries and keeps its own length, so a fork's
/// copy of a list's state costs nothing here however many there are. An
/// entry past a copy's length is another copy's (a rolled-back fork's),
/// cut off at its next push; a clear while another copy shares the entries
/// starts anew.
///
/// A shared allocation never grows: a copy that pushes while another
/// shares the entries writes in their spare room, or, with none, to a tail
/// of its own (`tail`, after the shared `len`). A fork's copy so costs
/// O(its pushes) and leaves the live copy's allocation as it was, whatever
/// its room (Driscoll et al. 1989: the fork appends a version, it never
/// moves the one it shares).
///
/// The shared entries are the world's that made them ([`WorldRefCell`],
/// review r18, decision 2): only a copy of that world writes into them, in
/// place or in their spare room; a check's copy of live entries writes to
/// its tail, or to entries of its own.
#[derive(Clone)]
struct Noted {
    items: Rc<crate::engine::WorldRefCell<Vec<(u64, u32)>>>,
    len: usize,
    /// Entries after the shared `len`, pushed while the shared ones had no
    /// room and another copy shared them.
    tail: Vec<(u64, u32)>,
}

impl Default for Noted {
    fn default() -> Self {
        Self {
            items: Self::fresh(),
            len: 0,
            tail: Vec::new(),
        }
    }
}

impl Noted {
    /// New entries, of the running world.
    fn fresh() -> Rc<crate::engine::WorldRefCell<Vec<(u64, u32)>>> {
        Rc::new(crate::engine::WorldRefCell::new(
            "a list's rows measured since a loan (`Noted`)",
            Vec::new(),
        ))
    }

    /// The entries, when this copy alone holds them and its world made them.
    fn alone(&mut self) -> Option<&mut Vec<(u64, u32)>> {
        Rc::get_mut(&mut self.items)
            .filter(|i| i.is_here())
            .map(crate::engine::WorldRefCell::get_mut)
    }

    fn push(&mut self, entry: (u64, u32)) {
        let len = self.len;
        if let Some(items) = Rc::get_mut(&mut self.items).filter(|i| i.is_here()) {
            // Alone: the tail, if any, joins the entries, which may grow.
            let items = items.get_mut();
            items.truncate(len);
            items.append(&mut self.tail);
            items.push(entry);
            self.len = items.len();
            return;
        }
        if self.tail.is_empty() && self.items.is_here() {
            let mut items = self.items.borrow_mut();
            if items.capacity() > self.len {
                items.truncate(self.len);
                items.push(entry);
                self.len = items.len();
                return;
            }
        }
        self.tail.push(entry);
    }

    /// Empty, its room kept while no other copy shares it.
    fn clear(&mut self) {
        match self.alone() {
            Some(items) => items.clear(),
            None => self.items = Self::fresh(),
        }
        self.len = 0;
        self.tail.clear();
    }

    /// Empty, its room let go.
    fn release(&mut self) {
        self.items = Self::fresh();
        self.len = 0;
        self.tail = Vec::new();
    }

    /// The entries, in order, taken out: it is empty after. Its room goes
    /// with them while no other copy shares it (no allocation), to come
    /// back through [`Self::recycle`].
    fn take(&mut self) -> Vec<(u64, u32)> {
        let len = std::mem::take(&mut self.len);
        let mut taken = match self.alone() {
            Some(items) => {
                let mut taken = std::mem::take(items);
                taken.truncate(len);
                taken
            }
            None => {
                let taken = self.items.borrow().get(..len).map(<[_]>::to_vec).unwrap_or_default();
                self.items = Self::fresh();
                taken
            }
        };
        taken.append(&mut self.tail);
        taken
    }

    /// Empty, with the room of `spent` (what [`Self::take`] gave) if it
    /// has none and no other copy shares it.
    fn recycle(&mut self, mut spent: Vec<(u64, u32)>) {
        self.len = 0;
        self.tail.clear();
        if let Some(items) = self.alone() {
            items.clear();
            if items.capacity() < spent.capacity() {
                spent.clear();
                *items = spent;
            }
        } else {
            self.items = Self::fresh();
        }
    }

    fn is_empty(&self) -> bool {
        self.len == 0 && self.tail.is_empty()
    }

    /// The entries, in order.
    fn to_vec(&self) -> Vec<(u64, u32)> {
        let mut entries = self
            .items
            .borrow()
            .get(..self.len)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        entries.extend_from_slice(&self.tail);
        entries
    }
}

impl fmt::Debug for Noted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.to_vec()).finish()
    }
}

/// Where a reorder puts the anchor and the focused row, worked out before
/// the write that applies it ([`ListState::follow_reorder`]).
struct Follow {
    anchor: Anchor,
    focus_row: Option<(u64, ElementKey)>,
}

impl Follow {
    fn apply(self, s: &mut State) {
        s.anchor = self.anchor;
        s.focus_row = self.focus_row;
        // The rows built, and those pinned, were found by position: the
        // window is decided anew.
        s.window = Window::default();
        Built::set(&mut s.built, &[], &[], 0);
        // The rows around the anchor may be estimates: it holds, as a row
        // scrolled to does, until the viewport shows it measured, aligned
        // as a scroll still pending asked (its bottom, say).
        if !wants_end(s) {
            let bottom = s.request.and_then(|r| r.bottom);
            ask(s, bottom);
        }
    }
}

/// What a reorder read its heights from, validated where it commits
/// ([`ListState::commit_reorder`]).
enum Read {
    /// A loan (`reorder_with`), and the notes it took, to recycle.
    Loan(Lent, Vec<(u64, u32)>),
    /// The list's own heights (plain `reorder`), as read, and its shape.
    Own { heights: ExtentTree, shape: u64 },
}

/// What a reorder worked the anchor and the focused row out from
/// ([`ListState::follow_reorder`]): a mapping that scrolls the list, asks
/// for its end or moves the focused row changes it, and the commit is
/// refused, the change standing.
#[derive(PartialEq)]
struct Seen {
    anchor: Anchor,
    request: Option<Request>,
    requests: u64,
    focus_row: Option<KeyedRow>,
    window: Window,
    pinned: bool,
    to_end: bool,
}

/// A reorder's read set: what it validates ([`Seen`]) and the focused row
/// as found by key, taken before any call of the caller's `to`.
struct ReadSet {
    seen: Seen,
    focus: Option<KeyedRow>,
    /// The window's focused node `focus` was worked out from: state of the
    /// window's, not the list's, so validated beside [`Seen`] (a `to` that
    /// moves focus refuses the reorder).
    focused: Option<crate::focus::FocusId>,
}

impl Seen {
    fn of(s: &State) -> Self {
        Self {
            anchor: s.anchor,
            request: s.request,
            requests: s.requests,
            focus_row: s.focus_row,
            window: s.window,
            pinned: s.pinned,
            to_end: s.to_end,
        }
    }
}

/// `row` was measured at `height` (units): heights lent lack it.
fn note_measured(s: &mut State, row: u64, height: u32) {
    if s.lent.is_some() {
        s.since_lent.push((row, height));
    }
}

/// Which row is where changed: heights lent no longer match.
fn reshaped(s: &mut State) {
    s.shape = s.shape.wrapping_add(1);
}

/// Ask for a scroll, in place of any asked before.
fn ask(s: &mut State, bottom: Option<f32>) {
    let rev = s.requests.wrapping_add(1);
    s.requests = rev;
    s.request = Some(Request { rev, bottom });
}

/// The viewport's height as last laid out (a first guess before that).
fn viewport_height(s: &State) -> f32 {
    match s.viewport.1 {
        h if h > 0.0 => h,
        _ => FIRST_VIEWPORT,
    }
}

/// The rows' extent and the end space, in units.
fn total(s: &State, ext: &ExtentTree) -> u64 {
    ext.extent().saturating_add(u64::from(units(s.end_space)))
}

/// Whether the view is to show the end, wherever the rows' measured
/// heights put it.
fn wants_end(s: &State) -> bool {
    (s.follow && s.pinned) || s.to_end
}

/// The reader's scroll its handle took in, `added` pixels down, laid on the
/// anchor ([`ListState::poke`]): on the bottom asked for, or the anchor's
/// offset, while a jump waits to be shown; on the anchor's offset, its row
/// (and key) kept, otherwise; and a list held at its end scrolled up from
/// it is let go of it there.
fn absorb(s: &mut State, added: f32) {
    if added == 0.0 {
        return;
    }
    if let Some(r) = &mut s.request {
        match &mut r.bottom {
            Some(b) => *b += added,
            None => s.anchor.offset += added,
        }
        return;
    }
    if !wants_end(s) {
        s.anchor.offset += added;
        return;
    }
    if added > 0.0 {
        // Held at its end, pushed on past it: still at its end.
        return;
    }
    let room = top_units_at(s, &s.extents, s.anchor);
    let up = crate::convert::f64_to_u64(signed_units(f64::from(-added)));
    let (row, into) = s.extents.row_at(room.saturating_sub(up));
    s.pinned = false;
    s.to_end = false;
    s.anchor = Anchor {
        row,
        key: None,
        offset: crate::convert::f64_to_f32(pixels(into)),
    };
}

/// The viewport's top in units with the anchor at `a`, within the extent.
fn top_units_at(s: &State, ext: &ExtentTree, a: Anchor) -> u64 {
    let len = ext.len();
    if len == 0 {
        return 0;
    }
    let vp = u64::from(units(viewport_height(s)));
    let room = total(s, ext).saturating_sub(vp);
    if wants_end(s) {
        return room;
    }
    let row = a.row.min(len.saturating_sub(1));
    let base = crate::convert::u64_to_f64(ext.offset_of(row));
    let offset = match s.request.and_then(|r| r.bottom) {
        Some(above) => {
            let height = crate::convert::u64_to_f64(u64::from(ext.height(row).unwrap_or(0)));
            height - crate::convert::u64_to_f64(vp) + signed_units(f64::from(above))
        }
        None => signed_units(f64::from(a.offset)),
    };
    let top = (base + offset).max(0.0);
    crate::convert::f64_to_u64(top).min(room)
}

/// The window over heights `ext`, the anchor at `anchor`, the compositor
/// able to scroll `reach` pixels before the next commit shows
/// ([`ListState::compute`]).
fn compute_at(s: &State, ext: &ExtentTree, anchor: Anchor, reach: f32) -> Window {
    let len = ext.len();
    if len == 0 {
        return Window::default();
    }
    let vp_px = viewport_height(s);
    let vp = u64::from(units(vp_px));
    let rows_end = ext.extent();
    let top = top_units_at(s, ext, anchor);
    let over = u64::from(units(s.prefetch.unwrap_or(vp_px)));
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
    let w = s.window;
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

/// The row of a window starting at `first` whose key is `k` among `keys`,
/// or among the rows `pins` holds.
fn row_keyed(first: u64, keys: &[ElementKey], pins: &[(u64, ElementKey)], k: ElementKey) -> Option<u64> {
    keys.iter()
        .position(|key| *key == k)
        .map(|j| first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)))
        .or_else(|| pins.iter().find(|(_, key)| *key == k).map(|(r, _)| *r))
}

/// What a layout found, measured into the state ([`ListState::laid_out`]).
enum Measured {
    /// Focus left the row kept built for it: the list renders again.
    Unpinned,
    /// The window laid out, the scroll asked for when it was, and which of
    /// the container's children are the window's rows.
    Rows {
        w: Window,
        asked: Option<Request>,
        rows: std::ops::Range<usize>,
    },
}

/// A thumb's top and height in its viewport.
type Thumb = (f32, f32);

/// What a layout settled once its rows were measured
/// ([`ListState::laid_out`]): read in one step, applied outside it.
struct Settled {
    /// The thumb's signal and where it goes.
    thumb: (Signal<Option<Thumb>>, Option<Thumb>),
    /// A list sized by its rows: its height's signal and that height.
    fit: Option<(Signal<Option<f32>>, f32)>,
    at_end: bool,
    wants_end: bool,
    /// The window that shows the viewport.
    next: Window,
}

/// The thumb's top and height over a viewport `viewport` tall whose top is
/// `local` pixels below the first row built, as a scroll container places
/// its own; `None` when every row fits.
fn thumb_geometry(s: &State, local: f32, viewport: f32) -> Option<(f32, f32)> {
    let total = total(s, &s.extents);
    let vp = u64::from(units(viewport));
    (viewport > 0.0 && total > vp.saturating_add(32)).then(|| {
        let height = thumb_height(viewport, pixels(total));
        let room = total.saturating_sub(vp);
        let top = s
            .extents
            .offset_of(s.window.first)
            .saturating_add(u64::from(units(local)))
            .min(room);
        let fraction = if room == 0 {
            0.0
        } else {
            crate::convert::u64_to_f64(top) / crate::convert::u64_to_f64(room)
        };
        let travel = (viewport - height - 2.0 * THUMB_INSET).max(0.0);
        (THUMB_INSET + crate::convert::f64_to_f32(fraction) * travel, height)
    })
}

/// Set `signal` to `value`; whether it changed (the frame is laid out
/// again to show it).
fn set_changed<T: Clone + PartialEq + 'static>(signal: Signal<T>, value: T) -> bool {
    if signal.with_untracked(|v| *v == value) {
        return false;
    }
    signal.set(value);
    true
}

impl ListState {
    /// A list whose rows are about `estimate` pixels tall until measured.
    #[must_use]
    pub fn new(estimate: f32) -> Self {
        let scroll = ScrollHandle::new();
        let state = State {
            version: detached(|| Signal::labeled("virtual list", 0)),
            extents: ExtentTree::new(0, units(estimate)),
            shape: 0,
            lent: None,
            holder: None,
            waiting: Rc::default(),
            since_lent: Noted::default(),
            anchor: Anchor::default(),
            request: None,
            requests: 0,
            window: Window::default(),
            built: Rc::default(),
            viewport: (0.0, 0.0),
            top: 0.0,
            scroll,
            prefetch: None,
            follow: false,
            pinned: false,
            to_end: false,
            end_space: 0.0,
            fit: None,
            height: detached(|| Signal::labeled("virtual list height", None)),
            no_scrollbar: detached(|| Signal::labeled("virtual list no scrollbar", false)),
            grab: 0.0,
            thumb: detached(|| Signal::labeled("virtual list thumb", None)),
            thumb_shown: detached(|| Signal::labeled("virtual list thumb shown", false)),
            hide_at: std::time::Duration::ZERO,
            hiding: false,
            dragging: false,
            hook: None,
            focus_row: None,
            active: None,
            active_from: detached(crate::Holder::new),
            around_focus: 0,
            viewport_path: None,
            taken: Weak::<Plan>::new(),
        };
        let me = Self(crate::engine::allocate(&LISTS, state));
        scroll.on_layout_built(move |l, built| me.laid_out(scroll, l, built));
        scroll.on_built(move |built| me.take(built));
        scroll.on_reveal(move |k| me.reveal(k));
        // The reader scrolled it, moved or held against an edge (which lays
        // nothing out again): the thumb shows.
        scroll.on_wheel(move || me.show_thumb());
        crate::engine::owned_by_scope(move || me.dispose());
        me
    }

    /// Let its state go now: it is inert from here on.
    pub fn dispose(self) {
        // `try_with`: dropped as the thread's state is torn down, it is gone.
        let Ok(Some(s)) = LISTS.try_with(|l| l.borrow_mut().remove(self.0)) else {
            return;
        };
        s.version.dispose_quietly();
        s.thumb.dispose_quietly();
        s.thumb_shown.dispose_quietly();
        s.height.dispose_quietly();
        s.no_scrollbar.dispose_quietly();
        s.active_from.dispose_quietly();
        s.scroll.dispose();
    }

    /// Whether its owner has not ended.
    #[must_use]
    pub fn is_alive(self) -> bool {
        LISTS.with(|l| l.borrow().contains(self.0))
    }

    /// Read its state; `None` once it is gone, which its owner's code
    /// reaching it is a bug, loud in a debug build
    /// ([`crate::engine::check_alive`]). `f` reaches no list.
    #[track_caller]
    fn read<R>(self, f: impl FnOnce(&State) -> R) -> Option<R> {
        let read = self.peek(f);
        #[cfg(debug_assertions)]
        crate::engine::check_alive(read.is_some(), "list state");
        read
    }

    /// [`Self::read`], quiet once it is gone: for the engine's own paths
    /// that may run once after its owner ended (a binding of an element
    /// still in a tree being replaced, a timer it set).
    fn peek<R>(self, f: impl FnOnce(&State) -> R) -> Option<R> {
        LISTS.with(|l| l.borrow().read(self.0, f))
    }

    /// Write its state (nothing once it is gone, loud in a debug build as
    /// [`Self::read`]). `f` reaches no list.
    #[track_caller]
    fn write<R>(self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let wrote = self.poke(f);
        #[cfg(debug_assertions)]
        crate::engine::check_alive(wrote.is_some(), "list state");
        wrote
    }

    /// [`Self::write`], quiet once it is gone (the engine's own paths, as
    /// [`Self::peek`]).
    ///
    /// Every write starts from where the reader has the list: the reader's
    /// scroll its handle took in since it was last placed
    /// ([`ScrollHandle::take_added`]) goes into the anchor first
    /// ([`absorb`]), so a write between a take and the list's layout (a
    /// splice, a reorder, a height measured, a tail followed, a rebase of
    /// any kind) moves the content under the place the reader scrolled to,
    /// not under the one before (ADR-0032 I5: a rebase is additive; a jump,
    /// which assigns the anchor after, drops it).
    fn poke<R>(self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        let added = self.peek(|s| s.scroll).map_or((0.0, 0.0), |h| h.take_added());
        // The list scrolls on y alone: its container's x range is 0, and
        // what the handle took in is clamped to its range as it came.
        debug_assert!(added.0 == 0.0, "a vertical list took in {added:?}");
        let added = added.1;
        LISTS.with(|l| {
            l.borrow().write(self.0, |s| {
                absorb(s, added);
                f(s)
            })
        })
    }

    /// Its scroll container's handle.
    pub(crate) fn scroll(self) -> Option<ScrollHandle> {
        self.read(|s| s.scroll)
    }

    /// Rows it holds.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.read(|s| s.extents.len()).unwrap_or(0)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Where it is scrolled to.
    #[must_use]
    pub fn anchor(&self) -> Anchor {
        let Some((version, anchor)) = self.read(|s| (s.version, s.anchor)) else {
            return Anchor::default();
        };
        version.with(|_| ());
        anchor
    }

    /// The rows built (in view or prefetched).
    #[must_use]
    pub fn window(&self) -> std::ops::Range<u64> {
        let Some((version, w)) = self.read(|s| (s.version, s.window)) else {
            return 0..0;
        };
        version.with(|_| ());
        w.first..w.last
    }

    /// The rows' total height in pixels, measured and estimated.
    #[must_use]
    pub fn extent(&self) -> f64 {
        pixels(self.read(|s| s.extents.extent()).unwrap_or(0))
    }

    /// The top of `row` in pixels from the first row's top (the extent for `row == len`).
    #[must_use]
    pub fn offset_of(&self, row: u64) -> f64 {
        pixels(self.read(|s| s.extents.offset_of(row)).unwrap_or(0))
    }

    /// `row`'s height in pixels, measured or estimated; 0 past the end.
    #[must_use]
    pub fn row_height(&self, row: u64) -> f32 {
        let h = self.read(|s| s.extents.height(row)).flatten().unwrap_or(0);
        crate::convert::f64_to_f32(pixels(u64::from(h)))
    }

    /// Change measured `row`'s height by `delta` pixels without laying it
    /// out: its owner changed the room it holds (a gap added below it as a
    /// row comes in after it), and a row not built is not measured again
    /// until it is. A row not measured keeps its estimate.
    pub fn adjust_row_height(&self, row: u64, delta: f32) {
        let changed = self.write(|s| {
            if !s.extents.is_measured(row) {
                return false;
            }
            let h = s.extents.height(row).unwrap_or(0);
            let d = units(delta.abs());
            let next = if delta >= 0.0 {
                h.saturating_add(d)
            } else {
                h.saturating_sub(d)
            };
            if !s.extents.set(row, next) {
                return false;
            }
            note_measured(s, row, next);
            true
        });
        if changed == Some(true) {
            // The anchor holds its row: the content moved under the place.
            self.moved(Move::Rebase);
        }
    }

    /// Which rows are where, as a count bumped by each change to it: what
    /// [`Views`](super::Views) checks its view still describes.
    pub(crate) fn shape(&self) -> u64 {
        self.peek(|s| s.shape).unwrap_or(0)
    }

    /// Lend the heights to be reordered off the UI thread ([`Heights::reorder`],
    /// as [`Views`](super::Views) does with each sort), in O(1): a snapshot,
    /// shared, not copied. The rows measured from now on are noted for this
    /// loan, so [`Self::reorder_with`] adds them to what comes back. A loan
    /// voids the one before: only the latest is taken back. The loan has a
    /// holder of its own ([`Self::lend_heights_for`]).
    #[must_use]
    pub fn lend_heights(&self) -> Heights {
        self.lend_heights_for(crate::engine::unique_id(&HOLDERS))
    }

    /// [`Self::lend_heights`] for `holder`: the loan is its, and only its
    /// release ([`Self::release_heights`]) or the loan's own end takes it
    /// back, never another holder's.
    pub(crate) fn lend_heights_for(&self, holder: u64) -> Heights {
        self.write(|s| {
            let shape = s.shape;
            // A unique id, not a count the list keeps: a fork's loan is
            // named in the forks' space, so a loan it kept never matches
            // one lent live after it (ADR-0028, rule 3).
            let loan = crate::engine::unique_id(&LOANS);
            s.lent = Some(Lent { loan, shape, holder });
            s.since_lent.clear();
            Heights {
                extents: s.extents.clone(),
                loan,
                shape,
                holder,
            }
        })
        // Gone: heights no list takes back (no loan is live).
        .unwrap_or_else(|| Heights {
            extents: ExtentTree::default(),
            loan: 0,
            shape: 0,
            holder,
        })
    }

    /// Whether `holder` may lend the heights: it holds them already, or
    /// none does and it takes them. One holder at a time ([`Views`](super::Views)
    /// asking again when refused would else void another's loan in turn,
    /// for ever); quiet once the list is gone.
    pub(crate) fn claim_heights(&self, holder: u64) -> bool {
        self.poke(|s| match s.holder {
            None => {
                s.holder = Some(holder);
                true
            }
            Some(h) => h == holder,
        })
        .unwrap_or(false)
    }

    /// `holder`, refused the heights, is woken through `wake` once when
    /// they are let go, to ask again; in place of what it left before (the
    /// latest request it holds is what it asks).
    pub(crate) fn wait_for_heights(&self, holder: u64, wake: Waiter) {
        self.poke(|s| {
            // Copied here alone in a check's copy (each waiter a `Weak`).
            let waiting = Rc::make_mut(&mut s.waiting);
            waiting.retain(|(h, _)| *h != holder);
            waiting.push((holder, wake));
        });
    }

    /// `holder` lets the heights go: its claim, its own loan (only its: a
    /// loan another holder made, and its notes, stand), and its place among
    /// those waiting. If it held them, every holder waiting is woken to ask
    /// again, outside the list (they lend), and never in a fork: a waiter
    /// is a caller's closure (ADR-0028, rule 5), and what the fork let go
    /// is put back by its rollback.
    pub(crate) fn release_heights(&self, holder: u64) {
        let woken = self.poke(|s| {
            if s.lent.is_some_and(|l| l.holder == holder) {
                s.lent = None;
                s.since_lent.release();
            }
            // Written (so, in a check's copy, copied) only if it waits.
            if s.waiting.iter().any(|(h, _)| *h == holder) {
                Rc::make_mut(&mut s.waiting).retain(|(h, _)| *h != holder);
            }
            if s.holder != Some(holder) {
                return Vec::new();
            }
            s.holder = None;
            if crate::engine::forking() || s.waiting.is_empty() {
                return Vec::new();
            }
            std::mem::take(Rc::make_mut(&mut s.waiting))
        });
        for (_, wake) in woken.into_iter().flatten() {
            if let Some(wake) = wake.upgrade() {
                wake();
            }
        }
    }

    /// Whether a loan of the heights is live.
    #[cfg(test)]
    pub(crate) fn is_lent(&self) -> bool {
        self.read(|s| s.lent.is_some()).unwrap_or(false)
    }

    /// Set the heights by hand (tests).
    #[cfg(test)]
    pub(crate) fn heights_mut(&self, f: impl FnOnce(&mut ExtentTree)) {
        self.write(|s| f(&mut s.extents));
    }

    /// End loan `loan` if it is the live one: nothing is noted for it any
    /// more, and what it lent is not taken back. Another holder's loan is
    /// left alone. Quiet once the list is gone: a list's views may let go
    /// after it.
    pub(crate) fn end_loan(&self, loan: u64) {
        self.poke(|s| {
            if s.lent.is_some_and(|l| l.loan == loan) {
                s.lent = None;
                s.since_lent.release();
            }
        });
    }

    /// [`Self::reorder`] with the heights already reordered off the UI
    /// thread (`heights`, lent by [`Self::lend_heights`] and given
    /// [`Heights::reorder`] with the same `rows` and `to`): they are put in
    /// place in O(1) and the rows measured since the loan added (O(those
    /// rows)), allocating nothing once warm; the heights replaced come back
    /// (`Ok`) to be dropped off the UI thread.
    ///
    /// Taken back only from the live loan, and only while no row moved
    /// since it was made (no splice, no rows removed at the end, no reset,
    /// no reorder): else nothing changes, the loan ends if it was the live
    /// one, and `heights` come back (`Err`), to be dropped off the UI thread
    /// too. Heights are never moved by positions that no longer hold; the
    /// caller asks again.
    ///
    /// One transaction, as an optimistic one commits (Kung and Robinson,
    /// 1981): every call of `to` runs first, reading only the loan's own
    /// notes (each taken only while the loan is still the list's), and one
    /// write ([`Self::commit_reorder`], plain [`Self::reorder`]'s too) then
    /// validates everything the mapping could have changed — the loan, the
    /// shape, the rows measured, the anchor, the scroll asked for and the
    /// focused row — and puts the heights, the anchor and the focused row
    /// in place and ends the loan together; else it writes nothing but the
    /// loan's end. Rows `to` measures get one more pass; a `to` that still
    /// measures after it is refused, the list's heights as they are. A `to`
    /// that scrolls the list refuses the reorder: the scroll stands.
    pub fn reorder_with(
        &self,
        heights: Heights,
        rows: u64,
        to: impl Fn(u64) -> Option<u64>,
    ) -> Result<Heights, Heights> {
        let Heights {
            mut extents,
            loan,
            shape,
            holder,
        } = heights;
        let lent = Lent { loan, shape, holder };
        // The loan is ended if `to` panics: the heights it lent are gone
        // with the unwind, so nothing could take them back, and a loan
        // left live would note every row measured from then on, for ever
        // (Codex 6, F1 review 7). Ended as a refusal ends it.
        let _unwind = EndLoanOnUnwind { list: *self, loan };
        let refused = |extents: ExtentTree| {
            self.end_loan(loan);
            Err(Heights {
                extents,
                loan,
                shape,
                holder,
            })
        };
        let live = self.read(|s| s.lent == Some(lent) && s.shape == shape);
        if live != Some(true) || extents.len() != rows {
            return refused(extents);
        }
        // The read set, taken before any call of `to`.
        let Some(read) = self.read_set() else {
            return refused(extents);
        };
        // Prepare: the caller's `to` runs with no list borrowed (it may read
        // this list, measure its rows, lend anew or splice). The rows
        // measured since the loan, then those `to` measured, mapped; a
        // newer loan's notes are its own, never taken here.
        let mut spent = Vec::new();
        let mut settled = false;
        for pass in 0..3 {
            // Quiet: a list `to` disposed is a refusal, never a failure.
            let measured = self.poke(|s| (s.lent == Some(lent)).then(|| s.since_lent.take()));
            let Some(Some(measured)) = measured else {
                return refused(extents);
            };
            if measured.is_empty() {
                settled = true;
                break;
            }
            if pass == 2 {
                break;
            }
            for &(row, height) in &measured {
                if let Some(at) = to(row).filter(|&r| r < rows) {
                    extents.set(at, height);
                }
            }
            spent = measured;
        }
        if !settled {
            // Measured again after the second pass: bounded, refused.
            return refused(extents);
        }
        let follow = Self::follow_reorder(rows, &to, &read);
        match self.commit_reorder(Read::Loan(lent, spent), &read, extents, follow) {
            Ok(old) => Ok(Heights {
                extents: old,
                loan,
                shape,
                holder,
            }),
            Err(extents) => refused(extents),
        }
    }

    /// The one commit of a reorder, [`Self::reorder`]'s and
    /// [`Self::reorder_with`]'s: nothing of the caller's runs from here to
    /// the write, which validates the transaction's read set (Kung and
    /// Robinson, 1981) — what it read the heights from (`read`: the loan,
    /// its shape and that no row was measured unseen; or the list's own
    /// heights, unwritten since), and what the anchor and the focused row
    /// were worked out from (`seen`) — and writes the heights, the anchor
    /// and the focused row together, or nothing (`Err`: the heights it was
    /// given). `Ok`: the heights replaced.
    fn commit_reorder(
        &self,
        read: Read,
        set: &ReadSet,
        extents: ExtentTree,
        follow: Follow,
    ) -> Result<ExtentTree, ExtentTree> {
        // The window's focus, read outside the list's borrow.
        if crate::untracked(crate::focus::focused) != set.focused {
            return Err(extents);
        }
        let seen = &set.seen;
        // Given to the write only once it runs: a list the caller's `to`
        // disposed takes nothing, and the caller's heights come back whole
        // (Codex 4, F1 review 8), quietly (a disposed list is a refusal).
        let mut given = Some(extents);
        let committed = self.poke(|s| {
            let extents = given.take()?;
            let valid = match &read {
                Read::Loan(lent, _) => s.lent == Some(*lent) && s.shape == lent.shape && s.since_lent.is_empty(),
                Read::Own { heights, shape } => s.extents.same(heights) && s.shape == *shape,
            };
            if !valid || Seen::of(s) != *seen {
                return Some(Err(extents));
            }
            if let Read::Loan(_, spent) = read {
                s.lent = None;
                s.since_lent.recycle(spent);
            }
            let old = std::mem::replace(&mut s.extents, extents);
            reshaped(s);
            follow.apply(s);
            Some(Ok(old))
        });
        match committed.flatten() {
            Some(Ok(old)) => {
                // The anchor follows its row: the rows moved under the place shown.
                self.moved(Move::Rebase);
                Ok(old)
            }
            Some(Err(extents)) => Err(extents),
            None => Err(given.unwrap_or_default()),
        }
    }

    /// Whether `row`'s height was measured.
    #[must_use]
    pub fn is_measured(&self, row: u64) -> bool {
        self.read(|s| s.extents.is_measured(row)).unwrap_or(false)
    }

    /// The viewport's height as last laid out (a first guess before that).
    #[must_use]
    pub fn viewport_height(&self) -> f32 {
        self.read(viewport_height).unwrap_or(FIRST_VIEWPORT)
    }

    /// Runs its height tree holds (rows measured apart, not rows).
    #[must_use]
    pub fn runs(&self) -> usize {
        self.read(|s| s.extents.runs()).unwrap_or(0)
    }

    /// Build `px` pixels of rows beyond the viewport on each side (one
    /// viewport by default).
    pub fn set_prefetch(&self, px: f32) {
        self.write(|s| s.prefetch = Some(px.max(0.0)));
    }

    /// Keep the last row in view as rows are added while the view is at the end.
    pub fn follow_tail(&self, follow: bool) {
        let changed = self.write(|s| {
            if std::mem::replace(&mut s.follow, follow) == follow {
                return false;
            }
            if follow {
                s.pinned = true;
            }
            true
        });
        if changed == Some(true) {
            self.moved(Move::Rebase);
        }
    }

    /// Whether it follows its tail.
    #[must_use]
    pub fn follows_tail(&self) -> bool {
        self.read(|s| s.follow).unwrap_or(false)
    }

    /// Whether the view is at the end.
    #[must_use]
    pub fn is_at_end(&self) -> bool {
        let Some((version, pinned)) = self.read(|s| (s.version, s.pinned)) else {
            return false;
        };
        version.with(|_| ());
        pinned
    }

    /// Leave `px` pixels of room below the last row (so the last rows can
    /// scroll up to the top); 0 by default.
    pub fn set_end_space(&self, px: f32) {
        let px = px.max(0.0);
        if self.write(|s| std::mem::replace(&mut s.end_space, px) != px) == Some(true) {
            self.moved(Move::Rebase);
        }
    }

    /// The room below the last row, in pixels.
    #[must_use]
    pub fn end_space(&self) -> f32 {
        self.read(|s| s.end_space).unwrap_or(0.0)
    }

    /// Size the list by its rows (measured, the rows not built estimated),
    /// up to `max` pixels, as a box of `max-height` and `overflow: auto`
    /// does; `None`: it fills the height its parent gives it.
    pub fn fit_rows(&self, max: Option<f32>) {
        self.write(|s| s.fit = max);
    }

    /// Draw no scrollbar (`scrollbar-width: none`), or draw it again.
    pub fn hide_scrollbar(&self, hide: bool) {
        if let Some(no_scrollbar) = self.read(|s| s.no_scrollbar) {
            no_scrollbar.set(hide);
        }
    }

    /// Run `f` after each layout that leaves the window where it is (the
    /// rows built are the rows needed, placed as asked): what the owner of
    /// the list reads of where its rows lie. It returns whether it changed
    /// what the frame is built from, and then the frame is laid out again.
    /// A caller's closure, reached only through the gate: a fork never runs
    /// it (ADR-0028, rule 5) and asks for no pass again.
    pub fn on_layout(&self, f: impl Fn(&ListLayout) -> bool + 'static) {
        let hook: Hook = Rc::new(move |l: &ListLayout| {
            crate::engine::call_out(crate::engine::Site::ListLayout, || f(l)).unwrap_or(false)
        });
        // What it held is dropped once the state is let go.
        let _was = self.write(|s| s.hook.replace(hook));
    }

    /// The row its owner treats as current, kept built (and in what a
    /// screen reader holds) while it is scrolled out of view: an option
    /// highlighted by the keyboard (`aria-activedescendant`). Read when the
    /// list renders.
    pub fn set_active(&self, row: Option<u64>) {
        self.write(|s| s.active = row);
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
        if let Some(active_from) = self.read(|s| s.active_from) {
            active_from.hold(f);
        }
    }

    /// Keep `rows` rows on each side of the focused one built with it (0 by
    /// default), so Tab and Shift+Tab from a row scrolled away reach its
    /// neighbours, not the rows in view. Read when the list renders.
    pub fn keep_around_focus(&self, rows: u64) {
        self.write(|s| s.around_focus = rows);
    }

    /// The row holding focus, as last laid out.
    #[must_use]
    pub fn focused_row(&self) -> Option<u64> {
        self.read(|s| s.focus_row).flatten().map(|(row, _)| row)
    }

    /// The rows built outside the window, kept for focus and the active row.
    #[must_use]
    pub fn pinned(&self) -> Vec<u64> {
        let Some(version) = self.read(|s| s.version) else {
            return Vec::new();
        };
        version.with(|_| ());
        self.read(|s| s.built.pins.iter().map(|(row, _)| *row).collect())
            .unwrap_or_default()
    }

    /// Bring the row keyed `key` into view (a screen reader asked); whether
    /// it is a row of this list.
    fn reveal(&self, key: ElementKey) -> bool {
        let row = self
            .read(|s| row_keyed(s.window.first, &s.built.keys, &s.built.pins, key))
            .flatten();
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
        let seg = self
            .read(|s| {
                s.viewport_path.as_ref().and_then(|v| {
                    path.0
                        .starts_with(&v.0)
                        .then(|| path.0.get(v.0.len()).copied())
                        .flatten()
                })
            })
            .flatten();
        // A row the list keyed: as a node, or as a component's root.
        let Some(crate::geometry::PathSeg::Key(k) | crate::geometry::PathSeg::Component(k, _)) = seg else {
            return None;
        };
        // Built in the last frame: in its window or pinned.
        let row = self
            .read(|s| row_keyed(s.window.first, &s.built.keys, &s.built.pins, k))
            .flatten()
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
        let Some((active_from, active, around)) = self.read(|s| (s.active_from, s.active, s.around_focus)) else {
            return (None, Vec::new());
        };
        let from = active_from.get();
        let active = from.map_or(active, |f| f()).filter(|&r| r < rows).map(|r| (r, key(r)));
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
        self.jump_to(anchor, None);
    }

    /// Scroll so `row`'s bottom lies `above` pixels above the viewport's,
    /// at whatever height the row is measured.
    pub fn scroll_to_row_bottom(&self, row: u64, above: f32) {
        self.jump_to(
            Anchor {
                row,
                key: None,
                offset: 0.0,
            },
            Some(above),
        );
    }

    /// The one jump a program's scroll makes: the anchor and the bottom
    /// asked for (`above`, if any) put in one write, then one
    /// [`Move::Jump`] (an assignment, ADR-0032 I5), so the container's
    /// offset and the rows the next layout builds come from the same
    /// anchor and go out in the same commit.
    fn jump_to(&self, anchor: Anchor, above: Option<f32>) {
        self.write(|s| {
            s.pinned = false;
            s.to_end = false;
            ask(s, above);
            s.anchor = Anchor {
                row: anchor.row.min(s.extents.len().saturating_sub(1)),
                ..anchor
            };
        });
        self.moved(Move::Jump);
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
        pixels(self.read(|s| top_units_at(s, &s.extents, s.anchor)).unwrap_or(0))
    }

    /// Scroll to the end (and follow it if [`Self::follow_tail`] is on).
    pub fn scroll_to_end(&self) {
        self.write(|s| {
            s.request = None;
            s.pinned = true;
            s.to_end = true;
        });
        self.moved(Move::Jump);
    }

    /// Scroll so a `fraction` of the whole extent lies above the viewport.
    pub fn scroll_to_fraction(&self, fraction: f64) {
        self.write(|s| {
            let vp = u64::from(units(viewport_height(s)));
            let room = total(s, &s.extents).saturating_sub(vp);
            let top = crate::convert::f64_to_u64(fraction.clamp(0.0, 1.0) * crate::convert::u64_to_f64(room));
            let (row, into) = s.extents.row_at(top);
            s.request = None;
            s.pinned = fraction >= 1.0 && s.follow;
            s.to_end = fraction >= 1.0;
            s.anchor = Anchor {
                row,
                key: None,
                offset: crate::convert::f64_to_f32(pixels(into)),
            };
        });
        self.moved(Move::Jump);
    }

    /// Rows `at..at + removed` were replaced by `inserted` new ones (not
    /// yet measured); the anchor keeps its row.
    pub fn splice(&self, at: u64, removed: u64, inserted: u64) {
        self.write(|s| {
            s.extents.splice(at, removed, inserted);
            reshaped(s);
            let a = &mut s.anchor;
            if a.row >= at.saturating_add(removed) {
                a.row = a.row.saturating_sub(removed).saturating_add(inserted);
            } else if a.row >= at {
                a.row = at;
                a.offset = 0.0;
            }
            if s.window.last > at {
                // The rows built moved: the window is decided anew.
                s.window = Window::default();
            }
        });
        // The anchor holds its row: the rows moved under the place shown.
        self.moved(Move::Rebase);
    }

    /// The rows were reordered or filtered (a sort, a new
    /// [`View`](super::View)): it holds `rows` rows now, and the row at each
    /// old position `p` is at `to(p)` (`None`: left out;
    /// [`View::moved_to`](super::View::moved_to) gives it). The heights
    /// measured follow their rows, the rows new to it take the estimate,
    /// and the anchor and the focused row follow theirs (a row left out
    /// leaves the anchor at its old index, its top). O(m log m) for the
    /// `m` rows measured, whatever the rows, on the calling thread; the
    /// heights it replaces are dropped there too, in O(m), less than the
    /// reorder itself ([`Self::reorder_with`] does both off the UI thread).
    ///
    /// One transaction with [`Self::reorder_with`]'s commit: every call of
    /// `to` runs first, then one write validates that nothing it read
    /// changed — the heights (unwritten since: no row measured, spliced,
    /// added or removed), the anchor, the scroll asked for and the focused
    /// row — and writes all of it, or nothing. `false`: refused, as a `to`
    /// that writes the list (scrolls it, splices it) is; what it wrote
    /// stands, and the caller asks again.
    pub fn reorder(&self, rows: u64, to: impl Fn(u64) -> Option<u64>) -> bool {
        // The caller's mapping runs with no list borrowed (it may read this
        // list, or make another): the heights are a persistent tree, taken
        // out in O(1), reordered, put back.
        // Everything read first, before any call of `to`.
        let Some((heights, shape)) = self.read(|s| (s.extents.clone(), s.shape)) else {
            return false;
        };
        let Some(read) = self.read_set() else {
            return false;
        };
        let mut extents = heights.clone();
        extents.reorder(rows, &to);
        let follow = Self::follow_reorder(rows, &to, &read);
        self.commit_reorder(Read::Own { heights, shape }, &read, extents, follow)
            .is_ok()
    }

    /// Where the anchor and the focused row go after the rows moved (`to`,
    /// as [`Self::reorder`] takes it), worked out with nothing written, and
    /// what it was worked out from: the caller's `to` runs here, before the
    /// one write that validates that and applies it.
    fn follow_reorder(rows: u64, to: &dyn Fn(u64) -> Option<u64>, read: &ReadSet) -> Follow {
        let (a, focus) = (read.seen.anchor, read.focus);
        let anchor = match to(a.row).filter(|&r| r < rows) {
            Some(row) => Anchor { row, ..a },
            None => Anchor {
                row: a.row.min(rows.saturating_sub(1)),
                key: None,
                offset: 0.0,
            },
        };
        let focus_row = focus.and_then(|(r, k)| to(r).filter(|&r| r < rows).map(|r| (r, k)));
        Follow { anchor, focus_row }
    }

    /// What a reorder reads of the list, taken before any call of the
    /// caller's `to` (Kung and Robinson: the read set is recorded as the
    /// read phase starts), so a `to` that writes the list changes it and
    /// the commit refuses.
    fn read_set(&self) -> Option<ReadSet> {
        let (window, focus_row, seen) = self.read(|s| (s.window, s.focus_row, Seen::of(s)))?;
        // What holds focus now (moved this turn or not), found by its key
        // among the rows built while they are still known by position.
        let focused = crate::untracked(crate::focus::focused);
        let focus = self.noted_focus(window, &[]).or(focus_row);
        Some(ReadSet { seen, focus, focused })
    }

    fn bump(&self) {
        if let Some(version) = self.read(|s| s.version) {
            version.update(|v| *v = v.wrapping_add(1));
        }
    }

    /// Where the view is to be changed (a scroll asked for, rows moved
    /// under it): the scroll container is put where the window the next
    /// render builds has it, and the list renders again. `kind` is what
    /// the change is to the reader ([`Move`]): a place asked for (every
    /// `scroll_to*`) is a jump; rows moved, spliced, reordered or measured
    /// under the place shown, a window built again, a list held at its
    /// end are rebases, which a gesture and its fling go on through.
    fn moved(&self, kind: Move) {
        self.place(self.compute(), kind);
        self.bump();
    }

    /// Put the scroll container where window `w` shows the viewport (at
    /// the end, as far as the rows reach once measured: the container
    /// clamps it), as `kind` says.
    fn place(&self, w: Window, kind: Move) {
        let Some((end, scroll)) = self.read(|s| (wants_end(s), s.scroll)) else {
            return;
        };
        let local = if end { f32::MAX } else { w.local };
        scroll.keep_at((0.0, local), kind);
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
        let reach = self.scroll().map_or(0.0, |h| h.reach().1);
        self.read(|s| compute_at(s, &s.extents, s.anchor, reach))
            .unwrap_or_default()
    }

    /// After a layout: measure the rows built, move the anchor to where the
    /// viewport is, and whether the window must move (another pass).
    /// `scroll` is its scroll container's handle (its own for life).
    fn laid_out(&self, scroll: ScrollHandle, l: &ScrollLayout, built: Option<&crate::scroll::Built>) -> bool {
        if let Some(b) = built {
            let _moved = self.take(b);
        }
        let width = l.bounds.width;
        let height = (l.bounds.height - l.padding.top - l.padding.bottom).max(0.0);
        // The anchor: the row at the viewport's top, unless one asked for is
        // not shown yet; the reader's scroll since it was asked is on top
        // of it already ([`Self::poke`], ADR-0032 I5: a jump is an
        // assignment, the reader's scroll after it adds to it).
        let y = l.offset.1 + l.padding.top;
        let measured = self.write(|s| {
            let (old_width, _) = s.viewport;
            s.viewport = (width, height);
            if s.viewport_path.as_ref() != Some(&l.path) {
                s.viewport_path = Some(l.path.clone());
            }
            s.top = l.bounds.y;
            if old_width > 0.0 && old_width != width {
                // Heights measured at another width: estimates again.
                s.extents.reset();
                reshaped(s);
            }
            let w = s.window;
            let built = usize::try_from(w.last.saturating_sub(w.first)).unwrap_or(usize::MAX);
            // The rows, not the pinned rows around them or the room below them.
            let before = s.built.pins_before.min(l.children.len());
            let rows = before..before.saturating_add(built).min(l.children.len());
            let children = l.children.get(rows.clone()).unwrap_or_default();
            // The row holding focus, by its key, built in the window or pinned.
            let focus = l
                .children
                .iter()
                .find(|c| c.focus_within)
                .and_then(|c| c.key)
                .and_then(|k| row_keyed(w.first, &s.built.keys, &s.built.pins, k).map(|r| (r, k)));
            let held = s.focus_row;
            let unpinned = held.is_some_and(|(r, _)| !(w.first..w.last).contains(&r)) && focus != held;
            s.focus_row = focus;
            if unpinned {
                return Measured::Unpinned;
            }
            // The rows not built take the mean of those measured, which the
            // heights keep themselves ([`ExtentTree::estimate`]).
            for (row, c) in (w.first..w.last).zip(children) {
                let height = units(c.bounds.height);
                if s.extents.set(row, height) {
                    note_measured(s, row, height);
                }
            }
            let asked = s.request;
            if asked.is_none() {
                let at = children
                    .iter()
                    .enumerate()
                    .find(|(_, c)| c.bounds.y + c.bounds.height > y)
                    .or_else(|| children.iter().enumerate().next_back());
                if let Some((j, c)) = at {
                    let key = s.built.keys.get(j).copied();
                    s.anchor = Anchor {
                        row: w.first.saturating_add(u64::try_from(j).unwrap_or(u64::MAX)),
                        key,
                        offset: (y - c.bounds.y).max(0.0),
                    };
                }
            }
            Measured::Rows { w, asked, rows }
        });
        let (w, asked, rows) = match measured {
            None => return false,
            Some(Measured::Unpinned) => {
                // Focus left the row kept built for it: it goes.
                self.bump();
                return true;
            }
            Some(Measured::Rows { w, asked, rows }) => (w, asked, rows),
        };
        let children = l.children.get(rows).unwrap_or_default();
        let (_, reach) = scroll.reach();
        // One step for what the layout settles: run every layout pass of
        // every list, it reads the state once.
        let Some(settled) = self.write(|s| {
            let at_end = l.max.1 - l.offset.1 <= 0.5 && w.last >= s.extents.len();
            let wants_end = wants_end(s);
            let settled = Settled {
                thumb: (s.thumb, thumb_geometry(s, l.offset.1, height)),
                fit: s.fit.map(|max| {
                    let rows = crate::convert::f64_to_f32(pixels(total(s, &s.extents))).min(max);
                    (s.height, rows)
                }),
                at_end,
                wants_end,
                next: Window::default(),
            };
            s.pinned = at_end;
            if at_end {
                s.to_end = false;
            }
            Settled {
                next: compute_at(s, &s.extents, s.anchor, reach),
                ..settled
            }
        }) else {
            return false;
        };
        let thumb_moved = set_changed(settled.thumb.0, settled.thumb.1);
        let fitted = settled.fit.is_some_and(|(signal, h)| set_changed(signal, Some(h)));
        let (at_end, wants_end, next) = (settled.at_end, settled.wants_end, settled.next);
        let placed = if wants_end {
            at_end
        } else {
            (next.local - l.offset.1).abs() <= PLACED
        };
        if next.first != w.first || next.last != w.last || !placed {
            // The next render builds this window, put where it shows the
            // viewport.
            self.write(|s| s.window = next);
            self.place(next, Move::Rebase);
            self.bump();
            return true;
        }
        // The rows built still cover it: nothing to build.
        let hook = self
            .write(|s| {
                s.window = next;
                if s.request.map(|r| r.rev) == asked.map(|r| r.rev) {
                    s.request = None;
                }
                let origin = pixels(s.extents.offset_of(w.first));
                s.hook.clone().map(|f| (f, origin, pixels(s.extents.extent())))
            })
            .flatten();
        let owner = hook.is_some_and(|(f, origin, extent)| {
            f(&ListLayout {
                width,
                viewport: height,
                top: origin + f64::from(l.offset.1),
                extent,
                rows: (w.first..w.last)
                    .zip(children)
                    .map(|(row, c)| RowLayout {
                        row,
                        top: c.bounds.y - y,
                        height: c.bounds.height,
                    })
                    .collect(),
                wheeled: scroll.take_wheeled(),
            })
        });
        thumb_moved | fitted | owner
    }

    /// The anchor kept on its key after the data moved under it, over
    /// `len` rows; `None` when it stays where it is.
    fn followed(&self, len: u64, key: &dyn Fn(u64) -> ElementKey) -> Option<Anchor> {
        let a = self.read(|s| s.anchor)?;
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
        self.write(|s| {
            let len = s.extents.len();
            if len != rows {
                resize(&mut s.extents, rows);
                // Rows added at the end move none; rows removed there go.
                if rows < len {
                    reshaped(s);
                }
            }
        });
    }

    /// What a render builds from the state as it stands and the data it is
    /// given, `rows` rows keyed `key`: worked out, nothing written (the
    /// layout hook takes it in, [`Self::take`]).
    fn plan(&self, rows: u64, key: &dyn Fn(u64) -> ElementKey) -> Plan {
        let followed = self.followed(rows, key);
        let reach = self.scroll().map_or(0.0, |h| h.reach().1);
        let (window, total, focus_row, rev) = self
            .read(|s| {
                // The heights as the data has them: rows added or removed at the end.
                let resized = (s.extents.len() != rows).then(|| {
                    let mut ext = s.extents.clone();
                    resize(&mut ext, rows);
                    ext
                });
                let ext = resized.as_ref().unwrap_or(&s.extents);
                let window = compute_at(s, ext, followed.unwrap_or(s.anchor), reach);
                (window, total(s, ext), s.focus_row, s.requests)
            })
            .unwrap_or_default();
        let keys: Vec<ElementKey> = (window.first..window.last).map(key).collect();
        let focus = self.noted_focus(window, &keys).or(focus_row);
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
            rev,
        }
    }

    /// Take in what the render built from, once (the plan is known by its
    /// own allocation, which the `Weak` held keeps from being reused, so a
    /// plan taken again changes nothing): the heights matched to its data,
    /// the anchor kept on its key, the rows it built. Taken after every
    /// layout pass that laid them out, the frame's last too. Whether it put
    /// the container elsewhere.
    fn take(&self, built: &crate::scroll::Built) -> bool {
        let fresh = self.write(|s| {
            if Weak::ptr_eq(&s.taken, &Rc::downgrade(built)) {
                return false;
            }
            s.taken = Rc::downgrade(built);
            true
        });
        if fresh != Some(true) {
            return false;
        }
        let Some(p) = built.downcast_ref::<Plan>() else {
            return false;
        };
        let Some(scroll) = self.scroll() else {
            return false;
        };
        let was = scroll.offset();
        self.sync_len(p.rows);
        // A scroll asked for since the render (a later effect's) is newer
        // than the anchor the plan followed: it wins, and the list renders
        // again for it.
        let current = self.read(|s| s.requests == p.rev).unwrap_or(false);
        if let Some(a) = p.followed.filter(|_| current) {
            // The reader's row moved to another index under them: the
            // container goes where the plan put that row, and the anchor
            // holds until the viewport shows it there (the hook reads no
            // other row from a layout at the old offset).
            self.write(|s| {
                s.anchor = a;
                ask(s, None);
            });
            self.place(p.window, Move::Rebase);
        }
        self.write(|s| {
            s.window = p.window;
            Built::set(&mut s.built, &p.keys, &p.pins, p.pins_before);
            s.focus_row = p.focus;
        });
        scroll.offset() != was
    }

    /// Show the thumb, and hide it [`THUMB_SHOWN_MS`] after the last call:
    /// each call moves the deadline, and one timer at most is out for it
    /// (a wheel at 120 Hz queues one, not one an event).
    fn show_thumb(&self) {
        let shown_for = std::time::Duration::from_millis(THUMB_SHOWN_MS);
        let deadline = crate::clock::current_time().saturating_add(shown_for);
        let Some((shown, out)) = self.write(|s| {
            s.hide_at = deadline;
            (s.thumb_shown, std::mem::replace(&mut s.hiding, true))
        }) else {
            return;
        };
        if !shown.get_untracked() {
            shown.set(true);
        }
        if !out {
            self.hide_thumb_after(shown_for);
        }
    }

    /// The one timer hiding the thumb: at the latest deadline, or, moved
    /// since it was set, again for what is left of it.
    fn hide_thumb_after(&self, wait: std::time::Duration) {
        let me = *self;
        crate::clock::after(wait, move || {
            let now = crate::clock::current_time();
            let Some((shown, left, dragging)) = me.peek(|s| (s.thumb_shown, s.hide_at.saturating_sub(now), s.dragging))
            else {
                return;
            };
            if left > std::time::Duration::ZERO {
                me.hide_thumb_after(left);
                return;
            }
            me.write(|s| s.hiding = false);
            if !dragging {
                shown.set(false);
            }
        });
    }

    /// Move the thumb to put its top at `y` (window pixels).
    fn drag_thumb(&self, e: &DragEvent) {
        self.write(|s| s.dragging = !e.phase.is_last());
        self.show_thumb();
        match e.phase {
            DragPhase::Start => {
                self.write(|s| s.grab = e.y - e.bounds.y);
            }
            // Cancelled, the list stays where the moves put it.
            DragPhase::Cancel => {}
            DragPhase::Move | DragPhase::End => {
                let fraction = self.read(|s| {
                    let (_, vp) = s.viewport;
                    let thumb = thumb_height(vp, pixels(total(s, &s.extents)));
                    let track = (vp - thumb - 2.0 * THUMB_INSET).max(1.0);
                    let top = e.y - s.grab - s.top - THUMB_INSET;
                    f64::from((top / track).clamp(0.0, 1.0))
                });
                if let Some(fraction) = fraction {
                    self.scroll_to_fraction(fraction);
                }
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
    let Some((version, end_space, scroll, no_scrollbar, fit)) =
        state.read(|s| (s.version, s.end_space, s.scroll, s.no_scrollbar, s.fit))
    else {
        // Its owner ended: it holds no rows.
        return div().named("virtual-list");
    };
    // Read so a moved window renders again.
    version.with(|_| ());
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
    let scroller = scroller.track_scroll_built(&scroll, Rc::new(plan));
    // The thumb's place and size are bound to the layout (ADR-0013): set
    // after each layout from the anchor over the whole extent, they move the
    // thumb without building the list again.
    let s = *state;
    // The bindings read through the handle, not the signals it holds: a
    // binding may run once more after the list's owner ended (its element
    // still in a tree being replaced), and a list gone is inert.
    let geometry = move || s.peek(|st| st.thumb).and_then(|t| t.get());
    let shown = move || s.peek(|st| st.thumb_shown).is_some_and(|t| t.get());
    let thumb = (!no_scrollbar.get()).then(|| {
        div()
            .named("virtual-list-thumb")
            .position(Position::Absolute)
            .right(THUMB_RIGHT)
            .width(THUMB_WIDTH)
            .radius(THUMB_WIDTH / 2.0)
            .background_image(thumb_ink())
            .top(move || geometry().map_or(0.0, |(y, _)| y))
            .height(move || geometry().map_or(0.0, |(_, h)| h))
            // Shown at once, faded out.
            .opacity(move || if shown() { 1.0 } else { 0.0 })
            .duration_ms(move || if shown() { 0.0 } else { THUMB_FADE_MS })
            .ease(Easing::Linear)
            .transition(crate::motion::Properties::OPACITY)
            .display(move || {
                if geometry().is_some() {
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
    match fit {
        Some(max) => {
            // Its rows' height so far; measured after layout, bound there.
            // Before its first layout, from the heights it starts with.
            let rows_h = crate::convert::f64_to_f32(pixels(total)).min(max);
            list.flex(Flex::NONE)
                .height(move || s.peek(|st| st.height).and_then(|h| h.get()).unwrap_or(rows_h))
        }
        None => list,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Scenario;
    use crate::element::text;
    use crate::inspect::OUTSIDE;
    use crate::layout::Content;
    use std::cell::Cell;

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
                    Some((row, n.bounds(OUTSIDE).y, n.bounds(OUTSIDE).y + n.bounds(OUTSIDE).height))
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
        let st = *state;
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
        let st = state;
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
        assert_eq!(s.read(crate::focus::focused), Some(h.id()), "row 3 keeps focus");
        assert_eq!(state.focused_row(), Some(3), "kept built, pinned");
    }

    /// Focus on a row of another list keyed as one of this list's rows
    /// pins nothing here: only a node under this list's container is its.
    #[test]
    fn focus_in_a_sibling_list_pins_no_row_here() {
        let b = ListState::new(30.0);
        let h = crate::FocusHandle::new();
        let (sa, sb) = (ListState::new(30.0), b);
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
        assert_eq!(s.read(crate::focus::focused), Some(h.id()));
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
        state.read(|s| s.thumb_shown).is_some_and(|shown| shown.get_untracked())
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

    /// Many wheel notifications while the thumb shows keep one timer out
    /// to hide it (moved on to the last deadline), not one per wheel.
    #[test]
    fn many_wheels_keep_one_hide_timer() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        s.hover(100.0, 200.0);
        s.wheel(10.0);
        s.settle().unwrap();
        assert!(state.read(|s| s.hiding).unwrap_or(false), "a timer is out");
        let one = crate::clock::pending_timers();
        for _ in 0..20 {
            s.wheel(10.0);
            s.settle().unwrap();
        }
        assert_eq!(crate::clock::pending_timers(), one, "still one for twenty more wheels");
    }

    /// `scroll_to_row_bottom` is one jump (ADR-0032 I5): one assignment,
    /// its generation counted once, however many writes it makes, and the
    /// list lands with the row's bottom at the viewport's.
    #[test]
    fn a_scroll_to_a_rows_bottom_is_one_jump() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        s.settle().unwrap();
        let h = state.scroll().expect("a scroll container");
        let before = h.generation();
        s.act("row 40's bottom at the viewport's", || {
            state.scroll_to_row_bottom(40, 0.0)
        });
        assert_eq!(h.generation(), before + 1, "one jump");
        s.settle().unwrap();
        assert_eq!(h.generation(), before + 1, "landing on it is no other");
        let vp = f64::from(state.viewport_height());
        let bottom = state.offset_of(40) + f64::from(state.row_height(40));
        assert!(
            (state.view_top() + vp - bottom).abs() < 0.5,
            "row 40's bottom at the viewport's: {} + {vp} vs {bottom}",
            state.view_top()
        );
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
        let st = state;
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
        let st = *state;
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

    /// Wheels turning fast, the last turn not yet taken by the UI, when
    /// the program jumps far (ADR-0032 I5, Codex r7 #2): the turn lands on
    /// the jump, and the rows built are those around where the list lands,
    /// the viewport's offset and its rows from the same anchor in the same
    /// commit (every frame checked against a cold one).
    #[test]
    fn a_jump_during_fast_wheels_lands_them_on_it_with_its_rows() {
        let built = |s: &Scenario| rows(s).into_iter().map(|r| r.0).collect::<Vec<_>>();
        let calm = ListState::new(30.0);
        let mut r = million(&calm);
        r.settle().unwrap();
        r.act("far", || calm.scroll_to_row(5_000));
        r.frame("jumped").unwrap();
        let from_rest = built(&r);
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
        let fast = built(&s).len();
        assert!(
            fast > from_rest.len(),
            "a fast scroll's reach builds rows past the viewport's margin"
        );
        wheel(&mut s);
        s.act("far", || state.scroll_to_row(5_000));
        s.frame("jumped").unwrap();
        s.settle().unwrap();
        let landed = state.offset_of(5_000) + 80.0;
        assert_eq!(state.view_top(), landed, "the turn on the jump");
        let shown = built(&s);
        let (first, last) = (shown[0], shown[shown.len() - 1]);
        assert!(
            first <= 5_003 && last > 5_003 && shown.len() <= fast,
            "the rows around where it landed: {first}..={last}"
        );
        // 80 px into rows of 24: row 5,003 holds the viewport's top.
        assert_eq!(top(&s), 5_003, "the viewport's first row");
    }

    /// A list with a button above it whose click writes the list (`edit`),
    /// as a live list's data changes in an input's handler.
    fn edited(state: &ListState, edit: impl Fn(ListState) + 'static) -> Scenario {
        let st = *state;
        let edit = std::rc::Rc::new(edit);
        Scenario::measuring(
            move || {
                let edit = std::rc::Rc::clone(&edit);
                div()
                    .width(300.0)
                    .child(div().height(40.0).on_click(move |_| edit(st)).child(text("Edit")))
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
            340,
            1.0,
        )
        .unwrap()
    }

    /// Opus r8 I2: a wheel the compositor scrolled, taken in before a
    /// click's handler, which splices rows far below: the splice moves
    /// nothing under the place shown, so the wheel stays (ADR-0032 I5: a
    /// rebase is additive). The input's take lays the list out before the
    /// handler runs, so this path kept the wheel before the anchor took
    /// it in at every write too; the frame's take does not (the test
    /// after).
    #[test]
    fn a_splice_in_a_handler_after_a_take_keeps_the_wheel() {
        let state = ListState::new(30.0);
        let mut s = edited(&state, |l| l.splice(500_000, 0, 3));
        s.settle().unwrap();
        s.hover(150.0, 200.0);
        s.settle().unwrap();
        assert!(s.wheel(80.0));
        let down = s.queue(crate::window::Input::MouseDown { x: 150.0, y: 20.0 });
        let up = s.queue(crate::window::Input::MouseUp { x: 150.0, y: 20.0 });
        s.handle(down);
        s.handle(up);
        assert_eq!(state.len(), 1_000_003, "spliced");
        s.settle().unwrap();
        s.settle().unwrap();
        assert_eq!(state.view_top(), 80.0, "the wheel kept");
    }

    /// Opus r8 I2: a live-sorted list re-sorted in each input's handler,
    /// and in each frame by another list's layout hook laid out above it,
    /// while the reader wheels it: every wheel moves it on.
    #[test]
    fn a_list_re_sorted_as_it_is_wheeled_scrolls_on() {
        let state = ListState::new(30.0);
        let mut s = edited(&state, |l| {
            assert!(l.reorder(1_000_000, Some));
        });
        s.settle().unwrap();
        for k in 1..=5_u8 {
            s.hover(150.0, 200.0);
            assert!(s.wheel(30.0));
            s.click(150.0, 20.0);
            s.settle().unwrap();
            s.settle().unwrap();
            assert_eq!(state.view_top(), 30.0 * f64::from(k), "wheel {k} moved it on");
        }
        let (a, b) = (ListState::new(30.0), ListState::new(30.0));
        b.on_layout(move |_| {
            a.reorder(1_000_000, Some);
            false
        });
        let row = |i: u64| div().height(24.0).child(text(format!("row {i}"))).into_element();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(virtual_list(&b, 10, |i| i, row).height(100.0))
                    .child(virtual_list(&a, 1_000_000, |i| i, row).height(200.0))
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.hover(150.0, 200.0);
        s.settle().unwrap();
        for k in 1..=5_u8 {
            assert!(s.wheel(30.0));
            s.settle().unwrap();
            assert_eq!(a.view_top(), 30.0 * f64::from(k), "frame {k} moved it on");
        }
    }

    /// Opus r8 I2, on the frame's take: a jump to row 5,000 asked while a
    /// wheel the UI has not taken waits; the frame takes the wheel, and
    /// before the list's own layout hook runs, another list's hook (laid
    /// out above it) splices rows far below its place. The splice is a
    /// rebase: the wheel still lands on the jump.
    #[test]
    fn a_splice_before_the_lists_hook_keeps_the_wheel_taken_on_its_jump() {
        let (a, b) = (ListState::new(30.0), ListState::new(30.0));
        let splice = Rc::new(Cell::new(false));
        let asked = Rc::clone(&splice);
        b.on_layout(move |_| {
            if asked.take() {
                a.splice(900_000, 0, 1);
            }
            false
        });
        let row = |i: u64| div().height(24.0).child(text(format!("row {i}"))).into_element();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(virtual_list(&b, 10, |i| i, row).height(100.0))
                    .child(virtual_list(&a, 1_000_000, |i| i, row).height(200.0))
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.hover(150.0, 200.0);
        s.settle().unwrap();
        assert!(s.wheel(80.0));
        s.act("far", || a.scroll_to_row(5_000));
        splice.set(true);
        s.settle().unwrap();
        s.settle().unwrap();
        assert!(!splice.get(), "spliced in the frame");
        assert_eq!(a.view_top(), a.offset_of(5_000) + 80.0, "the wheel on the jump");
    }

    /// The list's render writes nothing: the window it builds is taken in
    /// by its layout hook, so the cold check's render of every instance
    /// (which asserts no live write) leaves it as it was, through a scroll
    /// asked for, the wheel, rows added and a followed tail.
    #[test]
    fn a_list_renders_purely_under_the_cold_check() {
        let state = ListState::new(30.0);
        let n = Signal::new(1_000_000_u64);
        let st = state;
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
        let st = *state;
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
        let st = state;
        let mut s = Scenario::measuring(
            move || {
                let v = Rc::new(ids.get());
                let (k, r) = (Rc::clone(&v), Rc::clone(&v));
                let at = move |v: &[u64], i: u64| usize::try_from(i).ok().and_then(|i| v.get(i)).copied().unwrap_or(0);
                let asker = go.get().then(|| {
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
                .map(|n| n.bounds(OUTSIDE).width)
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
        let st = state;
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
                .map(|n| n.bounds(OUTSIDE).y)
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
        let st = state;
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
        let st = state;
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
        let st = state;
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

    /// Codex (M15 review 1): a sort applied on the UI thread costs what
    /// changed since the heights were lent, not the rows measured: the
    /// heights reordered off the thread go in place, and the rows measured
    /// meanwhile are added to them.
    #[test]
    fn heights_reordered_off_the_ui_thread_go_in_place_with_what_was_measured_since() {
        let state = ListState::new(30.0);
        let n = 100_000_u64;
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(n, units(30.0));
            for r in 0..n {
                ext.set(r, units(if r.is_multiple_of(2) { 20.0 } else { 21.0 }));
            }
        });
        let reverse = move |p: u64| Some(n - 1 - p);
        let mut lent = state.lend_heights();
        state.adjust_row_height(4, 10.0);
        // The worker's part.
        lent.reorder(n, reverse);
        let before = crate::sum_tree::visits();
        let replaced = state.reorder_with(lent, n, reverse);
        let seen = crate::sum_tree::visits() - before;
        assert!(replaced.is_ok(), "put in place, the old heights handed back");
        assert!(seen < 2_000, "{seen} entries looked at");
        assert_eq!(state.row_height(n - 1 - 4), 30.0, "measured since the loan");
        assert_eq!(state.row_height(n - 1 - 5), 21.0);
    }

    /// Three rows measured A 10, B 20, C 30; one of them sorted into C, B
    /// is still being built when its owner splices A out: B and C keep
    /// their heights, the result is refused, not moved by places that no
    /// longer hold (Codex, M15 review 2).
    #[test]
    fn heights_lent_before_a_splice_are_refused_and_rows_keep_theirs() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(3, units(30.0));
            for (row, h) in [(0, 10.0), (1, 20.0), (2, 30.0)] {
                ext.set(row, units(h));
            }
        });
        // [A, B, C] sorted to [C, B]: A left out.
        let to = |p: u64| match p {
            1 => Some(1),
            2 => Some(0),
            _ => None,
        };
        let mut lent = state.lend_heights();
        state.splice(0, 1, 0);
        lent.reorder(2, to);
        assert!(state.reorder_with(lent, 2, to).is_err(), "refused");
        assert_eq!((state.row_height(0), state.row_height(1)), (20.0, 30.0), "B, C");
    }

    /// Loan A, a row measured 20 then 30, loan B, then A comes back: A is
    /// void, so the 30 measured since stays (Codex, M15 review 2).
    #[test]
    fn a_loan_voided_by_a_newer_one_is_not_taken_back() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            ext.set(0, units(20.0));
        });
        let mut a = state.lend_heights();
        state.adjust_row_height(0, 10.0);
        let b = state.lend_heights();
        a.reorder(10, Some);
        assert!(state.reorder_with(a, 10, Some).is_err(), "A is void");
        assert_eq!(state.row_height(0), 30.0);
        assert!(state.reorder_with(b, 10, Some).is_ok(), "B is live");
        assert_eq!(state.row_height(0), 30.0, "noted for B, kept");
    }

    /// The caller's mapping runs with no list borrowed (Codex, F1 review
    /// 1): one that reads the list and makes another, given to `reorder`
    /// and to `reorder_with` with a row measured since the loan, runs.
    #[test]
    fn a_reorder_mapping_may_read_the_list_and_make_another() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            ext.set(2, units(20.0));
        });
        let made = std::cell::RefCell::new(Vec::new());
        let reverse = |p: u64| {
            let n = state.len();
            made.borrow_mut().push(ListState::new(10.0));
            n.checked_sub(1).and_then(|last| last.checked_sub(p))
        };
        let mut lent = state.lend_heights();
        state.adjust_row_height(2, 10.0);
        lent.reorder(10, reverse);
        assert!(state.reorder_with(lent, 10, reverse).is_ok());
        assert_eq!(state.row_height(7), 30.0, "the row measured since, mapped");
        state.reorder(10, reverse);
        assert_eq!(state.row_height(2), 30.0, "and back");
        assert!(made.borrow().iter().all(|l| l.is_alive()));
        for l in made.take() {
            l.dispose();
        }
    }

    /// A loan a fork made and kept past its rollback, handed back after a
    /// live loan at the same shape, is refused, and the live one stands
    /// (Codex, F1 review 2): loans are named by ids the rollback does not
    /// give out again.
    #[cfg(debug_assertions)]
    #[test]
    fn a_loan_a_fork_kept_is_refused_after_a_live_one() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| *ext = ExtentTree::new(10, units(30.0)));
        let mut kept = {
            let _cold = crate::engine::ColdFrame::begin();
            state.lend_heights()
        };
        let live = state.lend_heights();
        assert_ne!(kept.loan(), live.loan());
        kept.reorder(10, Some);
        assert!(state.reorder_with(kept, 10, Some).is_err(), "the fork's loan");
        assert!(state.reorder_with(live, 10, Some).is_ok(), "the live one stands");
    }

    /// A mapping that lends the heights anew makes the call it runs in
    /// refused, and the newer loan stands (Codex, F1 review 2): checked
    /// where the heights are put in place, after the mapping.
    #[test]
    fn a_mapping_that_lends_anew_is_refused_and_the_newer_loan_stands() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            ext.set(3, units(30.0));
        });
        let newer = std::cell::RefCell::new(None);
        let lend = |p: u64| {
            if newer.borrow().is_none() {
                *newer.borrow_mut() = Some(state.lend_heights());
            }
            Some(p)
        };
        let mut lent = state.lend_heights();
        state.adjust_row_height(3, 5.0);
        lent.reorder(10, Some);
        assert!(state.reorder_with(lent, 10, lend).is_err(), "superseded within");
        let mut newer = newer.take().expect("lent within");
        newer.reorder(10, Some);
        assert!(state.reorder_with(newer, 10, Some).is_ok(), "the newer loan stands");
        assert_eq!(state.row_height(3), 35.0);
    }

    /// Loan A's mapping lends B and measures row 3 again, 35 to 40 px
    /// (Codex 1 and Opus, F1 review 3): A never takes B's notes, so A is
    /// refused and B, taken back, keeps the 40.
    #[test]
    fn a_newer_loans_notes_are_never_taken_by_an_older_one() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            ext.set(3, units(30.0));
        });
        let newer = std::cell::RefCell::new(None);
        let lend = |p: u64| {
            if newer.borrow().is_none() {
                *newer.borrow_mut() = Some(state.lend_heights());
                state.adjust_row_height(3, 5.0);
            }
            Some(p)
        };
        let mut a = state.lend_heights();
        state.adjust_row_height(3, 5.0);
        a.reorder(10, Some);
        assert!(state.reorder_with(a, 10, lend).is_err(), "A, superseded");
        let mut b = newer.take().expect("lent within");
        b.reorder(10, Some);
        assert!(state.reorder_with(b, 10, Some).is_ok(), "B stands");
        assert_eq!(state.row_height(3), 40.0, "measured for B, kept");
    }

    /// A splice while the anchor is mapped refuses the reorder, and the
    /// splice's anchor stands; a splice while the rows measured are mapped
    /// refuses it and ends the loan, so nothing is noted for it after
    /// (Codex 2, F1 review 3): one commit, after every call of the mapping.
    #[test]
    fn a_splice_inside_either_mapping_refuses_the_reorder() {
        for in_measured in [false, true] {
            let state = ListState::new(30.0);
            state.heights_mut(|ext| {
                *ext = ExtentTree::new(10, units(30.0));
                ext.set(2, units(30.0));
            });
            let spliced = std::cell::Cell::new(None);
            let splice = |p: u64| {
                if spliced.get().is_none() {
                    state.splice(0, 1, 0);
                    spliced.set(Some(state.anchor()));
                }
                Some(9 - p)
            };
            let mut lent = state.lend_heights();
            if in_measured {
                state.adjust_row_height(2, 5.0);
            }
            lent.reorder(10, |p| Some(9 - p));
            assert!(state.reorder_with(lent, 10, splice).is_err(), "refused");
            assert_eq!(Some(state.anchor()), spliced.get(), "the splice's anchor");
            assert_eq!(state.len(), 9);
            assert!(!state.is_lent(), "the refused loan ended");
        }
    }

    /// A mapping that measures a row on each call, +1 then -1, never
    /// settles: refused after one more pass, the loan ended and the
    /// list's heights as the mapping left them (Codex 5, F1 review 3).
    #[test]
    fn a_mapping_that_never_stops_measuring_is_refused() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            ext.set(0, units(30.0));
        });
        let up = std::cell::Cell::new(true);
        let wobble = |p: u64| {
            state.adjust_row_height(0, if up.replace(!up.get()) { 1.0 } else { -1.0 });
            Some(p)
        };
        let mut lent = state.lend_heights();
        state.adjust_row_height(0, 1.0);
        lent.reorder(10, Some);
        assert!(state.reorder_with(lent, 10, wobble).is_err());
        assert!(!state.is_lent());
        let h = state.row_height(0);
        assert!(h == 30.0 || h == 31.0 || h == 32.0, "{h}");
    }

    /// A row the mapping measures is kept (Opus, F1 review 2): it is
    /// noted, and mapped with the rest.
    #[test]
    fn a_row_the_mapping_measures_is_kept() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(10, units(30.0));
            for r in [1, 2] {
                ext.set(r, units(30.0));
            }
        });
        let measured = std::cell::Cell::new(false);
        let reverse = |p: u64| {
            if !measured.replace(true) {
                state.adjust_row_height(1, 10.0);
            }
            Some(9 - p)
        };
        let mut lent = state.lend_heights();
        state.adjust_row_height(2, 5.0);
        lent.reorder(10, |p| Some(9 - p));
        assert!(state.reorder_with(lent, 10, reverse).is_ok());
        assert_eq!(state.row_height(7), 35.0, "measured since the loan");
        assert_eq!(state.row_height(8), 40.0, "measured by the mapping");
    }

    /// Codex 5 (F1 review 4): a mapping that scrolls the list to row 7
    /// changes what the anchor was worked out from: the commit, one for
    /// both reorders, refuses, and the scroll stands (the restored anchor
    /// never overwrites it).
    #[test]
    fn a_mapping_that_scrolls_refuses_either_reorder_and_the_scroll_stands() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| *ext = ExtentTree::new(10, units(30.0)));
        let scrolls = |p: u64| {
            state.scroll_to_row(7);
            Some(p)
        };
        let mut lent = state.lend_heights();
        lent.reorder(10, Some);
        assert!(state.reorder_with(lent, 10, scrolls).is_err(), "reorder_with refused");
        assert_eq!(state.anchor().row, 7, "row 7 stands");
        state.scroll_to_row(0);
        assert!(!state.reorder(10, scrolls), "plain reorder refused");
        assert_eq!(state.anchor().row, 7, "row 7 stands");
        assert!(state.reorder(10, |p| Some(9 - p)), "a mapping that only maps commits");
        assert_eq!(state.anchor().row, 2, "the anchor follows its row");
    }

    /// Codex 3 (F1 review 5): the read set is taken before any call of
    /// `to`. Ten rows, one measured since the loan; only the first call of
    /// `to` (the measured row's, or the heights') scrolls to row 7, the
    /// rest map `9 - p`. Either reorder refuses, and row 7 stands.
    #[test]
    fn a_first_call_of_to_that_scrolls_refuses_either_reorder() {
        for with_loan in [true, false] {
            let state = ListState::new(30.0);
            state.heights_mut(|ext| {
                *ext = ExtentTree::new(10, units(30.0));
                ext.set(3, units(30.0));
            });
            let first = std::cell::Cell::new(true);
            let to = |p: u64| {
                if first.replace(false) {
                    state.scroll_to_row(7);
                }
                Some(9 - p)
            };
            if with_loan {
                let mut lent = state.lend_heights();
                state.adjust_row_height(3, 16.0);
                lent.reorder(10, |p| Some(9 - p));
                assert!(state.reorder_with(lent, 10, to).is_err(), "reorder_with refused");
            } else {
                state.adjust_row_height(3, 16.0);
                assert!(!state.reorder(10, to), "plain reorder refused");
            }
            assert!(!first.get(), "`to` ran");
            assert_eq!(state.anchor().row, 7, "row 7 stands");
        }
    }

    /// Codex 2 (F1 review 6): the window's focus is in the read set. The
    /// first call of `to` moves focus from A to B (the window's state, the
    /// list's unchanged): either reorder refuses, and focus stays on B.
    #[test]
    fn a_first_call_of_to_that_moves_focus_refuses_either_reorder() {
        for with_loan in [true, false] {
            let state = ListState::new(30.0);
            state.heights_mut(|ext| {
                *ext = ExtentTree::new(10, units(30.0));
                ext.set(3, units(30.0));
            });
            let (a, b) = (crate::FocusHandle::new(), crate::FocusHandle::new());
            crate::focus::set_focus(Some(a.id()), false);
            let first = std::cell::Cell::new(true);
            let to = |p: u64| {
                if first.replace(false) {
                    crate::focus::set_focus(Some(b.id()), false);
                }
                Some(9 - p)
            };
            if with_loan {
                let mut lent = state.lend_heights();
                state.adjust_row_height(3, 16.0);
                lent.reorder(10, |p| Some(9 - p));
                assert!(state.reorder_with(lent, 10, to).is_err(), "reorder_with refused");
            } else {
                assert!(!state.reorder(10, to), "plain reorder refused");
            }
            assert_eq!(
                crate::untracked(crate::focus::focused),
                Some(b.id()),
                "focus stays on B"
            );
        }
    }

    /// Codex 6 (F1 review 7): a `to` that panics ends the loan it was
    /// given, so no row measured after it is noted for a loan nothing can
    /// take back.
    #[test]
    fn a_panicking_to_ends_the_loan_it_was_given() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| *ext = ExtentTree::new(10, units(30.0)));
        let mut lent = state.lend_heights();
        state.adjust_row_height(3, 16.0);
        lent.reorder(10, |p| Some(9 - p));
        let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.reorder_with(lent, 10, |_| panic!("a mapping fails")).is_ok()
        }));
        assert!(ran.is_err(), "the panic is raised");
        assert!(!state.is_lent(), "the loan ended");
        state.adjust_row_height(4, 16.0);
        assert!(
            state.read(|s| s.since_lent.is_empty()).unwrap_or(false),
            "nothing is noted for it"
        );
    }

    /// Codex 4 (F1 review 8): a `to` that disposes the list refuses the
    /// reorder quietly (no failure in a debug build) and hands the
    /// caller's heights back whole, never a default tree in their place:
    /// whether it disposes it while the measured rows are mapped (a row
    /// measured since the loan) or as the anchor is followed (none).
    #[test]
    fn a_to_that_disposes_the_list_gets_the_callers_heights_back() {
        for measured in [true, false] {
            let state = ListState::new(30.0);
            state.heights_mut(|ext| *ext = ExtentTree::new(10, units(30.0)));
            let mut lent = state.lend_heights();
            if measured {
                state.adjust_row_height(3, 16.0);
            }
            lent.reorder(10, |p| Some(9 - p));
            let called = std::cell::Cell::new(false);
            let refused = state.reorder_with(lent, 10, |p| {
                if !called.replace(true) {
                    state.dispose();
                }
                Some(9 - p)
            });
            assert!(called.get(), "`to` ran");
            let back = refused.expect_err("refused");
            assert_eq!(back.extents.len(), 10, "the caller's heights, not a default tree");
            assert_eq!(back.extents.height(0), Some(units(30.0)));
        }
    }

    /// Opus 3 (F1 review 4): a splice inside plain reorder's mapping is
    /// not lost under heights reordered from before it: the commit sees the
    /// heights written since it read them and refuses.
    #[test]
    fn a_splice_inside_plain_reorders_mapping_is_not_lost() {
        let state = ListState::new(30.0);
        state.heights_mut(|ext| *ext = ExtentTree::new(10, units(30.0)));
        let spliced = std::cell::Cell::new(false);
        let splices = |p: u64| {
            if !spliced.replace(true) {
                state.splice(0, 0, 5);
            }
            Some(9 - p)
        };
        assert!(!state.reorder(10, splices), "refused");
        assert_eq!(state.len(), 15, "the splice stands");
    }

    /// A fork's first write of a list copies its state in O(1), however
    /// many rows were measured since a loan (Codex, F1 review 1), and
    /// putting it back costs what it touched.
    #[cfg(debug_assertions)]
    #[test]
    fn a_fork_copies_a_list_with_many_rows_measured_since_a_loan_in_o_1() {
        let state = ListState::new(30.0);
        let n = 100_000_u64;
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(n, units(30.0));
            for r in 0..n {
                ext.set(r, units(20.0));
            }
        });
        let _lent = state.lend_heights();
        for r in 0..n {
            state.adjust_row_height(r, 1.0);
        }
        let ((), bytes) = crate::heap::allocated_bytes(|| {
            let _cold = crate::engine::ColdFrame::begin();
            state.adjust_row_height(0, 1.0);
        });
        assert!(bytes < 64 * 1024, "{bytes} bytes for one write of one list");
        assert_eq!(state.row_height(0), 21.0, "put back");
    }

    /// The rows noted since a loan fill their allocation, then a fork notes
    /// one more (Codex, F1 review 2): the fork's note goes to a tail of its
    /// own, O(its notes), and after the rollback the live allocation is the
    /// one it was, at the room it had.
    #[cfg(debug_assertions)]
    #[test]
    fn a_fork_never_grows_the_live_notes() {
        let state = ListState::new(30.0);
        let n = 16_384_u64;
        state.heights_mut(|ext| {
            *ext = ExtentTree::new(n + 1, units(30.0));
            for r in 0..=n {
                ext.set(r, units(30.0));
            }
        });
        let _lent = state.lend_heights();
        for r in 0..n {
            state.adjust_row_height(r, 1.0);
        }
        let room = |state: &ListState| {
            state
                .peek(|s| {
                    let items = s.since_lent.items.borrow();
                    (items.as_ptr().addr(), items.capacity(), s.since_lent.len)
                })
                .expect("alive")
        };
        let before = room(&state);
        assert_eq!(before.1, before.2, "full");
        let ((), bytes) = crate::heap::allocated_bytes(|| {
            let _cold = crate::engine::ColdFrame::begin();
            state.adjust_row_height(n, 1.0);
            let noted = state.peek(|s| (s.since_lent.len, s.since_lent.tail.len()));
            assert_eq!(noted, Some((16_384, 1)), "the shared ones, and its own");
        });
        // The fork's own bookkeeping, not a copy of the 16384 notes (256 KB).
        assert!(bytes < 32 * 1024, "{bytes} bytes for one note in a fork");
        assert_eq!(room(&state), before, "the live allocation, as it was");
        assert_eq!(state.peek(|s| s.since_lent.to_vec().len()), Some(16_384));
    }

    /// ADR-0034, S7: a check's first write of one small field of a list
    /// with many rows built, pinned and holders waiting copies none of
    /// them: its copy shares the rows built and the holders waiting (each
    /// waiter by identity), and live is as it was after.
    #[cfg(debug_assertions)]
    #[test]
    fn a_checks_first_write_to_a_list_shares_its_rows_and_waiters() {
        let state = ListState::new(30.0);
        let n = 20_000_u64;
        let keys: Vec<ElementKey> = (0..n).collect();
        let pins: Vec<(u64, ElementKey)> = (0..n).map(|r| (r, r)).collect();
        state.write(|s| Built::set(&mut s.built, &keys, &pins, 7));
        let wake: Rc<dyn Fn()> = Rc::new(|| ());
        for holder in 1..=1_000 {
            state.wait_for_heights(holder, Rc::downgrade(&wake));
        }
        let roots = |state: ListState| {
            state
                .peek(|s| (Rc::clone(&s.built), Rc::clone(&s.waiting), s.holder))
                .expect("alive")
        };
        let (built, waiting, holder) = roots(state);
        assert_eq!(holder, None);
        let (shared, bytes) = crate::engine::in_cold_frame(|| {
            crate::heap::allocated_bytes(|| {
                assert!(state.claim_heights(5_000), "claimed in the check");
                let (b, w, h) = roots(state);
                assert_eq!(h, Some(5_000), "the check reads its write");
                Rc::ptr_eq(&b, &built) && Rc::ptr_eq(&w, &waiting)
            })
        });
        assert!(shared, "the check's copy shares the rows built and the waiters");
        // The rows built and pinned are 480 KB, the waiters 24 KB: the
        // copy is the fork's own bookkeeping.
        assert!(bytes < 16 * 1024, "{bytes} bytes for one write of one field");
        let (b, w, h) = roots(state);
        assert_eq!(h, None, "live is as it was");
        assert!(Rc::ptr_eq(&b, &built) && Rc::ptr_eq(&w, &waiting));
        assert_eq!(
            (b.keys.len(), b.pins.len(), b.pins_before, w.len()),
            (20_000, 20_000, 7, 1_000)
        );
        assert!(
            w.iter().all(|(_, waiter)| waiter.ptr_eq(&Rc::downgrade(&wake))),
            "the waiters by identity"
        );
        drop((b, w));
        assert_eq!(
            (Rc::strong_count(&built), Rc::strong_count(&waiting)),
            (2, 2),
            "live's and this test's: the check's copy is gone"
        );
    }

    /// A list of the rows `ids` holds, keyed and labelled by id, `tall(id)`
    /// pixels each, in a 300 px viewport; the row `focus` names tracks its handle.
    fn by_id(
        state: &ListState,
        ids: Signal<Vec<u64>>,
        tall: fn(u64) -> f32,
        focus: Option<(u64, crate::FocusHandle)>,
    ) -> Scenario {
        let st = *state;
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
                                let id = at(&r, i);
                                let d = div().height(tall(id)).child(text(format!("row {id}")));
                                match focus {
                                    Some((f, h)) if f == id => d.track_focus(h),
                                    _ => d,
                                }
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

    /// Codex (M15 review 1): the reader's row sorted deep behind rows never
    /// measured, estimated 24 px, that lay out at 48: the anchor holds at
    /// its offset through the layouts that measure them, as a row scrolled
    /// to does, and no row above it is taken for the anchor.
    #[test]
    fn the_readers_row_sorted_behind_taller_rows_stays_at_its_offset() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..1_000).collect());
        let mut s = by_id(&state, ids, |id| if id < 50 { 24.0 } else { 48.0 }, None);
        s.settle().unwrap();
        s.act("row 10, 5 px above the top", || {
            state.scroll_to(Anchor {
                row: 10,
                key: None,
                offset: 5.0,
            });
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, 10), Some(-5.0));
        // Ids 50..850 first, then 0..50: id 10 goes to 810, behind 800
        // rows that lay out twice as tall as the estimate.
        let to = |p: u64| {
            Some(if p < 50 {
                p + 800
            } else if p < 850 {
                p - 50
            } else {
                p
            })
        };
        s.act("sort", || {
            ids.update(|v| v[..850].rotate_left(50));
            state.reorder(1_000, to);
        });
        s.settle().unwrap();
        assert_eq!(state.anchor().row, 810);
        assert_eq!(top_of(&s, 10), Some(-5.0), "the reader's row where it was");
    }

    /// Codex (M15 review 2): a row below the viewport brought into view
    /// (its bottom at the viewport's), and the data sorted before the
    /// layout: the row is found at its new index and its bottom is still
    /// the viewport's.
    #[test]
    fn a_reorder_keeps_the_alignment_of_a_pending_scroll() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..1_000).collect());
        let mut s = by_id(&state, ids, |_| 24.0, None);
        s.settle().unwrap();
        let to = |p: u64| {
            Some(match p {
                40 => 60,
                60 => 40,
                p => p,
            })
        };
        s.act("row 40 into view, then sorted to 60", || {
            state.scroll_into_view(40);
            ids.update(|v| v.swap(40, 60));
            state.reorder(1_000, to);
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, 40), Some(300.0 - 24.0), "its bottom at the viewport's");
    }

    /// Codex (M15 review 1): focus moved to a row and the data sorted in
    /// the same turn, the row going far from the window: the row is found
    /// from what holds focus now, not from the last layout, and is kept
    /// built and focused.
    #[test]
    fn a_row_focused_then_sorted_away_in_the_same_turn_keeps_focus() {
        let state = ListState::new(24.0);
        let ids: Signal<Vec<u64>> = Signal::new((0..1_000).collect());
        let h = crate::FocusHandle::new();
        let mut s = by_id(&state, ids, |_| 24.0, Some((3, h)));
        s.settle().unwrap();
        // Rows 3 and 900 trade places; the reader's row (0) stays.
        let to = |p: u64| {
            Some(match p {
                3 => 900,
                900 => 3,
                p => p,
            })
        };
        s.act("focus row 3 and sort it away", || {
            h.focus();
            ids.update(|v| v.swap(3, 900));
            state.reorder(1_000, to);
        });
        s.settle().unwrap();
        assert_eq!(top(&s), 0, "the window stayed at the top");
        assert_eq!(s.read(crate::focus::focused), Some(h.id()), "row 3 keeps focus");
        assert_eq!(state.focused_row(), Some(900), "kept built at its new index");
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

    /// A list built while its owner ran and laid out after the owner ended
    /// (a layer's content mounted late) is inert: its bindings read through
    /// the handle, never a signal the owner disposed.
    #[test]
    fn a_list_laid_out_after_its_owner_ended_is_inert() {
        let scope = crate::Scope::new();
        let built = scope.run(|| {
            let state = ListState::new(24.0);
            state.fit_rows(Some(200.0));
            virtual_list(
                &state,
                50,
                |i| i,
                |i| div().height(24.0).child(text(format!("row {i}"))).into_element(),
            )
            .into_element()
        });
        scope.dispose();
        let built = Rc::new(std::cell::RefCell::new(Some(built)));
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(300.0)
                    .child(built.borrow_mut().take().unwrap_or_else(|| div().into_element()))
                    .into_element()
            },
            300,
            300,
            1.0,
        )
        .unwrap();
        // The view hands over one element built before: a cold build of
        // it has nothing to build again.
        s.check_cold(false);
        s.settle().unwrap();
    }

    /// What a list and its scroll container hold, as their readers see it.
    #[cfg(debug_assertions)]
    fn held(state: ListState) -> String {
        let list = state
            .read(|s| {
                format!(
                    "{:?} {:?} {:?} {:?} {:?} {} {} {:?} {} {} {} {:?} {:?} {}",
                    s.anchor,
                    s.window,
                    s.request,
                    s.built.keys,
                    s.viewport,
                    s.extents.extent(),
                    s.extents.runs(),
                    s.extents.height(3),
                    s.follow,
                    s.pinned,
                    s.to_end,
                    s.built.pins,
                    s.since_lent,
                    s.shape,
                )
            })
            .unwrap_or_default();
        let scroll = state.scroll().expect("alive");
        format!(
            "{list} | {:?} {} {:?} {:?} {}",
            scroll.offset(),
            scroll.wheels(),
            scroll.reach(),
            scroll.layout(),
            scroll.is_alive()
        )
    }

    /// A list written through inside a fork (ADR-0028, the alias test):
    /// its anchor, window, heights and scroll moved there are put back
    /// after, and a copy of the handle made before the fork reads the
    /// same as the handle. A list the fork made is gone with it.
    #[cfg(debug_assertions)]
    #[test]
    fn what_a_fork_writes_through_a_list_is_put_back() {
        let state = ListState::new(30.0);
        let mut s = scenario(&state);
        s.act("down", || state.scroll_to_row(400));
        s.settle().unwrap();
        let alias = state;
        let before = held(state);
        let made = {
            let _cold = crate::engine::ColdFrame::begin();
            state.scroll_to_row(9_000);
            state.follow_tail(true);
            state.heights_mut(|e| {
                e.set(3, 99);
                e.splice(0, 1, 5);
            });
            let mut laid = state.scroll().and_then(|h| h.layout()).expect("laid out");
            laid.bounds.height = 123.0;
            laid.offset.1 += 50.0;
            let _moved = state.laid_out(state.scroll().expect("alive"), &laid, None);
            state.scroll().expect("alive").scroll_to((0.0, 5_000.0));
            assert_ne!(held(alias), before, "the fork sees its own writes through any copy");
            ListState::new(10.0)
        };
        assert_eq!(held(state), before, "put back");
        assert_eq!(held(alias), before, "the copy made before reads the same");
        assert!(!made.is_alive(), "what the fork made went with it");
        // The fork's id never names a live list made after it (Codex, F1
        // review 1): its slot is reused under a later generation.
        let later = ListState::new(10.0);
        assert!(!made.is_alive(), "the fork's handle does not alias the later list");
        made.dispose();
        assert!(later.is_alive(), "disposing the fork's handle leaves the later list");
        later.dispose();
        s.settle().unwrap();
        assert_eq!(top(&s), 400, "the live list is where it was");
    }
}
