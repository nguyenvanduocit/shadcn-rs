//! Message scroller (`ui/message-scroller`): the scrolling frame of a chat
//! thread. Its engine (`useMessageScroller`) keeps the thread pinned to the
//! bottom while replies stream and backs off when the reader scrolls up,
//! settles a new turn near the top with a peek of the one before, opens where
//! asked, keeps the reader's place when history is prepended, and says which
//! way there is more to read and which turn is in view.
//!
//! The thread is a virtual list ([`gui::list`], ADR-0014): a thread of a
//! million messages builds only the rows in view and a prefetch margin, and
//! every behaviour works on the messages' indices in the data, built or not.
//! The list holds the scroll position as a row and an offset into it, so a
//! message asked for lands exactly once the rows around it are measured,
//! history prepended above keeps the reader's message where it was (the
//! viewport reports the rows inserted to the list), and the room below the
//! last turn that lets it scroll to the top is the list's end space.
//!
//! The engine reads the list after each layout ([`ListState::on_layout`]),
//! where Vue reads the DOM in its observers: what it changes (the anchor, the
//! end space) is laid out again before the frame is painted. The thread's
//! gap is part of each row (room below every message but the last), so the
//! list's offsets are its rows' boxes exactly. Scrolls jump: `gui` animates
//! no scroll, so `behavior: "smooth"` lands at once.

use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gui::list::{Anchor, ListLayout, ListState, virtual_list_in};
use gui::prelude::*;
use gui::style::Calc;
use gui::{
    ComponentElement, Div, Easing, Element, ElementKey, Icon, Properties, Signal, clock, component, div, icon, key_of,
    provide, use_context, watch,
};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::StateLook;
use crate::theme::theme;

const SCROLL_EPSILON: f64 = 0.5;
const AUTOSCROLLING_TIMEOUT: Duration = Duration::from_millis(180);
/// The thread's gap between messages, in pixels.
const DEFAULT_GAP: f32 = 32.0;
/// How tall a message is taken to be until one is measured.
const ROW_ESTIMATE: f32 = 96.0;

/// A height or offset in the list's `f64` pixels as an `f32` (small: one
/// relative to a row or the viewport).
#[expect(clippy::as_conversions, reason = "a pixel offset local to the viewport")]
fn local(v: f64) -> f32 {
    v as f32
}

/// Where a thread opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MessageScrollerDefaultScrollPosition {
    Start,
    #[default]
    End,
    /// The last anchored turn near the top, or the end when what follows it fits.
    LastAnchor,
}

/// Which edge of the viewport a message is brought to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MessageScrollerScrollAlign {
    #[default]
    Start,
    Center,
    End,
    /// Only as far as it takes to show it whole.
    Nearest,
}

/// How [`MessageScroller::scroll_to_message`] places a message.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct MessageScrollerScrollOptions {
    pub align: MessageScrollerScrollAlign,
    /// Room kept between the message and the edge; the scroller's own when `None`.
    pub scroll_margin: Option<f32>,
}

/// Whether there is more of the thread above (`start`) and below (`end`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct MessageScrollerScrollable {
    pub start: bool,
    pub end: bool,
}

/// The messages in view and the anchored turn the reader is in, as indices
/// in the thread's data.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageScrollerVisibility {
    /// The last anchored turn whose top is at or above the line a new turn
    /// settles at (built or not).
    pub current_anchor: Option<usize>,
    /// The messages showing below that line.
    pub visible: Range<usize>,
}

/// Vue's `MessageScrollerProvider` props.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageScrollerProps {
    /// Follow the end of the thread while the reader is there.
    pub auto_scroll: bool,
    pub default_scroll_position: MessageScrollerDefaultScrollPosition,
    /// How close to an edge counts as at it.
    pub scroll_edge_threshold: f32,
    /// How much of the turn before stays in view above a new anchored turn.
    pub scroll_previous_item_peek: f32,
    pub scroll_margin: f32,
}

impl Default for MessageScrollerProps {
    fn default() -> Self {
        Self {
            auto_scroll: false,
            default_scroll_position: MessageScrollerDefaultScrollPosition::End,
            scroll_edge_threshold: 8.0,
            scroll_previous_item_peek: 64.0,
            scroll_margin: 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    FollowingBottom,
    FreeScrolling,
    AnchoredToMessage,
    SettlingJump,
}

/// An edge a message is placed at (`Nearest` decided).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edge {
    Start,
    Center,
    End,
}

/// A message being placed: held there while its turn grows (`hold`), or
/// until it is measured where asked.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settle {
    row: u64,
    edge: Edge,
    margin: f32,
    hold: bool,
}

/// What the data changed since the last layout.
#[derive(Clone, Debug, Default)]
struct Change {
    /// Messages before it.
    previous: u64,
    /// Messages inserted above the first.
    prepended: u64,
    /// Anchored messages appended at the end: index and key.
    added_anchors: Vec<(u64, ElementKey)>,
    /// The last message replaced by an anchored one, the count unchanged.
    replaced_anchor: Option<(u64, ElementKey)>,
}

