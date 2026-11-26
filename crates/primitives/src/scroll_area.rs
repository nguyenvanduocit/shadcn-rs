//! A box that scrolls its content behind bars of its own (reka's
//! `ScrollArea`: `ScrollAreaRoot`, `ScrollAreaViewport`,
//! `ScrollAreaScrollbar`, `ScrollAreaThumb`, `ScrollAreaCorner`).
//!
//! **Parts.** The root is the frame and the region the pointer hovers; the
//! viewport inside it scrolls and is a Tab stop; a bar per axis, laid over
//! the viewport's edge, holds a thumb; the corner fills the square where
//! two bars meet. The engine's own overlay thumb is turned off on the
//! viewport: the bars are these parts, drawn by the look.
//!
//! **Axes.** Only an axis the caller gives a bar scrolls; the other is
//! clipped, by the wheel, touch and keys alike
//! (`ScrollArea/ScrollAreaViewport.vue:59-60`). reka's quirk of turning
//! both axes off when one bar unmounts (`ScrollAreaScrollbar.vue:66-69`)
//! is not kept: the axes are the caller's [`ScrollAreaOptions::bars`].
//!
//! **Keys.** The viewport is a Tab stop (`:63`). reka leaves its keys to the
//! browser's own scrolling, which `gui` does not have, so they are
//! Chrome's here (`ScrollableArea::LineStep`, `PageStep`): with the
//! viewport focused, or a key no descendant took,
//!
//! | Key | Result |
//! | --- | --- |
//! | ArrowDown, ArrowUp | 40 px down, up |
//! | ArrowRight, ArrowLeft | the view 40 px right, left (a horizontal bar), right to left too |
//! | PageDown, Space; PageUp, Shift+Space | 87.5 % of the viewport down; up |
//! | End, Home | to the end; to the start |
//!
//! A key that cannot scroll it further is left to the scroller around it
//! (Chrome's scroll chaining).
//! Space and Shift+Space count only while the viewport itself holds focus,
//! so a button inside still takes Space. Chrome animates these scrolls; here
//! they land at once. No key with Control, Alt or Command.
//!
//! **When a bar shows** ([`Policy`], reka's `type`; each bar on its own,
//! `ScrollArea/ScrollAreaScrollbar*.vue`):
//!
//! | Policy | Shown |
//! | --- | --- |
//! | [`Policy::Always`] | always |
//! | [`Policy::Auto`] | while the content overflows its axis, checked [`OVERFLOW_CHECK`] (10 ms) after the last change of size |
//! | [`Policy::Hover`] (the default) | while the pointer is in the root and the content overflows; leaving, it hides after the delay |
//! | [`Policy::Scroll`] | while its axis scrolls; [`SCROLL_END`] (100 ms) after the last scroll it waits the delay and hides; the pointer on the bar keeps it |
//! | [`Policy::Glimpse`] | the pointer entering the root shows it, then it hides after the delay though the pointer stays, at once if it leaves first; a scroll shows it until 100 ms after, then the delay; the pointer entering while it scrolls or waits keeps it until the pointer leaves, then the delay; never without overflow |
//!
//! The delay is [`HIDE_DELAY`] (600 ms) unless the caller sets one. reka's
//! `scroll` bar never reaches its "interacting" state, as nothing tells it
//! of the pointer, so it hides under the pointer
//! (`ScrollAreaScrollbarScroll.vue:24-41`); here the pointer over the bar
//! is what it waits for (research §4 item 13). reka's hover bar remounts
//! its overflow check on each entry and so shows 10 ms after the pointer
//! enters; here the overflow is known all along and it shows at once.
//! A bar that hides plays the look's exit through [`Presence`].
//!
//! **The thumb** is there only while 0 < viewport / content < 1
//! (`ScrollAreaScrollbarVisible.vue:48-51`). Its length is the track's
//! (less its padding) times that ratio, never under [`MIN_THUMB`] (18 px,
//! macOS's; `ScrollArea/utils.ts:17-24`); its offset follows the scroll
//! offset, clamped so overscroll never moves it (`:73-93`). Both are bound
//! to the thumb's node: a scroll moves it with no render, in the frame
//! that lays the scroll out.
//!
//! **Pointer** on a bar (`ScrollAreaScrollbarImpl.vue:41-90`, `utils.ts:99-127`):
//!
//! - pressing the thumb and dragging keeps the point grabbed under the
//!   pointer;
//! - pressing the track jumps so the thumb's centre is under the pointer,
//!   at once, then drags on until release (a press does not page, as a
//!   system bar's would);
//! - a drag goes on outside the bar, captured;
//! - the wheel over a bar scrolls that bar's axis by its vertical delta,
//!   the horizontal bar too; it is taken (the page does not scroll along)
//!   only while the offset is strictly between 0 and the end. reka tests
//!   that against a right-to-left offset that is negative, so it never
//!   takes it there; here the offset runs from the start in every
//!   direction and the test holds in both.
//!
//! **Placement** (`ScrollAreaScrollbarX.vue:31-36`, `ScrollAreaScrollbarY.vue:32-38`,
//! `ScrollAreaCornerImpl.vue:12-43`): the vertical bar along the right
//! edge (the left, right to left), from the top down to the corner; the
//! horizontal bar along the bottom, from the start to the corner; the
//! corner in the bottom right (left) square, as wide as the vertical bar
//! and as tall as the horizontal one, only when both bars show, both have
//! a size, and the policy is not [`Policy::Scroll`]
//! (`ScrollAreaCorner.vue:18-23`). Right to left, a horizontal scroll starts
//! at the right and so does its thumb, and a drag or a press counts from
//! that side.
//!
//! **To assistive technology.** Nothing of its own: the bars, thumbs and
//! corner are decoration, left out of the tree; the viewport holds the
//! content.
//!
//! **The virtualizer** (reka's `ScrollAreaVirtualizer`, research §12): a
//! viewport of any number of rows that builds only those in view
//! ([`ScrollArea::virtualizer`] in setup, [`ScrollArea::virtual_viewport`]
//! in render), laid on `gui::list` (ADR-0014) and scrolled by these bars,
//! keys and wheel. Its numbers are reka's (`ScrollAreaVirtualizer.vue:53-91`):
//! rows estimated [`ROW_ESTIMATE`] (28 px) until measured, [`OVERSCAN`]
//! (12) rows' worth built beyond each edge, and the focused row kept built
//! with its neighbours, so Tab and Shift+Tab from a row scrolled away go on
//! to the next and previous rows. A row that takes focus while not wholly
//! in view is brought into view (Chrome's focus scroll, which reka's rows
//! get from the browser). Better than reka: the list tells assistive
//! technology the whole row count and each row's place itself, where reka
//! leaves `aria-setsize` and `aria-posinset` to the caller
//! (`scroll-area.md:309-314`); its offset is a row and a distance into it,
//! so ten million rows scroll as exactly as ten. It runs down the column
//! only: reka's `horizontal` mode, and its right-to-left scroll to a row
//! (`:61-69`), which only that mode reads, are not built (ADR-0025, group 8
//! second review fixes, item 1).