/// Vue's `createEngine`: the scroller's state between layouts.
struct Engine {
    props: MessageScrollerProps,
    list: ListState,
    me: Weak<RefCell<Engine>>,
    /// Owns the engine's timers: they are cancelled with the provider.
    owner: gui::Scope,
    scrollable: Signal<MessageScrollerScrollable>,
    visibility: Signal<MessageScrollerVisibility>,
    autoscrolling: Signal<bool>,
    /// The gap below every message but the last, in pixels.
    gap: f32,
    /// The data as last rendered: its length, first and last keys.
    len: u64,
    first: Option<ElementKey>,
    last: Option<ElementKey>,
    /// The rows built: the first's index and whether each is an anchor.
    built_first: u64,
    built_anchors: Vec<bool>,
    /// The last anchor above the rows built.
    anchor_before: Option<u64>,
    /// The last anchor of the thread (for `LastAnchor`, until it is applied).
    last_anchor: Option<u64>,
    /// "The last anchor at or before `from` is `found`", for the data as it is.
    anchor_cache: Option<(u64, Option<u64>)>,
    change: Option<Change>,
    laid: bool,
    /// The viewport's width and height and the rows' extent, as last laid out.
    size: (f32, f32, f64),
    shown_top: f64,
    last_scroll_top: f64,
    mode: Mode,
    settle: Option<Settle>,
    default_applied: bool,
    pending: Option<(u64, MessageScrollerScrollOptions)>,
    handled_anchors: HashSet<ElementKey>,
    /// Bumped by each programmatic scroll; the timer that ends autoscrolling
    /// acts only for the last.
    autoscroll_run: u64,
    /// The frame must be laid out again.
    dirty: bool,
}

impl Engine {
    // ── At render ──────────────────────────────────────────────────────────

    /// The data as the viewport renders it: what changed since the last
    /// render, reported to the list before it builds (rows inserted above,
    /// the gap moving to the row that is no longer last).
    fn data(&mut self, len: u64, gap: f32, key: &dyn Fn(u64) -> ElementKey, anchor: &dyn Fn(u64) -> bool) {
        self.gap = gap;
        let first = (len > 0).then(|| key(0));
        let last = len.checked_sub(1).map(key);
        if (len, first, last) != (self.len, self.first, self.last) {
            let previous = self.len;
            // Earlier messages came in above: the old first found further down.
            let prepended = match self.first {
                Some(old) if first != Some(old) && len > previous => {
                    (1..=len.saturating_sub(previous)).find(|&k| key(k) == old).unwrap_or(0)
                }
                _ => 0,
            };
            if prepended > 0 {
                self.list.splice(0, 0, prepended);
                self.shift(prepended);
            }
            let was_last = previous.checked_sub(1).map(|r| r.saturating_add(prepended));
            // The old last message gains the gap as messages come after it,
            // the new last one loses it when those after it went.
            match (was_last, self.last) {
                (Some(r), Some(old)) if r.saturating_add(1) < len && key(r) == old => {
                    self.list.adjust_row_height(r, gap);
                }
                _ if prepended == 0 && first == self.first && len > 0 && len < previous => {
                    self.list.adjust_row_height(len.saturating_sub(1), -gap);
                }
                _ => {}
            }
            let added_anchors = if previous > 0 {
                (previous.saturating_add(prepended)..len)
                    .filter(|&r| anchor(r))
                    .map(|r| (r, key(r)))
                    .collect()
            } else {
                Vec::new()
            };
            let replaced_anchor = match last {
                Some(k) if len == previous && prepended == 0 && last != self.last => len
                    .checked_sub(1)
                    .filter(|&r| anchor(r) && !self.handled_anchors.contains(&k))
                    .map(|r| (r, k)),
                _ => None,
            };
            let change = match self.change.take() {
                Some(mut c) => {
                    c.prepended = c.prepended.saturating_add(prepended);
                    for a in &mut c.added_anchors {
                        a.0 = a.0.saturating_add(prepended);
                    }
                    c.added_anchors.extend(added_anchors);
                    c.replaced_anchor = replaced_anchor.or(c.replaced_anchor);
                    c
                }
                None => Change {
                    previous,
                    prepended,
                    added_anchors,
                    replaced_anchor,
                },
            };
            self.change = Some(change);
            self.anchor_cache = None;
            (self.len, self.first, self.last) = (len, first, last);
        }
        if !self.default_applied
            && self.props.default_scroll_position == MessageScrollerDefaultScrollPosition::LastAnchor
        {
            self.last_anchor = len.checked_sub(1).and_then(|q| self.anchor_at_or_before(q, anchor));
        }
    }

    /// The rows the list built this render, from `first`.
    fn built(&mut self, first: u64, anchors: Vec<bool>, anchor: &dyn Fn(u64) -> bool) {
        self.built_first = first;
        self.built_anchors = anchors;
        self.anchor_before = first.checked_sub(1).and_then(|q| self.anchor_at_or_before(q, anchor));
    }

    /// The last anchor at or before row `q`, scanning only what the last
    /// answer does not cover.
    fn anchor_at_or_before(&mut self, q: u64, anchor: &dyn Fn(u64) -> bool) -> Option<u64> {
        match self.anchor_cache {
            Some((from, found)) if from <= q => {
                let hit = (from.saturating_add(1)..=q).rev().find(|&r| anchor(r)).or(found);
                self.anchor_cache = Some((q, hit));
                hit
            }
            // No anchor between `found` and `from`, so none after it up to `q`.
            Some((_, Some(found))) if found <= q => Some(found),
            _ => {
                let hit = (0..=q).rev().find(|&r| anchor(r));
                self.anchor_cache = Some((q, hit));
                hit
            }
        }
    }

    /// Rows `k` were inserted above every row the engine holds on to.
    fn shift(&mut self, k: u64) {
        if let Some(s) = &mut self.settle {
            s.row = s.row.saturating_add(k);
        }
        if let Some((row, _)) = &mut self.pending {
            *row = row.saturating_add(k);
        }
    }

    // ── After layout ────────────────────────────────────────────────────────

    /// What the observers and the scroll events of Vue's viewport react to.
    fn laid_out(&mut self, l: &ListLayout) -> bool {
        self.dirty = false;
        if l.wheeled {
            self.user_scroll_intent();
        }
        let first = !self.laid;
        self.laid = true;
        let size = (l.width, l.viewport, l.extent);
        let moved = l.top - self.shown_top;
        self.shown_top = l.top;
        if !first && !l.wheeled && moved > SCROLL_EPSILON && self.following() {
            // The list followed the end down as the thread grew: Vue's autoscroll.
            self.set_autoscrolling(true);
        }
        if let Some(change) = self.change.take() {
            self.size = size;
            self.apply_content_change(&change);
        } else if size != self.size {
            self.size = size;
            self.handle_resize();
        }
        if moved.abs() > SCROLL_EPSILON {
            self.commit_scroll_state();
        }
        if first {
            // Vue's provider, once mounted.
            self.apply_default_scroll_position();
            self.commit_scroll_state();
        }
        if self.pending.is_some() {
            self.flush_pending_scroll_to_message();
        }
        self.hold_settle();
        self.sync_visibility(l);
        self.sync_follow();
        self.dirty
    }

    fn following(&self) -> bool {
        self.props.auto_scroll && self.mode == Mode::FollowingBottom
    }

    fn apply_content_change(&mut self, c: &Change) {
        if self.flush_pending_scroll_to_message() {
            return;
        }
        let auto = self.props.auto_scroll;
        if c.previous == 0 {
            if self.apply_default_scroll_position() || (self.len > 0 && auto && self.scroll_to_end()) {
                return;
            }
            self.commit_scroll_state();
            return;
        }
        if c.prepended > 0 {
            // The list's anchor followed its message down: the reader's place is kept.
            self.commit_scroll_state();
            return;
        }
        if self.len > c.previous
            && let Some(&(row, key)) = c.added_anchors.first()
        {
            if self.following() && c.added_anchors.len() > 1 {
                self.scroll_to_end();
                return;
            }
            self.scroll_to_element(row, MessageScrollerScrollOptions::default(), true);
            self.handled_anchors.insert(key);
            return;
        }
        if self.len == c.previous
            && let Some((row, key)) = c.replaced_anchor
        {
            self.scroll_to_element(row, MessageScrollerScrollOptions::default(), true);
            self.handled_anchors.insert(key);
            return;
        }
        if self.following() {
            self.scroll_to_end();
        } else {
            self.commit_scroll_state();
        }
    }

    fn handle_resize(&mut self) {
        if self.following() {
            self.scroll_to_end();
            return;
        }
        // A held turn is placed again below (`hold_settle`).
        if !self.settle.is_some_and(|s| s.hold) {
            self.commit_scroll_state();
        }
    }

    fn user_scroll_intent(&mut self) {
        self.settle = None;
        if self.mode != Mode::FreeScrolling {
            self.mode = Mode::FreeScrolling;
        }
    }

    /// Place the message being settled again with what is measured now: a
    /// held turn while its reply grows, a jump until its row is measured.
    fn hold_settle(&mut self) {
        let Some(s) = self.settle else { return };
        if s.row >= self.list.len() {
            self.settle = None;
            return;
        }
        let before = self.list.end_space();
        let placed = self.place(s);
        if s.hold {
            if self.props.auto_scroll && before > 0.0 && self.list.end_space() == 0.0 {
                // The reply outgrew the room below: follow it from here.
                self.scroll_to_end();
            }
        } else if placed && self.list.is_measured(s.row) {
            self.settle = None;
        }
    }

    // ── State the view reads ───────────────────────────────────────────────

    fn commit_scroll_state(&mut self) {
        let edge = f64::from(self.props.scroll_edge_threshold);
        let top = self.list.view_top();
        let measured = if self.laid {
            MessageScrollerScrollable {
                start: top > edge,
                end: self.list.extent() - top - f64::from(self.list.viewport_height()) > edge,
            }
        } else {
            MessageScrollerScrollable::default()
        };
        self.update_mode_from_scroll(measured, top);
        let next = if self.mode == Mode::FollowingBottom {
            MessageScrollerScrollable { end: false, ..measured }
        } else {
            measured
        };
        if self.scrollable.get_untracked() != next {
            self.scrollable.set(next);
            self.dirty = true;
        }
        self.sync_follow();
    }

    fn update_mode_from_scroll(&mut self, next: MessageScrollerScrollable, top: f64) {
        let scrolled_up = top < self.last_scroll_top - SCROLL_EPSILON;
        self.last_scroll_top = top;
        if self.props.auto_scroll
            && !next.end
            && self.mode != Mode::SettlingJump
            && self.mode != Mode::AnchoredToMessage
        {
            self.mode = Mode::FollowingBottom;
        } else if self.mode == Mode::FollowingBottom && next.end && scrolled_up && !self.autoscrolling.get_untracked() {
            self.mode = Mode::FreeScrolling;
        }
    }

    /// The list follows its tail exactly while the engine follows the bottom.
    fn sync_follow(&mut self) {
        let follow = self.following();
        if self.list.follows_tail() != follow {
            self.list.follow_tail(follow);
            self.dirty = true;
        }
    }

    /// Whether row `row` is an anchor, for a row built this render.
    fn built_anchor(&self, row: u64) -> bool {
        row.checked_sub(self.built_first)
            .and_then(|j| usize::try_from(j).ok())
            .and_then(|j| self.built_anchors.get(j))
            .copied()
            .unwrap_or(false)
    }

    /// Vue's `computeVisibility`: what shows below the line a new turn settles at.
    fn sync_visibility(&mut self, l: &ListLayout) {
        let line = self.props.scroll_margin + self.props.scroll_previous_item_peek;
        let mut shown: Option<(u64, u64)> = None;
        let mut current = self.anchor_before;
        for r in &l.rows {
            let bottom = r.top + self.visual_height_of(r.row, r.height);
            if bottom > line && r.top < l.viewport {
                shown = Some(shown.map_or((r.row, r.row), |(a, _)| (a, r.row)));
            }
            if r.top <= line + local(SCROLL_EPSILON) && self.built_anchor(r.row) {
                current = Some(r.row);
            }
        }
        let index = |r: u64| usize::try_from(r).unwrap_or(usize::MAX);
        let next = MessageScrollerVisibility {
            current_anchor: current.map(index),
            visible: shown.map_or(0..0, |(a, b)| index(a)..index(b).saturating_add(1)),
        };
        if self.visibility.get_untracked() != next {
            self.visibility.set(next);
            self.dirty = true;
        }
    }