use gui::engine::WorldCell;
use std::rc::Rc;
use std::time::Duration;

use gui::list::{ListLayout, ListState, virtual_list_in};
use gui::prelude::*;
use gui::{
    Div, DragEvent, DragPhase, Element, ElementKey, KeyEvent, NamedKey, ScrollHandle, ScrollLayout, Signal, WheelEvent,
};

use crate::direction::ReadDirection;
use crate::hover::Delay;
use crate::presence::Presence;

/// How far an arrow key scrolls, in pixels (Chrome's
/// `ScrollableArea::PixelsPerLineStep`).
pub const LINE_STEP: f32 = 40.0;

/// How much of the viewport a page key scrolls (Chrome's
/// `kMinFractionToStepWhenPaging`).
pub const PAGE_FRACTION: f32 = 0.875;

/// How long a bar waits before it hides unless the caller says otherwise
/// (`ScrollArea/ScrollAreaRoot.vue:53-56`).
pub const HIDE_DELAY: Duration = Duration::from_millis(600);

/// How long after the last scroll a scroll counts as ended
/// (`ScrollAreaScrollbarScroll.vue:55`, `ScrollAreaScrollbarGlimpse.vue:64`).
pub const SCROLL_END: Duration = Duration::from_millis(100);

/// How long after the last change of size the overflow is checked
/// (`ScrollAreaScrollbarAuto.vue:23-43`).
pub const OVERFLOW_CHECK: Duration = Duration::from_millis(10);

/// The shortest a thumb gets, in pixels, as macOS's (`ScrollArea/utils.ts:17-24`).
pub const MIN_THUMB: f32 = 18.0;

/// How tall a virtualizer's row is taken to be until measured, in pixels
/// (`ScrollAreaVirtualizer.vue:53-59`).
pub const ROW_ESTIMATE: f32 = 28.0;

/// How many rows a virtualizer builds beyond each edge of the viewport
/// (`ScrollAreaVirtualizer.vue:53-59`).
pub const OVERSCAN: u16 = 12;

/// What the caller decides of a virtualizer (reka's `estimateSize` and
/// `overscan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualizerOptions {
    /// A row's height until it is measured, in pixels.
    pub estimate: f32,
    /// Rows built beyond each edge of the viewport, counted at the estimate.
    pub overscan: u16,
}

impl Default for VirtualizerOptions {
    fn default() -> Self {
        Self {
            estimate: ROW_ESTIMATE,
            overscan: OVERSCAN,
        }
    }
}

/// Which axes have a scrollbar, and so scroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Bars {
    /// Down only (shadcn's default).
    #[default]
    Vertical,
    /// Across only.
    Horizontal,
    Both,
}

impl Bars {
    #[must_use]
    pub const fn vertical(self) -> bool {
        matches!(self, Self::Vertical | Self::Both)
    }

    #[must_use]
    pub const fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal | Self::Both)
    }

    /// Whether `axis` has a bar.
    #[must_use]
    pub const fn has(self, axis: Axis) -> bool {
        match axis {
            Axis::Horizontal => self.horizontal(),
            Axis::Vertical => self.vertical(),
        }
    }
}

/// When the bars show (reka's `type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Policy {
    Always,
    Auto,
    #[default]
    Hover,
    Scroll,
    Glimpse,
}

/// What the caller decides of a scroll area, read when it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollAreaOptions {
    pub policy: Policy,
    /// How long a bar waits before it hides.
    pub hide_delay: Duration,
    /// The axes with a bar.
    pub bars: Bars,
}

impl Default for ScrollAreaOptions {
    fn default() -> Self {
        Self {
            policy: Policy::Hover,
            hide_delay: HIDE_DELAY,
            bars: Bars::Vertical,
        }
    }
}

/// A thumb's place in its track, in pixels: what its node is bound to.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Thumb {
    /// Along the axis.
    pub length: f32,
    /// From the track's start (inside its padding), the reading direction's
    /// start for the horizontal bar.
    pub offset: f32,
}