    fn set_autoscrolling(&mut self, active: bool) {
        self.autoscroll_run = self.autoscroll_run.wrapping_add(1);
        if self.autoscrolling.get_untracked() != active {
            self.autoscrolling.set(active);
            self.dirty = true;
            self.commit_scroll_state();
        }
        if active {
            let (me, run) = (self.me.clone(), self.autoscroll_run);
            self.owner.run(|| {
                clock::after(AUTOSCROLLING_TIMEOUT, move || {
                    if let Some(engine) = me.upgrade()
                        && let Ok(mut e) = engine.try_borrow_mut()
                        && e.autoscroll_run == run
                    {
                        e.autoscrolling.set(false);
                        e.commit_scroll_state();
                    }
                })
            });
        }
    }

    // ── Moving the viewport ────────────────────────────────────────────────

    /// A row's height without the gap below it.
    fn visual_height_of(&self, row: u64, height: f32) -> f32 {
        if row.saturating_add(1) < self.len {
            (height - self.gap).max(0.0)
        } else {
            height
        }
    }

    fn set_end_space(&mut self, height: f32) {
        let next = height.ceil().max(0.0);
        if self.list.end_space() != next {
            self.list.set_end_space(next);
            self.dirty = true;
        }
    }

    /// Put `s.row` at its edge with what is measured now, the end space
    /// letting it get there; whether the viewport shows it there already.
    fn place(&mut self, s: Settle) -> bool {
        let vp = f64::from(self.list.viewport_height());
        let top = self.list.offset_of(s.row);
        let h = f64::from(self.visual_height_of(s.row, self.list.row_height(s.row)));
        let m = f64::from(s.margin);
        let target = match s.edge {
            Edge::Start => top - m,
            Edge::Center => top - (vp - h) / 2.0 - m,
            Edge::End => top + h - vp + m,
        }
        .max(0.0);
        self.set_end_space(local(target + vp - self.list.extent()));
        if (self.list.view_top() - target).abs() <= SCROLL_EPSILON {
            return true;
        }
        self.list.scroll_to(Anchor {
            row: s.row,
            key: None,
            offset: local(target - top),
        });
        self.dirty = true;
        false
    }

    fn scroll_to_start(&mut self) -> bool {
        if !self.laid {
            return false;
        }
        self.set_end_space(0.0);
        self.settle = None;
        self.mode = Mode::FreeScrolling;
        if self.list.view_top() > SCROLL_EPSILON {
            self.list.scroll_to_row(0);
            self.dirty = true;
        }
        self.commit_scroll_state();
        true
    }

    fn scroll_to_end(&mut self) -> bool {
        if !self.laid {
            return false;
        }
        self.set_end_space(0.0);
        self.settle = None;
        self.mode = if self.props.auto_scroll {
            Mode::FollowingBottom
        } else {
            Mode::FreeScrolling
        };
        let end = (self.list.extent() - f64::from(self.list.viewport_height())).max(0.0);
        if (self.list.view_top() - end).abs() > SCROLL_EPSILON {
            self.set_autoscrolling(true);
            self.list.scroll_to_end();
            self.dirty = true;
        }
        self.commit_scroll_state();
        true
    }

    /// Bring message `row` to `options.align`; with `keep_previous_peek`, as a
    /// new turn, the one before it peeking in above and the turn held there
    /// while it grows.
    fn scroll_to_element(&mut self, row: u64, options: MessageScrollerScrollOptions, keep_previous_peek: bool) -> bool {
        if !self.laid || row >= self.list.len() {
            return false;
        }
        let margin = options.scroll_margin.unwrap_or(self.props.scroll_margin);
        let margin = if keep_previous_peek {
            margin + self.props.scroll_previous_item_peek
        } else {
            margin
        };
        self.mode = if keep_previous_peek {
            Mode::AnchoredToMessage
        } else {
            Mode::SettlingJump
        };
        let edge = match options.align {
            MessageScrollerScrollAlign::Start => Some(Edge::Start),
            MessageScrollerScrollAlign::Center => Some(Edge::Center),
            MessageScrollerScrollAlign::End => Some(Edge::End),
            MessageScrollerScrollAlign::Nearest => {
                let (view, vp) = (self.list.view_top(), f64::from(self.list.viewport_height()));
                let top = self.list.offset_of(row);
                let h = f64::from(self.visual_height_of(row, self.list.row_height(row)));
                if top >= view && top + h <= view + vp {
                    None
                } else if top < view {
                    Some(Edge::Start)
                } else {
                    Some(Edge::End)
                }
            }
        };
        self.settle = edge.map(|edge| Settle {
            row,
            edge,
            margin,
            hold: keep_previous_peek,
        });
        if let Some(s) = self.settle {
            self.place(s);
        }
        self.commit_scroll_state();
        true
    }

    fn scroll_to_message(&mut self, row: u64, options: MessageScrollerScrollOptions) -> bool {
        self.default_applied = true;
        if self.laid && self.len > 0 {
            if row >= self.len {
                return false;
            }
            self.pending = None;
            return self.scroll_to_element(row, options, false);
        }
        // Before the thread has a message laid out: placed once it has.
        self.pending = Some((row, options));
        true
    }

    fn flush_pending_scroll_to_message(&mut self) -> bool {
        let Some((row, options)) = self.pending else {
            return false;
        };
        if !self.scroll_to_element(row, options, false) {
            return false;
        }
        self.pending = None;
        self.default_applied = true;
        true
    }

    fn apply_default_scroll_position(&mut self) -> bool {
        if self.default_applied || self.len == 0 {
            return false;
        }
        let applied = match self.props.default_scroll_position {
            MessageScrollerDefaultScrollPosition::Start => self.scroll_to_start(),
            MessageScrollerDefaultScrollPosition::End => self.scroll_to_end(),
            MessageScrollerDefaultScrollPosition::LastAnchor => match self.last_anchor {
                Some(a) if self.list.extent() - self.list.offset_of(a) > f64::from(self.list.viewport_height()) => {
                    self.scroll_to_element(a, MessageScrollerScrollOptions::default(), true)
                }
                _ => self.scroll_to_end(),
            },
        };
        if applied {
            self.default_applied = true;
        }
        applied
    }
}

/// A thread's scroll state and commands, as the parts below a
/// [`message_scroller_provider`] read it ([`use_message_scroller`], Vue's
/// `useMessageScroller`). It is the provider's: it lives while the provider
/// is mounted.
#[derive(Clone)]
pub struct MessageScroller {
    list: ListState,
    engine: Rc<RefCell<Engine>>,
    scrollable: Signal<MessageScrollerScrollable>,
    visibility: Signal<MessageScrollerVisibility>,
    autoscrolling: Signal<bool>,
}

impl MessageScroller {
    /// Made in the provider's setup: what it makes belongs to the provider.
    fn new(props: MessageScrollerProps) -> Self {
        let list = ListState::new(ROW_ESTIMATE);
        list.follow_tail(props.auto_scroll);
        let scrollable = Signal::labeled("message scroller scrollable", MessageScrollerScrollable::default());
        let visibility = Signal::labeled("message scroller visibility", MessageScrollerVisibility::default());
        let autoscrolling = Signal::labeled("message scroller autoscrolling", false);
        let owner = gui::Scope::new();
        let engine = Rc::new_cyclic(|me| {
            RefCell::new(Engine {
                props,
                list: list.clone(),
                me: me.clone(),
                owner,
                scrollable,
                visibility,
                autoscrolling,
                gap: DEFAULT_GAP,
                len: 0,
                first: None,
                last: None,
                built_first: 0,
                built_anchors: Vec::new(),
                anchor_before: None,
                last_anchor: None,
                anchor_cache: None,
                change: None,
                laid: false,
                size: (0.0, 0.0, 0.0),
                shown_top: 0.0,
                last_scroll_top: 0.0,
                mode: if props.auto_scroll {
                    Mode::FollowingBottom
                } else {
                    Mode::FreeScrolling
                },
                settle: None,
                default_applied: false,
                pending: None,
                handled_anchors: HashSet::new(),
                autoscroll_run: 0,
                dirty: false,
            })
        });
        let weak = Rc::downgrade(&engine);
        list.on_layout(move |l| {
            weak.upgrade()
                .and_then(|e| e.try_borrow_mut().ok().map(|mut e| e.laid_out(l)))
                .unwrap_or(false)
        });
        Self {
            list,
            engine,
            scrollable,
            visibility,
            autoscrolling,
        }
    }

    /// The virtual list the thread is laid out by.
    #[must_use]
    pub fn list(&self) -> &ListState {
        &self.list
    }

    /// Whether there is more of the thread above and below (reactive).
    #[must_use]
    pub fn scrollable(&self) -> MessageScrollerScrollable {
        self.scrollable.get()
    }

    /// The messages in view and the anchored turn the reader is in (reactive).
    #[must_use]
    pub fn visibility(&self) -> MessageScrollerVisibility {
        self.visibility.get()
    }

    /// Jump to the start of the thread.
    pub fn scroll_to_start(&self) -> bool {
        self.engine.try_borrow_mut().is_ok_and(|mut e| e.scroll_to_start())
    }

    /// Jump to the end of the thread, following it from there with `auto_scroll`.
    pub fn scroll_to_end(&self) -> bool {
        self.engine.try_borrow_mut().is_ok_and(|mut e| e.scroll_to_end())
    }

    /// Bring message `index` (its place in the thread's data, built or not)
    /// into view; before the thread has a message, once it has. False when
    /// the thread has messages and none at `index`.
    pub fn scroll_to_message(&self, index: usize, options: MessageScrollerScrollOptions) -> bool {
        let row = u64::try_from(index).unwrap_or(u64::MAX);
        self.engine
            .try_borrow_mut()
            .is_ok_and(|mut e| e.scroll_to_message(row, options))
    }

    /// Take the provider's new props: those read as the thread moves (the
    /// peek, the margin, the edge threshold, following the end).
    fn set_props(&self, props: MessageScrollerProps) {
        self.list.follow_tail(props.auto_scroll);
        if let Ok(mut e) = self.engine.try_borrow_mut() {
            e.props = props;
        }
    }
}

/// The scroller of the [`message_scroller_provider`] above this instance
/// (Vue's `useMessageScroller`); call it in a component's setup.
#[must_use]
pub fn use_message_scroller() -> Option<MessageScroller> {
    use_context::<MessageScroller>()
}

/// The provider (Vue's `MessageScrollerProvider`), around `child`: it keeps
/// the thread's scroll state for the frame, viewport, button and any control
/// inside it, which read it through [`use_message_scroller`]. It draws
/// nothing of its own. Its props are read as they change; give it a new key
/// to start afresh (Vue's `:key`).
#[must_use]
pub fn message_scroller_provider(props: MessageScrollerProps, child: impl IntoElement) -> ComponentElement {
    component(
        ProviderProps {
            props,
            child: gui::Slot(child.into_element()),
        },
        |cx| {
            let props = cx.props();
            let scroller = MessageScroller::new(props.with_untracked(|p| p.props));
            provide(scroller.clone());
            let _props = watch(move || props.with(|p| p.props), move |_, new| scroller.set_props(*new));
            move || props.with(|p| p.child.0.clone())
        },
    )
}