/// The viewport as last laid out, per axis (x, y).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct Metrics {
    viewport: (f32, f32),
    content: (f32, f32),
    offset: (f32, f32),
    max: (f32, f32),
}

impl Metrics {
    fn of(layout: &ScrollLayout) -> Self {
        let b = layout.bounds;
        Self {
            viewport: (b.width, b.height),
            content: (b.width + layout.max.0, b.height + layout.max.1),
            offset: layout.offset,
            max: layout.max,
        }
    }

    const fn along(self, axis: Axis) -> (f32, f32, f32, f32) {
        match axis {
            Axis::Horizontal => (self.viewport.0, self.content.0, self.offset.0, self.max.0),
            Axis::Vertical => (self.viewport.1, self.content.1, self.offset.1, self.max.1),
        }
    }
}

/// A bar's track as last laid out.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
struct Track {
    /// Along the axis, inside its borders (`clientHeight`).
    size: f32,
    pad_start: f32,
    pad_end: f32,
    /// Across the axis, its border box: the corner's side.
    thickness: f32,
}

/// Where a `scroll` or `glimpse` bar is (reka's state machines).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Machine {
    #[default]
    Hidden,
    Glimpse,
    Scrolling,
    Interacting,
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Scroll,
    ScrollEnd,
    PointerEnter,
    PointerLeave,
    Hide,
}

/// reka's tables (`ScrollAreaScrollbarScroll.vue:22-36`,
/// `ScrollAreaScrollbarGlimpse.vue:28-50`); the glimpse state only under
/// [`Policy::Glimpse`].
const fn transition(policy: Policy, now: Machine, event: Event) -> Machine {
    use Event::{Hide, PointerEnter, PointerLeave, Scroll, ScrollEnd};
    use Machine::{Glimpse, Hidden, Idle, Interacting, Scrolling};
    let glimpse = matches!(policy, Policy::Glimpse);
    match (now, event) {
        (Hidden, Scroll) | (Glimpse, Scroll) | (Idle, Scroll) => Scrolling,
        (Hidden, PointerEnter) | (Glimpse, PointerEnter) if glimpse => Glimpse,
        (Glimpse, Hide) | (Glimpse, PointerLeave) | (Idle, Hide) => Hidden,
        (Scrolling, ScrollEnd) | (Interacting, PointerLeave) => Idle,
        (Scrolling, PointerEnter) | (Interacting, Scroll) | (Idle, PointerEnter) => Interacting,
        (state, _) => state,
    }
}

/// One axis's bar.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bar {
    track: Signal<Track>,
    machine: Signal<Machine>,
    /// The content overflows the axis, as last checked.
    overflow: Signal<bool>,
    /// Where the pointer grabbed the thumb along it while a drag runs (0:
    /// its centre, a press on the track).
    grab: Signal<Option<f32>>,
    hide: Delay,
    scroll_end: Delay,
    check: Delay,
}

impl Bar {
    fn new() -> Self {
        Self {
            track: Signal::labeled("scroll bar track", Track::default()),
            machine: Signal::labeled("scroll bar state", Machine::Hidden),
            overflow: Signal::labeled("scroll bar overflow", false),
            grab: Signal::labeled("scroll bar grab", None),
            hide: Delay::new(),
            scroll_end: Delay::new(),
            check: Delay::new(),
        }
    }
}

struct State {
    handle: ScrollHandle,
    options: Rc<dyn Fn() -> ScrollAreaOptions>,
    direction: ReadDirection,
    /// The viewport's focus: Space scrolls only while it has it.
    focus: gui::FocusHandle,
    /// The sizes the last overflow check was asked for.
    sized: WorldCell<((f32, f32), (f32, f32))>,
    /// The virtualizer's rows, while the part that made them is mounted
    /// ([`ScrollArea::virtualizer`]): they scroll, not `handle`.
    list: gui::Holder<ListState>,
    /// The row holding focus in the virtualizer, as last laid out.
    focused_row: WorldCell<Option<u64>>,
}

/// A scroll area's state: made once in its root's setup, `Copy`. Its
/// handle holds the viewport's offset; the caller scrolls it through
/// [`ScrollArea::handle`] (reka's `scrollTop`, `scrollTopLeft` and
/// `viewport`).
#[derive(Clone, Copy, PartialEq)]
pub struct ScrollArea {
    metrics: Signal<Metrics>,
    /// The viewport's and the content's sizes alone, so what depends only
    /// on them (whether a thumb is there) is not run again by a scroll.
    sizes: Signal<((f32, f32), (f32, f32))>,
    /// The `hover` policy's bars show.
    hover_shown: Signal<bool>,
    hover_hide: Delay,
    x: Bar,
    y: Bar,
    state: Signal<State>,
}

impl std::fmt::Debug for ScrollArea {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScrollArea").finish_non_exhaustive()
    }
}

impl ScrollArea {
    /// A scroll area with the caller's `options` read when used, its
    /// horizontal bar placed in `direction`. Called in setup.
    #[must_use]
    #[track_caller]
    pub fn new(options: impl Fn() -> ScrollAreaOptions + 'static, direction: impl Into<ReadDirection>) -> Self {
        let me = Self {
            metrics: Signal::labeled("scroll area metrics", Metrics::default()),
            sizes: Signal::labeled("scroll area sizes", ((0.0, 0.0), (0.0, 0.0))),
            hover_shown: Signal::labeled("scroll area hovered", false),
            hover_hide: Delay::new(),
            x: Bar::new(),
            y: Bar::new(),
            state: Signal::new(State {
                handle: ScrollHandle::new(),
                options: Rc::new(options),
                direction: direction.into(),
                focus: gui::FocusHandle::new(),
                sized: WorldCell::new("a scroll area's sizes checked", ((-1.0, -1.0), (-1.0, -1.0))),
                list: gui::Holder::new(),
                focused_row: WorldCell::new("a scroll area's focused row", None),
            }),
        };
        let handle = me.handle();
        handle.on_layout(move |layout| me.laid_out(layout));
        me
    }

    /// The caller's options now (tracked through what its closure reads).
    #[must_use]
    pub fn options(self) -> ScrollAreaOptions {
        let f = self.state.with_untracked(|s| Rc::clone(&s.options));
        f()
    }

    /// The viewport's offset and layout. A virtualizer's rows scroll
    /// through its list instead ([`ScrollArea::virtualizer`]).
    #[must_use]
    pub fn handle(self) -> ScrollHandle {
        self.state.with_untracked(|s| s.handle)
    }

    /// The virtualizer's rows, when the viewport is one (untracked: for
    /// handlers and hooks).
    fn list(self) -> Option<ListState> {
        if !self.state.is_alive() {
            return None;
        }
        self.state.with_untracked(|s| s.list).get_untracked()
    }

    /// [`Self::list`], tracked: a render that read it renders again as a
    /// part makes or replaces the list, or leaves with it.
    fn list_tracked(self) -> Option<ListState> {
        if !self.state.is_alive() {
            return None;
        }
        self.state.with_untracked(|s| s.list).get()
    }

    /// Back to the top (reka's `scrollTop`).
    pub fn scroll_top(self) {
        if let Some(list) = self.list() {
            list.scroll_to_row(0);
            return;
        }
        let handle = self.handle();
        let (x, _) = handle.offset();
        handle.scroll_to((x, 0.0));
    }

    /// Back to the top left corner (reka's `scrollTopLeft`).
    pub fn scroll_top_left(self) {
        if let Some(list) = self.list() {
            list.scroll_to_row(0);
            return;
        }
        self.handle().scroll_to((0.0, 0.0));
    }

    /// Make the viewport a virtualizer of `options` (reka's
    /// `ScrollAreaVirtualizer`): its rows are laid on the list this returns,
    /// which the caller scrolls to a row ([`ListState::scroll_to_row`],
    /// reka's `scrollToIndex`) and whose layout hook is the scroll area's.
    /// Called in setup, the list is the area's while the calling part is
    /// mounted (its leaving lets go of it); the render lays the rows out
    /// with [`ScrollArea::virtual_viewport`].
    #[must_use]
    #[track_caller]
    pub fn virtualizer(self, options: VirtualizerOptions) -> ListState {
        let list = ListState::new(options.estimate);
        list.set_prefetch(f32::from(options.overscan) * options.estimate);
        list.hide_scrollbar(true);
        list.keep_around_focus(1);
        // The list is read from the state, not held by its own hook.
        let me = self;
        list.on_layout(move |l| me.metrics.is_alive() && me.list().is_some_and(|list| me.list_laid_out(&list, l)));
        self.state.with_untracked(|s| s.list).hold(list);
        list
    }

    /// The virtualizer's viewport (its setup called
    /// [`ScrollArea::virtualizer`]): `rows` rows, `row(i)` built for those
    /// in view and the overscan, each keyed `key(i)`, in a scroll container
    /// styled by `look`. It fills the box it is given, is a Tab stop and
    /// scrolls by Chrome's keys, as [`ScrollArea::viewport`]. Without a
    /// virtualizer, the rows are none.
    #[must_use]
    pub fn virtual_viewport(
        self,
        rows: u64,
        key: impl Fn(u64) -> ElementKey,
        row: impl Fn(u64) -> Element,
        look: impl FnOnce(Div) -> Div,
    ) -> Div {
        let Some(list) = self.list_tracked() else {
            return look(gui::div());
        };
        let focus = self.state.with_untracked(|s| s.focus);
        virtual_list_in(&list, rows, key, row, move |v| {
            look(v).track_focus(focus).on_key_down(move |ev| {
                let bars = gui::untracked(|| self.options()).bars;
                self.key_virtual(bars, ev, focus.is_focused())
            })
        })
    }

    /// The virtualizer was laid out: the bars read its offset over its
    /// whole extent, and a row that took focus out of view comes into it.
    fn list_laid_out(self, list: &ListState, l: &ListLayout) -> bool {
        if !self.metrics.is_alive() {
            return false;
        }
        let focused = list.focused_row();
        let moved_focus = self.state.with_untracked(|s| s.focused_row.replace(focused)) != focused;
        let mut again = false;
        if moved_focus && let Some(row) = focused {
            let top = list.offset_of(row) - l.top;
            let bottom = top + f64::from(list.row_height(row));
            if top < 0.0 || bottom > f64::from(l.viewport) {
                list.scroll_into_view(row);
                again = true;
            }
        }
        let total = l.extent + f64::from(list.end_space());
        let max = (total - f64::from(l.viewport)).max(0.0);
        let now = Metrics {
            viewport: (l.width, l.viewport),
            content: (l.width, narrow(total)),
            offset: (0.0, narrow(l.top)),
            max: (0.0, narrow(max)),
        };
        self.take_metrics(now) || again
    }

    /// The virtualizer's offset from its top and how far it goes, in
    /// pixels, exact over any extent.
    fn list_range(list: &ListState) -> (f64, f64) {
        let total = list.extent() + f64::from(list.end_space());
        (list.view_top(), (total - f64::from(list.viewport_height())).max(0.0))
    }