/// What [`message_scroller_provider`] is given.
/// Its child is the parent's slot, so it never compares equal.
#[derive(Clone, PartialEq)]
struct ProviderProps {
    props: MessageScrollerProps,
    child: gui::Slot<Element>,
}

/// The frame: it clips the [`message_scroller_viewport`] and holds the
/// [`message_scroller_button`] over it.
#[must_use]
pub fn message_scroller() -> Div {
    div()
        .named("message-scroller")
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .size(pct(100.0))
        .min_height(0.0)
        .overflow(Overflow::Hidden)
}

type Meta<T> = Rc<dyn for<'a> Fn(&'a T) -> (&'a str, bool)>;
type Row<T> = Rc<dyn Fn(usize, &T) -> Div>;

/// What [`message_scroller_viewport`] is given.
pub struct MessageScrollerViewportProps<T: 'static> {
    items: Rc<[T]>,
    meta: Meta<T>,
    row: Row<T>,
    gap: f32,
    viewport: Rc<dyn Fn(Div) -> Div>,
}

impl<T: 'static> Clone for MessageScrollerViewportProps<T> {
    fn clone(&self) -> Self {
        Self {
            items: Rc::clone(&self.items),
            meta: Rc::clone(&self.meta),
            row: Rc::clone(&self.row),
            gap: self.gap,
            viewport: Rc::clone(&self.viewport),
        }
    }
}

impl<T: 'static> PartialEq for MessageScrollerViewportProps<T> {
    fn eq(&self, _: &Self) -> bool {
        // Its row builder is the parent's slot: made anew by each render.
        false
    }
}

/// The scrolling thread of `items` (Vue's viewport, content and items),
/// laid out by a virtual list: only the messages in view and a prefetch
/// margin are built. `meta` gives each message its id (unique in the
/// thread) and whether it is an anchor, a turn the thread settles near the
/// top when it comes in; `row(index, item)` builds a message, set in a
/// `message-scroller-item` row. The thread's gap is 32 px; change it with
/// [`MessageScrollerViewport::gap`].
#[must_use]
pub fn message_scroller_viewport<T: 'static>(
    items: impl Into<Rc<[T]>>,
    meta: impl for<'a> Fn(&'a T) -> (&'a str, bool) + 'static,
    row: impl Fn(usize, &T) -> Div + 'static,
) -> MessageScrollerViewport<T> {
    MessageScrollerViewport(MessageScrollerViewportProps {
        items: items.into(),
        meta: Rc::new(meta),
        row: Rc::new(row),
        gap: DEFAULT_GAP,
        viewport: Rc::new(|v| v),
    })
}

/// A [`message_scroller_viewport`] being built.
pub struct MessageScrollerViewport<T: 'static>(MessageScrollerViewportProps<T>);

impl<T: 'static> MessageScrollerViewport<T> {
    /// The gap between messages, in pixels.
    #[must_use]
    pub fn gap(mut self, gap: f32) -> Self {
        self.0.gap = gap;
        self
    }

    /// Style the scroll container (horizontal padding: its rows' offsets are
    /// its messages' boxes, so the thread takes no vertical padding).
    #[must_use]
    pub fn viewport(mut self, f: impl Fn(Div) -> Div + 'static) -> Self {
        self.0.viewport = Rc::new(f);
        self
    }
}

impl<T: 'static> IntoElement for MessageScrollerViewport<T> {
    fn into_element(self) -> Element {
        component(self.0, |cx| {
            let props = cx.props();
            let scroller = use_message_scroller();
            move || {
                let Some(state) = &scroller else {
                    return Element::Empty;
                };
                props.with(|p| viewport(state, p)).into_element()
            }
        })
        .into_element()
    }
}

fn viewport<T>(state: &MessageScroller, p: &MessageScrollerViewportProps<T>) -> Div {
    let items = &*p.items;
    let (meta, row, gap) = (&*p.meta, &*p.row, p.gap);
    let len = u64::try_from(items.len()).unwrap_or(u64::MAX);
    let at = |i: u64| usize::try_from(i).ok().and_then(|i| items.get(i));
    let key = |i: u64| at(i).map_or(ElementKey::default(), |t| key_of(meta(t).0));
    let anchor = |i: u64| at(i).is_some_and(|t| meta(t).1);
    if let Ok(mut e) = state.engine.try_borrow_mut() {
        e.data(len, gap, &key, &anchor);
    }
    state.list.hide_scrollbar(state.autoscrolling.get());
    let built = RefCell::new(Vec::new());
    let list = virtual_list_in(
        &state.list,
        len,
        key,
        |i| {
            let Some(t) = at(i) else { return Element::Empty };
            built.borrow_mut().push(meta(t).1);
            div()
                .named("message-scroller-item")
                .width(pct(100.0))
                .min_width(0.0)
                .flex_shrink(0.0)
                .when(i.saturating_add(1) < len, |d| d.padding_bottom(gap))
                .child(row(usize::try_from(i).unwrap_or(usize::MAX), t))
                .into_element()
        },
        // In flow, not over the list: the messages built give the thread its
        // width, as Vue's do (a card sized by its content is as wide as them).
        |v| {
            (p.viewport)(
                v.position(Position::Relative)
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .mask(ScrollFade {
                        axis: Axis::Vertical,
                        start: false,
                        end: true,
                        size: None,
                    }),
            )
        },
    );
    let window = state.list.window();
    if let Ok(mut e) = state.engine.try_borrow_mut() {
        e.built(window.start, built.into_inner(), &anchor);
    }
    list.named("message-scroller-viewport")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .size(pct(100.0))
        .min_height(0.0)
        .min_width(0.0)
}