    /// Put the virtualizer's top `to` pixels below its first row's.
    fn list_scroll_to(list: &ListState, to: f64) {
        let (_, max) = Self::list_range(list);
        if max > 0.0 {
            list.scroll_to_fraction((to / max).clamp(0.0, 1.0));
        }
    }

    /// Chrome's keys on the virtualizer: down the column only.
    fn key_virtual(self, bars: Bars, ev: &KeyEvent, itself: bool) -> bool {
        let Some(list) = self.list() else {
            return false;
        };
        if !bars.vertical() {
            return false;
        }
        let (y, max) = Self::list_range(&list);
        let page = f64::from(list.viewport_height());
        let Some((_, to)) = key((0.0, y), (0.0, max), page, Bars::Vertical, ev, itself, false) else {
            return false;
        };
        Self::list_scroll_to(&list, to);
        true
    }

    const fn bar(self, axis: Axis) -> Bar {
        match axis {
            Axis::Horizontal => self.x,
            Axis::Vertical => self.y,
        }
    }

    fn rtl(self) -> bool {
        self.state.with_untracked(|s| s.direction.get()) == Direction::Rtl
    }

    /// The viewport was laid out: what the bars read follows, and a scroll
    /// or a change of size is told to them. Whether the frame must be laid
    /// out again (the thumbs are bound to what changed).
    fn laid_out(self, layout: &ScrollLayout) -> bool {
        if !self.metrics.is_alive() {
            return false;
        }
        self.take_metrics(Metrics::of(layout))
    }

    /// The viewport now measures `now`.
    fn take_metrics(self, now: Metrics) -> bool {
        let was = self.metrics.get_untracked();
        if was == now {
            return false;
        }
        let policy = gui::untracked(|| self.options()).policy;
        if matches!(policy, Policy::Scroll | Policy::Glimpse) {
            for (axis, moved) in [
                (Axis::Horizontal, was.offset.0 != now.offset.0),
                (Axis::Vertical, was.offset.1 != now.offset.1),
            ] {
                if moved {
                    self.scrolled(axis);
                }
            }
        }
        // The overflow is checked 10 ms after the last change of size
        // (reka's debounced `ResizeObserver`), not after a scroll.
        let sized = (now.viewport, now.content);
        if self.state.with_untracked(|s| s.sized.replace(sized)) != sized {
            self.sizes.set(sized);
            for axis in [Axis::Horizontal, Axis::Vertical] {
                let bar = self.bar(axis);
                bar.check.after(OVERFLOW_CHECK, move || {
                    if bar.overflow.is_alive() {
                        let (view, content, ..) = self.metrics.get_untracked().along(axis);
                        let over = view < content;
                        if bar.overflow.get_untracked() != over {
                            bar.overflow.set(over);
                        }
                    }
                });
            }
        }
        self.metrics.set(now);
        true
    }

    fn scrolled(self, axis: Axis) {
        self.dispatch(axis, Event::Scroll);
        self.bar(axis)
            .scroll_end
            .after(SCROLL_END, move || self.dispatch(axis, Event::ScrollEnd));
    }

    fn dispatch(self, axis: Axis, event: Event) {
        let bar = self.bar(axis);
        if !bar.machine.is_alive() {
            return;
        }
        let options = gui::untracked(|| self.options());
        let now = bar.machine.get_untracked();
        let next = transition(options.policy, now, event);
        if next == now {
            return;
        }
        bar.machine.set(next);
        bar.hide.cancel();
        if next == Machine::Idle || next == Machine::Glimpse {
            bar.hide
                .after(options.hide_delay, move || self.dispatch(axis, Event::Hide));
        }
    }

    /// The pointer entered (or left) the root.
    fn hovered(self, inside: bool) {
        let options = gui::untracked(|| self.options());
        match options.policy {
            Policy::Hover if inside => {
                self.hover_hide.cancel();
                self.hover_shown.set(true);
            }
            Policy::Hover => {
                let shown = self.hover_shown;
                self.hover_hide.after(options.hide_delay, move || {
                    if shown.is_alive() {
                        shown.set(false);
                    }
                });
            }
            Policy::Glimpse => {
                let event = if inside {
                    Event::PointerEnter
                } else {
                    Event::PointerLeave
                };
                self.dispatch(Axis::Horizontal, event);
                self.dispatch(Axis::Vertical, event);
            }
            Policy::Always | Policy::Auto | Policy::Scroll => {}
        }
    }

    /// Whether the bar of `axis` shows (tracked): its look's presence.
    #[must_use]
    pub fn shown(self, axis: Axis) -> bool {
        let options = self.options();
        if !options.bars.has(axis) {
            return false;
        }
        let bar = self.bar(axis);
        match options.policy {
            Policy::Always => true,
            Policy::Auto => bar.overflow.get(),
            Policy::Hover => self.hover_shown.get() && bar.overflow.get(),
            Policy::Scroll => bar.machine.get() != Machine::Hidden,
            Policy::Glimpse => bar.machine.get() != Machine::Hidden && bar.overflow.get(),
        }
    }

    /// The thumb of `axis`'s bar (tracked): none while the content fits,
    /// or fills nothing.
    #[must_use]
    pub fn thumb_at(self, axis: Axis) -> Option<Thumb> {
        let (view, content, offset, max) = self.metrics.get().along(axis);
        let track = self.bar(axis).track.get();
        thumb(view, content, offset, max, track)
    }

    /// Whether `axis`'s bar has a thumb (tracked).
    #[must_use]
    pub fn has_thumb(self, axis: Axis) -> bool {
        let ((vw, vh), (cw, ch)) = self.sizes.get();
        match axis {
            Axis::Horizontal => has_thumb(vw, cw),
            Axis::Vertical => has_thumb(vh, ch),
        }
    }

    /// The corner's width and height (tracked): both bars show, both have a
    /// size, and the policy is not `scroll`.
    #[must_use]
    pub fn corner_size(self) -> Option<(f32, f32)> {
        if self.options().policy == Policy::Scroll || !self.shown(Axis::Horizontal) || !self.shown(Axis::Vertical) {
            return None;
        }
        let (w, h) = (self.y.track.get().thickness, self.x.track.get().thickness);
        (w > 0.0 && h > 0.0).then_some((w, h))
    }

    /// `d`, the look's frame, as the root: the region the pointer hovers.
    /// Sets `on_hover`.
    #[must_use]
    pub fn root(self, d: Div) -> Div {
        d.on_hover(move |inside| self.hovered(inside))
    }

    /// `d`, the look's box, as the viewport: it scrolls on the axes with a
    /// bar and clips the others, shows no bar of the engine's, is a Tab stop
    /// and scrolls by Chrome's keys. Sets `on_key_down`.
    #[must_use]
    pub fn viewport(self, d: Div) -> Div {
        let axis = |on: bool| if on { Overflow::Scroll } else { Overflow::Hidden };
        let (handle, focus) = self.state.with_untracked(|s| (s.handle, s.focus));
        let bars = self.options().bars;
        let keys = handle;
        d.overflow_y(axis(bars.vertical()))
            .overflow_x(axis(bars.horizontal()))
            .scrollbar_hidden(true)
            .track_scroll(&handle)
            .track_focus(focus)
            .on_key_down(move |ev| {
                let bars = gui::untracked(|| self.options()).bars;
                let Some(layout) = keys.layout() else {
                    return false;
                };
                let (x, y) = layout.offset;
                let (max_x, max_y) = layout.max;
                let page = f64::from(layout.bounds.height);
                let offset = (f64::from(x), f64::from(y));
                let max = (f64::from(max_x), f64::from(max_y));
                match key(offset, max, page, bars, ev, focus.is_focused(), self.rtl()) {
                    Some((x, y)) => {
                        keys.scroll_to((narrow(x), narrow(y)));
                        true
                    }
                    None => false,
                }
            })
    }

    /// `d`, the look's bar of `axis`, placed over the viewport's edge and
    /// shown as the policy says: the thumb dragged or the track pressed
    /// scrolls its axis, and so does the wheel over it. Its padding and
    /// borders along the axis are the track's ends. Sets `on_drag`,
    /// `on_wheel`, `on_hover` and `on_measured`.
    #[must_use]
    pub fn scrollbar(self, mut d: Div, axis: Axis) -> Element {
        let style = d.style_mut();
        let (pad, border) = (style.padding, style.border_widths);
        let (pad_start, pad_end, border_along) = match axis {
            Axis::Horizontal => (pad.left, pad.right, border.left + border.right),
            Axis::Vertical => (pad.top, pad.bottom, border.top + border.bottom),
        };
        let bar = self.bar(axis);
        let rtl = self.rtl();
        let corner = move |w: bool| self.corner_size().map_or(0.0, |(cw, ch)| if w { cw } else { ch });
        let d = d.position(Position::Absolute).aria_hidden(true);
        let d = match axis {
            Axis::Vertical => {
                let d = d.top(0.0).bottom(move || Length::Px(corner(false)));
                if rtl { d.left(0.0) } else { d.right(0.0) }
            }
            Axis::Horizontal => {
                let d = d.bottom(0.0);
                if rtl {
                    d.right(0.0).left(move || Length::Px(corner(true)))
                } else {
                    d.left(0.0).right(move || Length::Px(corner(true)))
                }
            }
        };
        let d = d
            .on_measured(move |b| {
                let (along, across) = match axis {
                    Axis::Horizontal => (b.width, b.height),
                    Axis::Vertical => (b.height, b.width),
                };
                let track = Track {
                    size: (along - border_along).max(0.0),
                    pad_start,
                    pad_end,
                    thickness: across,
                };
                if bar.track.is_alive() && bar.track.get_untracked() != track {
                    bar.track.set(track);
                }
            })
            .on_drag(move |e| self.drag(axis, e))
            .on_wheel(move |w| self.wheel(axis, w))
            // The pointer on a `scroll` bar keeps it (reka never knows).
            .on_hover(move |inside| {
                if gui::untracked(|| self.options()).policy == Policy::Scroll {
                    let event = if inside {
                        Event::PointerEnter
                    } else {
                        Event::PointerLeave
                    };
                    self.dispatch(axis, event);
                }
            });
        Presence::of(self.shown(axis), false).render(d)
    }

    /// `d`, the look's thumb, in `axis`'s bar: its length and offset bound
    /// to the node, so a scroll moves it without a render; none while the
    /// content fits.
    #[must_use]
    pub fn thumb(self, d: Div, axis: Axis) -> Option<Div> {
        if !self.has_thumb(axis) {
            return None;
        }
        let at = move || self.thumb_at(axis).unwrap_or_default();
        let rtl = self.rtl();
        let d = d.flex_shrink(0.0).aria_hidden(true);
        Some(match axis {
            Axis::Vertical => d
                .height(move || Length::Px(at().length))
                .translate_y(move || Calc::px(at().offset)),
            Axis::Horizontal => d
                .width(move || Length::Px(at().length))
                .translate_x(move || Calc::px(if rtl { -at().offset } else { at().offset })),
        })
    }