/// Which end of the thread the button jumps to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MessageScrollerDirection {
    Start,
    #[default]
    End,
}

/// A round arrow button centred over the bottom (or top) edge, shown while
/// there is more of the thread that way, jumping there. Vue centres it with
/// `-translate-x-1/2`; here a full-width row that lets clicks through centres it.
#[must_use]
pub fn message_scroller_button(direction: MessageScrollerDirection) -> ComponentElement {
    component(direction, |cx| {
        let direction = cx.props();
        let scroller = use_message_scroller();
        move || {
            let Some(state) = scroller.clone() else {
                return Element::Empty;
            };
            scroll_button(state, direction.get()).into_element()
        }
    })
}

fn scroll_button(state: MessageScroller, direction: MessageScrollerDirection) -> Div {
    let t = theme();
    let scrollable = state.scrollable();
    let active = match direction {
        MessageScrollerDirection::Start => scrollable.start,
        MessageScrollerDirection::End => scrollable.end,
    };
    let arrow = icon(Icon::ArrowDown).when(direction == MessageScrollerDirection::Start, |i| i.rotate(180.0));
    let control = button(ButtonVariant::Secondary, ButtonSize::IconSm);
    let i = control.interaction();
    let control = control
        .named("message-scroller-button")
        .border(1.0)
        .border_color(t.border)
        .background(t.background)
        .text_color(t.foreground)
        .background_while(move || i.hovered(), t.muted)
        .focus_ring_on(i, &t)
        .transition(Properties::TRANSFORM | Properties::OPACITY)
        .when(active, |b| {
            b.duration_ms(200.0).ease(Easing::CubicBezier(0.23, 1.0, 0.32, 1.0))
        })
        .when(!active, |b| {
            // Out of the way, past the edge it points at.
            let off = match direction {
                MessageScrollerDirection::End => 1.0,
                MessageScrollerDirection::Start => -1.0,
            };
            b.duration_ms(400.0)
                .ease(Easing::CubicBezier(0.7, 0.0, 0.84, 0.0))
                .scale(0.95)
                .opacity(0.0)
                .translate_y(Calc::fraction(off))
                .pass_through()
                // `inert`, not `disabled`: hidden, not dimmed.
                .dim_disabled(i, 0.0)
        })
        .disabled(!active)
        .child(arrow)
        .on_click(move |_| {
            match direction {
                MessageScrollerDirection::Start => state.scroll_to_start(),
                MessageScrollerDirection::End => state.scroll_to_end(),
            };
        });
    let row = div()
        .position(Position::Absolute)
        .left(0.0)
        .right(0.0)
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .pass_through()
        .child(control);
    match direction {
        MessageScrollerDirection::End => row.bottom(16.0),
        MessageScrollerDirection::Start => row.top(16.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::layout::Content;
    use gui::{Scenario, text};
    use std::cell::RefCell;
    use std::rc::Rc;

    const WIDTH: u32 = 400;
    const HEIGHT: f32 = 600.0;

    #[derive(Clone, PartialEq)]
    struct Msg {
        id: String,
        user: bool,
        height: f32,
    }

    /// Message `i`: a user turn on even indices, 40 px tall; a reply 120 px.
    fn msg(i: usize) -> Msg {
        let user = i.is_multiple_of(2);
        Msg {
            id: format!("m{i}"),
            user,
            height: if user { 40.0 } else { 120.0 },
        }
    }

    fn thread_of(n: usize) -> Vec<Msg> {
        (0..n).map(msg).collect()
    }

    /// A 400×600 thread of `msgs` under a provider with `props`, settled,
    /// and the provider's scroller (as a control inside it reads it).
    fn scenario(props: MessageScrollerProps, msgs: Signal<Vec<Msg>>) -> (Scenario, MessageScroller) {
        let got: Rc<RefCell<Option<MessageScroller>>> = Rc::default();
        let into = Rc::clone(&got);
        let mut s = Scenario::measuring(
            move || {
                let into = Rc::clone(&into);
                let capture = component((), move |_| {
                    *into.borrow_mut() = use_message_scroller();
                    || Element::Empty
                });
                div()
                    .width(400.0)
                    .height(HEIGHT)
                    .child(message_scroller_provider(
                        props,
                        message_scroller()
                            .child(message_scroller_viewport(
                                msgs.get(),
                                |m: &Msg| (m.id.as_str(), m.user),
                                |_, m: &Msg| div().height(m.height).child(text(m.id.clone())),
                            ))
                            .child(capture),
                    ))
                    .into_element()
            },
            WIDTH,
            600,
            1.0,
        )
        .unwrap();
        s.settle().unwrap();
        s.settle().unwrap();
        let state = got.borrow().clone().unwrap();
        (s, state)
    }

    /// The messages built: id and the top of their box.
    fn built(s: &Scenario) -> Vec<(String, f32)> {
        s.last()
            .nodes
            .iter()
            .filter_map(|n| match &n.content {
                Content::Text { content, .. } if content.starts_with('m') => Some((content.to_string(), n.bounds.y)),
                _ => None,
            })
            .collect()
    }

    fn top_of(s: &Scenario, id: &str) -> Option<f32> {
        built(s).into_iter().find(|(m, _)| m == id).map(|(_, y)| y)
    }

    /// The message whose box holds the viewport's top, and its top.
    fn at_top(s: &Scenario, msgs: &[Msg]) -> (String, f32) {
        built(s)
            .into_iter()
            .find(|(id, y)| {
                let h = msgs.iter().find(|m| &m.id == id).map_or(0.0, |m| m.height);
                *y <= 0.0 && y + h > 0.0
            })
            .unwrap()
    }

    #[test]
    fn a_hundred_thousand_messages_build_only_the_rows_in_view() {
        let msgs = Signal::new(thread_of(100_000));
        let (s, state) = scenario(MessageScrollerProps::default(), msgs);
        let rows = built(&s);
        assert!(rows.len() < 40, "{} rows built", rows.len());
        // It opens at the end: the last message (a reply) ends at the viewport's.
        assert_eq!(top_of(&s, "m99999"), Some(HEIGHT - 120.0));
        assert_eq!(
            state.scrollable(),
            MessageScrollerScrollable {
                start: true,
                end: false
            }
        );
        let v = state.visibility();
        assert_eq!(v.visible.end, 100_000);
        // Tops from the end: m99998 at 408, m99996 at 216, m99994 at -8, the
        // last user turn at or above the 64 px line.
        assert_eq!(v.current_anchor, Some(99_994));
    }

    #[test]
    fn a_message_never_built_lands_exactly_at_its_edge() {
        let msgs = Signal::new(thread_of(100_000));
        let (mut s, state) = scenario(
            MessageScrollerProps {
                scroll_margin: 24.0,
                ..MessageScrollerProps::default()
            },
            msgs,
        );
        assert!(top_of(&s, "m50000").is_none());
        s.act("jump", || {
            assert!(state.scroll_to_message(50_000, MessageScrollerScrollOptions::default()));
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, "m50000"), Some(24.0));
        // The gap below the one before it is not part of it: 32 px above it.
        assert_eq!(top_of(&s, "m49999"), Some(24.0 - 32.0 - 120.0));
        assert_eq!(state.visibility().current_anchor, Some(50_000));
        s.settle().unwrap();
        assert_eq!(top_of(&s, "m50000"), Some(24.0));
        // To the bottom edge: the reply's own box, not its gap, ends 24 px above it.
        s.act("jump to the end edge", || {
            state.scroll_to_message(
                70_001,
                MessageScrollerScrollOptions {
                    align: MessageScrollerScrollAlign::End,
                    scroll_margin: None,
                },
            );
        });
        s.settle().unwrap();
        assert_eq!(top_of(&s, "m70001"), Some(HEIGHT - 24.0 - 120.0));
        assert!(built(&s).len() < 40);
    }

    #[test]
    fn a_streaming_reply_is_followed_until_the_reader_scrolls_up() {
        let msgs = Signal::new(thread_of(30));
        let (mut s, state) = scenario(
            MessageScrollerProps {
                auto_scroll: true,
                ..MessageScrollerProps::default()
            },
            msgs,
        );
        assert_eq!(top_of(&s, "m29"), Some(HEIGHT - 120.0));
        // A reply comes in after the last and grows.
        s.act("reply", || {
            msgs.update(|m| {
                m.push(Msg {
                    id: "m30".to_owned(),
                    user: false,
                    height: 20.0,
                });
            });
        });
        s.settle().unwrap();
        for h in [60.0, 200.0, 420.0, 900.0] {
            s.act("stream", || msgs.update(|m| m.last_mut().unwrap().height = h));
            s.settle().unwrap();
            assert_eq!(top_of(&s, "m30"), Some(HEIGHT - h), "at {h}");
            assert!(!state.scrollable().end);
        }
        // The reader wheels up: the reply grows below without moving the view.
        s.hover(200.0, 300.0);
        s.wheel(-300.0);
        s.settle().unwrap();
        let before = top_of(&s, "m30").unwrap();
        s.act("stream", || msgs.update(|m| m.last_mut().unwrap().height = 1000.0));
        s.settle().unwrap();
        assert_eq!(top_of(&s, "m30"), Some(before));
        assert!(state.scrollable().end);
        // Back to the end: followed again.
        s.act("to the end", || {
            state.scroll_to_end();
        });
        s.settle().unwrap();
        s.act("stream", || msgs.update(|m| m.last_mut().unwrap().height = 1100.0));
        s.settle().unwrap();
        assert_eq!(top_of(&s, "m30"), Some(HEIGHT - 1100.0));
    }

    #[test]
    fn a_new_anchored_turn_settles_with_a_peek_of_the_one_before() {
        let msgs = Signal::new(thread_of(20));
        let (mut s, state) = scenario(MessageScrollerProps::default(), msgs);
        s.act("send", || {
            msgs.update(|m| {
                m.push(msg(20));
                m.push(Msg {
                    height: 30.0,
                    ..msg(21)
                });
            });
        });
        s.settle().unwrap();
        // The turn's top at the peek line (64 px), held while its reply grows.
        assert_eq!(top_of(&s, "m20"), Some(64.0));
        assert_eq!(top_of(&s, "m19").map(|y| y + 120.0 + 32.0), Some(64.0));
        for h in [100.0, 300.0, 700.0] {
            s.act("stream", || msgs.update(|m| m.last_mut().unwrap().height = h));
            s.settle().unwrap();
            assert_eq!(top_of(&s, "m20"), Some(64.0), "at {h}");
        }
        assert_eq!(state.visibility().current_anchor, Some(20));
    }

    #[test]
    fn history_prepended_keeps_the_reader_on_the_same_message() {
        let all = thread_of(1_200);
        let msgs = Signal::new(all.get(1_000..).unwrap().to_vec());
        let (mut s, _) = scenario(MessageScrollerProps::default(), msgs);
        s.hover(200.0, 300.0);
        for _ in 0..5 {
            s.wheel(-170.0);
            s.settle().unwrap();
        }
        let (id, y) = at_top(&s, &all);
        s.act("history", || msgs.set(all.clone()));
        s.settle().unwrap();
        assert_eq!(top_of(&s, &id), Some(y), "{id}");
        s.settle().unwrap();
        assert_eq!(top_of(&s, &id), Some(y), "{id}");
        assert!(built(&s).len() < 40);
    }
}