    /// `d`, the look's corner, placed where the bars meet, sized to them;
    /// none unless both show.
    #[must_use]
    pub fn corner(self, d: Div) -> Option<Div> {
        let (w, h) = self.corner_size()?;
        let d = d
            .position(Position::Absolute)
            .bottom(0.0)
            .width(w)
            .height(h)
            .aria_hidden(true);
        Some(if self.rtl() { d.left(0.0) } else { d.right(0.0) })
    }

    /// The scroll offset of `axis` the pointer `along` the bar (from its
    /// left or top border edge) asks for, the thumb grabbed `grab` from its
    /// start (0: its centre), as an offset from the reading start.
    fn offset_for_pointer(self, axis: Axis, along: f32, grab: f32) -> Option<f32> {
        let (view, content, _, max) = gui::untracked(|| self.metrics.get()).along(axis);
        let track = self.bar(axis).track.get_untracked();
        if !has_thumb(view, content) {
            return None;
        }
        let s = scroll_from_pointer(along, grab, view, content, track);
        let s = s.clamp(0.0, max);
        Some(if axis == Axis::Horizontal && self.rtl() {
            max - s
        } else {
            s
        })
    }

    fn set_offset(self, axis: Axis, to: f32) {
        if let Some(list) = self.list() {
            if axis == Axis::Vertical {
                Self::list_scroll_to(&list, f64::from(to));
            }
            return;
        }
        let handle = self.handle();
        let (x, y) = handle.offset();
        let to = match axis {
            Axis::Horizontal => (to, y),
            Axis::Vertical => (x, to),
        };
        if to != (x, y) {
            handle.scroll_to(to);
        }
    }

    fn drag(self, axis: Axis, e: &DragEvent) {
        let bar = self.bar(axis);
        let along = match axis {
            Axis::Horizontal => e.x - e.bounds.x,
            Axis::Vertical => e.y - e.bounds.y,
        };
        match e.phase {
            DragPhase::Start => {
                // On the thumb: where it was grabbed; on the track: its
                // centre comes under the pointer.
                let grab = gui::untracked(|| self.thumb_at(axis)).map_or(0.0, |t| {
                    let track = bar.track.get_untracked();
                    let start = if axis == Axis::Horizontal && self.rtl() {
                        track.size - track.pad_end - t.offset - t.length
                    } else {
                        track.pad_start + t.offset
                    };
                    let inside = along - start;
                    if (0.0..=t.length).contains(&inside) {
                        inside
                    } else {
                        0.0
                    }
                });
                bar.grab.set(Some(grab));
                if let Some(to) = self.offset_for_pointer(axis, along, grab) {
                    self.set_offset(axis, to);
                }
            }
            DragPhase::Move => {
                let Some(grab) = bar.grab.get_untracked() else {
                    return;
                };
                if let Some(to) = self.offset_for_pointer(axis, along, grab) {
                    self.set_offset(axis, to);
                }
            }
            DragPhase::End | DragPhase::Cancel => bar.grab.set(None),
        }
    }

    fn wheel(self, axis: Axis, w: &WheelEvent) -> bool {
        if let Some(list) = self.list() {
            if axis != Axis::Vertical {
                return false;
            }
            let (offset, max) = Self::list_range(&list);
            let to = offset + f64::from(w.dy);
            Self::list_scroll_to(&list, to.clamp(0.0, max));
            return to > 0.0 && to < max;
        }
        let (_, _, offset, max) = gui::untracked(|| self.metrics.get()).along(axis);
        // The vertical delta, on the horizontal bar too; right to left the
        // start is the right edge.
        let to = if axis == Axis::Horizontal && self.rtl() {
            offset - w.dy
        } else {
            offset + w.dy
        };
        self.set_offset(axis, to.clamp(0.0, max));
        to > 0.0 && to < max
    }
}

fn has_thumb(view: f32, content: f32) -> bool {
    let ratio = if content > 0.0 { view / content } else { 0.0 };
    ratio > 0.0 && ratio < 1.0
}

/// reka's `getThumbSize` and `getThumbOffsetFromScroll`.
fn thumb(view: f32, content: f32, offset: f32, max: f32, track: Track) -> Option<Thumb> {
    if !has_thumb(view, content) {
        return None;
    }
    let inner = (track.size - track.pad_start - track.pad_end).max(0.0);
    let length = (inner * view / content).max(MIN_THUMB);
    let room = inner - length;
    let offset = if max > 0.0 {
        offset.clamp(0.0, max) / max * room
    } else {
        0.0
    };
    Some(Thumb { length, offset })
}

/// reka's `getScrollPositionFromPointer`, left to right: the offset the
/// pointer at `along` asks for with the thumb grabbed `grab` from its start
/// (0: its centre). Not clamped.
fn scroll_from_pointer(along: f32, grab: f32, view: f32, content: f32, track: Track) -> f32 {
    let inner = (track.size - track.pad_start - track.pad_end).max(0.0);
    let length = (inner * view / content).max(MIN_THUMB);
    let grab = if grab == 0.0 { length / 2.0 } else { grab };
    let lo = track.pad_start + grab;
    let hi = track.size - track.pad_end - (length - grab);
    let max = content - view;
    if (hi - lo).abs() < f32::EPSILON || max == 0.0 {
        return 0.0;
    }
    (along - lo) * max / (hi - lo)
}

/// An offset as the layout holds it.
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    reason = "an offset in pixels; `as` rounds to the nearest f32"
)]
fn narrow(px: f64) -> f32 {
    px as f32
}

/// Where `ev` scrolls a viewport at `(x, y)` of `(max_x, max_y)`, `height`
/// tall, with `bars`; `None` when it does not move it. A key that cannot
/// move it (at the edge already) is not taken, so it goes on to the
/// scroller around it, as Chrome chains it. `itself`: the viewport holds
/// focus, not a descendant. `rtl`: the horizontal offset runs from the
/// right edge leftwards (gui's, as right to left content overflows to the
/// left), so ArrowRight, which scrolls the view to the right as Chrome's
/// does, lowers it. In `f64`, exact over a virtualizer's extent.
fn key(
    (x, y): (f64, f64),
    (max_x, max_y): (f64, f64),
    height: f64,
    bars: Bars,
    ev: &KeyEvent,
    itself: bool,
    rtl: bool,
) -> Option<(f64, f64)> {
    let m = ev.modifiers;
    if m.control || m.alt || m.command {
        return None;
    }
    let line = f64::from(LINE_STEP);
    let page = (height * f64::from(PAGE_FRACTION)).max(1.0);
    let space = ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ");
    let right = if rtl { -line } else { line };
    let to = if ev.is(NamedKey::ArrowDown) && bars.vertical() {
        (x, y + line)
    } else if ev.is(NamedKey::ArrowUp) && bars.vertical() {
        (x, y - line)
    } else if ev.is(NamedKey::ArrowRight) && bars.horizontal() {
        (x + right, y)
    } else if ev.is(NamedKey::ArrowLeft) && bars.horizontal() {
        (x - right, y)
    } else if bars.vertical() && (ev.is(NamedKey::PageDown) || (space && itself && !m.shift)) {
        (x, y + page)
    } else if bars.vertical() && (ev.is(NamedKey::PageUp) || (space && itself && m.shift)) {
        (x, y - page)
    } else if ev.is(NamedKey::End) && bars.vertical() {
        (x, max_y)
    } else if ev.is(NamedKey::Home) && bars.vertical() {
        (x, 0.0)
    } else {
        return None;
    };
    let to = (to.0.clamp(0.0, max_x), to.1.clamp(0.0, max_y));
    (to != (x, y)).then_some(to)
}

#[cfg(test)]
mod tests {
    use super::{Event, MIN_THUMB, Machine, Policy, Track, scroll_from_pointer, thumb, transition};

    fn track(size: f32) -> Track {
        Track {
            size,
            pad_start: 1.0,
            pad_end: 1.0,
            thickness: 10.0,
        }
    }

    #[test]
    fn the_thumb_is_the_share_shown_never_under_18_px() {
        // 200 of 1000 shown on a 202 px track (1 px padding each end).
        let t = thumb(200.0, 1000.0, 0.0, 800.0, track(202.0)).expect("overflows");
        assert_eq!(t.length, 40.0);
        assert_eq!(t.offset, 0.0);
        let end = thumb(200.0, 1000.0, 800.0, 800.0, track(202.0)).expect("overflows");
        assert_eq!(end.offset, 160.0, "at the end it reaches the track's end");
        let past = thumb(200.0, 1000.0, 900.0, 800.0, track(202.0)).expect("overflows");
        assert_eq!(past.offset, 160.0, "overscroll does not move it");
        let tiny = thumb(10.0, 100_000.0, 0.0, 99_990.0, track(202.0)).expect("overflows");
        assert_eq!(tiny.length, MIN_THUMB);
        assert!(
            thumb(200.0, 200.0, 0.0, 0.0, track(202.0)).is_none(),
            "the content fits"
        );
    }

    #[test]
    fn a_press_on_the_track_centres_the_thumb_and_a_grab_stays_under_the_pointer() {
        let tr = track(202.0);
        // Centre: the thumb (40 px) centred at the pointer, from 1 + 20 to 201 − 20.
        assert_eq!(scroll_from_pointer(21.0, 0.0, 200.0, 1000.0, tr), 0.0);
        assert_eq!(scroll_from_pointer(181.0, 0.0, 200.0, 1000.0, tr), 800.0);
        assert_eq!(scroll_from_pointer(101.0, 0.0, 200.0, 1000.0, tr), 400.0);
        // Grabbed 5 px from its start: that point follows the pointer.
        assert_eq!(scroll_from_pointer(6.0, 5.0, 200.0, 1000.0, tr), 0.0);
        assert_eq!(scroll_from_pointer(166.0, 5.0, 200.0, 1000.0, tr), 800.0);
    }

    #[test]
    fn the_scroll_and_glimpse_machines() {
        use Event::{Hide, PointerEnter, PointerLeave, Scroll, ScrollEnd};
        use Machine::{Glimpse, Hidden, Idle, Interacting, Scrolling};
        let s = |m, e| transition(Policy::Scroll, m, e);
        assert_eq!(s(Hidden, Scroll), Scrolling);
        assert_eq!(
            s(Hidden, PointerEnter),
            Hidden,
            "scroll: the pointer alone shows nothing"
        );
        assert_eq!(s(Scrolling, ScrollEnd), Idle);
        assert_eq!(s(Scrolling, PointerEnter), Interacting);
        assert_eq!(s(Interacting, ScrollEnd), Interacting, "held while hovered");
        assert_eq!(s(Interacting, PointerLeave), Idle);
        assert_eq!(s(Idle, Hide), Hidden);
        assert_eq!(s(Idle, Scroll), Scrolling);
        let g = |m, e| transition(Policy::Glimpse, m, e);
        assert_eq!(g(Hidden, PointerEnter), Glimpse);
        assert_eq!(g(Glimpse, PointerLeave), Hidden, "leaving a glimpse hides at once");
        assert_eq!(g(Glimpse, Hide), Hidden);
        assert_eq!(g(Glimpse, Scroll), Scrolling);
        assert_eq!(g(Idle, PointerEnter), Interacting);
    }
}
