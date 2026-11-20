//! Message scroller (`ui/message-scroller`): the scrolling frame of a chat
//! thread. Its engine (`useMessageScroller`) keeps the thread pinned to the
//! bottom while replies stream and backs off when the reader scrolls up,
//! settles a new turn near the top with a peek of the one before, opens where
//! asked, keeps the reader's place when history is prepended, and says which
//! way there is more to read and which turn is in view.
//!
//! The engine reads the viewport through a [`ScrollHandle`] after each layout,
//! where Vue reads the DOM in its observers: what it changes (the offset, the
//! spacer that lets the last turn scroll to the top) is laid out again before
//! the frame is painted. Vue's viewport and content are one box here, since a
//! handle reports where a scroll container's own children lie. Scrolls jump:
//! `gui` animates no scroll, so `behavior: "smooth"` lands at once.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gui::prelude::*;
use gui::style::Calc;
use gui::{Div, Easing, ElementKey, Icon, Properties, ScrollHandle, ScrollLayout, Signal, clock, div, icon, key_of};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::theme::theme;

const SCROLL_EPSILON: f32 = 0.5;
const AUTOSCROLLING_TIMEOUT: Duration = Duration::from_millis(180);
/// The content's row gap until two items show the one the caller set (`gap-8`).
const DEFAULT_GAP: f32 = 32.0;
const SPACER: &str = "message-scroller-spacer";

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

/// How [`MessageScrollerState::scroll_to_message`] places a message.
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

/// The messages in view and the anchored turn the reader is in.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MessageScrollerVisibility {
    pub current_anchor_id: Option<String>,
    pub visible_message_ids: Vec<String>,
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

/// One item as laid out, in the content's coordinates.
#[derive(Clone, Debug)]
struct Item {
    key: ElementKey,
    id: String,
    anchor: bool,
    top: f32,
    height: f32,
}

impl Item {
    fn bottom(&self) -> f32 {
        self.top + self.height
    }
}

/// The viewport as last laid out, with the offset asked for since.
#[derive(Clone, Debug, Default)]
struct View {
    height: f32,
    top: f32,
    /// The content's padding at the start and the end.
    padding: (f32, f32),
    items: Vec<Item>,
}

impl View {
    /// How tall the items make the content, the spacer left out.
    fn content_height(&self) -> f32 {
        let (start, end) = self.padding;
        self.items.iter().fold(start + end, |h, i| h.max(i.bottom() + end))
    }

    fn item(&self, key: ElementKey) -> Option<&Item> {
        self.items.iter().find(|i| i.key == key)
    }

    fn index(&self, key: ElementKey) -> Option<usize> {
        self.items.iter().position(|i| i.key == key)
    }

    /// The offset that brings `item` to `align`, `margin` from the edge.
    fn top_for(&self, item: &Item, align: MessageScrollerScrollAlign, margin: f32) -> f32 {
        let (start, end) = self.padding;
        match align {
            MessageScrollerScrollAlign::Start => item.top - start - margin,
            MessageScrollerScrollAlign::Center => {
                let available = (self.height - start - end).max(0.0);
                item.top - start - (available - item.height) / 2.0 - margin
            }
            MessageScrollerScrollAlign::End => item.top - self.height + item.height + end + margin,
            MessageScrollerScrollAlign::Nearest => {
                let (visible_top, visible_bottom) = (self.top + start, self.top + self.height - end);
                if item.top >= visible_top && item.bottom() <= visible_bottom {
                    self.top
                } else if item.top < visible_top {
                    item.top - start - margin
                } else {
                    item.bottom() - self.height + end + margin
                }
            }
        }
    }
}

/// Vue's `createEngine`: the scroller's state between layouts.
struct Engine {
    props: MessageScrollerProps,
    handle: ScrollHandle,
    me: Weak<RefCell<Engine>>,
    scrollable: Signal<MessageScrollerScrollable>,
    visibility: Signal<MessageScrollerVisibility>,
    autoscrolling: Signal<bool>,
    /// The items built this frame: id and whether each is an anchor.
    registered: HashMap<ElementKey, (String, bool)>,
    view: View,
    laid: bool,
    /// What the last layout showed, to tell what changed since: the items,
    /// and the viewport's and the content's size.
    keys: Vec<ElementKey>,
    size: (f32, f32, f32),
    shown_top: f32,
    gap: f32,
    spacer: f32,
    mode: Mode,
    streaming_turn: Option<ElementKey>,
    first_item: Option<ElementKey>,
    item_count: usize,
    last_scroll_top: f32,
    default_applied: bool,
    prepend_restore: Option<(ElementKey, f32)>,
    pending: Option<(String, MessageScrollerScrollOptions)>,
    handled_anchors: HashSet<ElementKey>,
    /// Bumped by each programmatic scroll; the timer that ends autoscrolling
    /// acts only for the last.
    autoscroll_run: u64,
    /// The frame must be laid out again.
    dirty: bool,
}

impl Engine {
    // ── After layout ────────────────────────────────────────────────────────

    /// What the observers and the scroll events of Vue's viewport react to.
    fn laid_out(&mut self, l: &ScrollLayout) -> bool {
        self.dirty = false;
        if self.handle.take_wheeled() {
            self.user_scroll_intent();
        }
        let spacer = key_of(SPACER);
        let items: Vec<Item> = l
            .children
            .iter()
            .filter_map(|c| {
                let key = c.key.filter(|k| *k != spacer)?;
                let (id, anchor) = self.registered.get(&key)?.clone();
                Some(Item {
                    key,
                    id,
                    anchor,
                    top: c.bounds.y,
                    height: c.bounds.height,
                })
            })
            .collect();
        if let [a, b, ..] = items.as_slice() {
            self.gap = b.top - a.bottom();
        }
        let first = !self.laid;
        self.laid = true;
        self.view = View {
            height: l.bounds.height,
            top: l.offset.1,
            padding: (l.padding.top, l.padding.bottom),
            items,
        };
        let keys: Vec<ElementKey> = self.view.items.iter().map(|i| i.key).collect();
        self.registered.retain(|k, _| keys.contains(k));
        let size = (l.bounds.width, l.bounds.height, l.max.1 + l.bounds.height);
        let scrolled = (l.offset.1 - self.shown_top).abs() > SCROLL_EPSILON;
        self.shown_top = l.offset.1;

        if keys != self.keys {
            self.keys = keys;
            self.size = size;
            self.handle_content_change();
        } else if size != self.size {
            self.size = size;
            self.handle_resize();
        }
        if scrolled {
            self.sync_after_scroll();
        }
        if first {
            // Vue's provider, once mounted.
            self.apply_default_scroll_position();
            self.sync_after_scroll();
        }
        if self.pending.is_some() && self.flush_pending_scroll_to_message() {
            self.capture_prepend_anchor();
        }
        self.sync_visibility();
        self.dirty
    }

    fn handle_content_change(&mut self) {
        let previous_count = self.item_count;
        let previous_first = self.first_item;
        self.item_count = self.view.items.len();
        self.first_item = self.view.items.first().map(|i| i.key);
        self.apply_content_change(previous_count, previous_first);
        self.capture_prepend_anchor();
    }

    fn apply_content_change(&mut self, previous_count: usize, previous_first: Option<ElementKey>) {
        if self.flush_pending_scroll_to_message() {
            return;
        }
        let auto = self.props.auto_scroll;
        if previous_count == 0 {
            if self.apply_default_scroll_position() || (!self.view.items.is_empty() && auto && self.scroll_to_end()) {
                return;
            }
            self.commit_scroll_state();
            return;
        }
        // Earlier messages came in above: keep the reader's place.
        if previous_first.and_then(|k| self.view.index(k)).is_some_and(|i| i > 0) {
            self.apply_prepend_restore();
            return;
        }
        let added = self.view.items.get(previous_count..).unwrap_or_default();
        let len = self.view.items.len();
        if len > previous_count
            && let Some(anchor) = added.iter().find(|i| i.anchor).map(|i| i.key)
        {
            if auto && self.mode == Mode::FollowingBottom && added.iter().filter(|i| i.anchor).count() > 1 {
                self.scroll_to_end();
                return;
            }
            self.scroll_to_element(anchor, MessageScrollerScrollOptions::default(), true);
            self.handled_anchors.insert(anchor);
            return;
        }
        if len == previous_count
            && let Some(anchor) = self
                .view
                .items
                .iter()
                .find(|i| i.anchor && !self.handled_anchors.contains(&i.key))
                .map(|i| i.key)
        {
            self.scroll_to_element(anchor, MessageScrollerScrollOptions::default(), true);
            self.handled_anchors.insert(anchor);
            return;
        }
        if self.mode == Mode::FollowingBottom && auto {
            self.scroll_to_end();
        } else {
            self.commit_scroll_state();
        }
    }

    fn handle_resize(&mut self) {
        if self.mode == Mode::FollowingBottom && self.props.auto_scroll {
            self.scroll_to_end();
            return;
        }
        let previous_spacer = self.spacer;
        if self.reanchor_to_anchored_message() {
            if self.props.auto_scroll && previous_spacer > 0.0 && self.spacer == 0.0 {
                self.scroll_to_end();
            }
            return;
        }
        self.commit_scroll_state();
    }

    fn sync_after_scroll(&mut self) {
        self.commit_scroll_state();
        self.capture_prepend_anchor();
    }

    fn user_scroll_intent(&mut self) {
        if self.mode != Mode::FreeScrolling {
            self.streaming_turn = None;
            self.mode = Mode::FreeScrolling;
        }
    }

    // ── State the view reads ───────────────────────────────────────────────

    fn commit_scroll_state(&mut self) {
        let v = &self.view;
        let edge = self.props.scroll_edge_threshold;
        let measured = if self.laid {
            MessageScrollerScrollable {
                start: v.top > edge,
                end: v.content_height() - v.top - v.height > edge,
            }
        } else {
            MessageScrollerScrollable::default()
        };
        self.update_mode_from_scroll(measured);
        let next = if self.mode == Mode::FollowingBottom {
            MessageScrollerScrollable { end: false, ..measured }
        } else {
            measured
        };
        if self.scrollable.get() != next {
            self.scrollable.set(next);
            self.dirty = true;
        }
    }

    fn update_mode_from_scroll(&mut self, next: MessageScrollerScrollable) {
        let top = self.view.top;
        let scrolled_up = top < self.last_scroll_top - SCROLL_EPSILON;
        self.last_scroll_top = top;
        if self.props.auto_scroll
            && !next.end
            && self.mode != Mode::SettlingJump
            && self.mode != Mode::AnchoredToMessage
        {
            self.mode = Mode::FollowingBottom;
        } else if self.mode == Mode::FollowingBottom && next.end && scrolled_up && !self.autoscrolling.get() {
            self.mode = Mode::FreeScrolling;
        }
    }

    /// Vue's `computeVisibility`: what shows below the line a new turn settles at.
    fn sync_visibility(&mut self) {
        let v = &self.view;
        let line = self.props.scroll_margin + self.props.scroll_previous_item_peek;
        let mut next = MessageScrollerVisibility::default();
        for item in &v.items {
            let top = item.top - v.top;
            if item.bottom() - v.top > line && top < v.height {
                next.visible_message_ids.push(item.id.clone());
            }
            if item.anchor && top <= line + SCROLL_EPSILON {
                next.current_anchor_id = Some(item.id.clone());
            }
        }
        if self.visibility.get() != next {
            self.visibility.set(next);
            self.dirty = true;
        }
    }

    fn set_autoscrolling(&mut self, active: bool) {
        self.autoscroll_run = self.autoscroll_run.wrapping_add(1);
        if self.autoscrolling.get() != active {
            self.autoscrolling.set(active);
            self.dirty = true;
            self.commit_scroll_state();
        }
        if active {
            let (me, run) = (self.me.clone(), self.autoscroll_run);
            clock::after(AUTOSCROLLING_TIMEOUT, move || {
                if let Some(engine) = me.upgrade()
                    && let Ok(mut e) = engine.try_borrow_mut()
                    && e.autoscroll_run == run
                {
                    e.autoscrolling.set(false);
                    e.commit_scroll_state();
                }
            });
        }
    }

    // ── Moving the viewport ────────────────────────────────────────────────

    fn set_spacer_height(&mut self, height: f32) {
        let next = height.ceil().max(0.0);
        if self.spacer != next {
            self.spacer = next;
            self.dirty = true;
        }
    }

    /// Put the viewport at `top`, as far as the content and spacer let it go.
    fn scroll_to(&mut self, top: f32, autoscrolling: bool) {
        let max = (self.view.content_height() + self.spacer - self.view.height).max(0.0);
        let target = top.clamp(0.0, max);
        if (self.view.top - target).abs() <= SCROLL_EPSILON {
            self.view.top = target;
            self.commit_scroll_state();
            return;
        }
        if autoscrolling {
            self.set_autoscrolling(true);
        }
        self.view.top = target;
        self.handle.scroll_to((0.0, target));
        self.dirty = true;
    }

    fn scroll_to_start(&mut self) -> bool {
        if !self.laid {
            return false;
        }
        self.set_spacer_height(0.0);
        self.streaming_turn = None;
        self.mode = Mode::FreeScrolling;
        self.scroll_to(0.0, false);
        true
    }

    fn scroll_to_end(&mut self) -> bool {
        if !self.laid {
            return false;
        }
        self.set_spacer_height(0.0);
        self.streaming_turn = None;
        self.mode = if self.props.auto_scroll {
            Mode::FollowingBottom
        } else {
            Mode::FreeScrolling
        };
        let end = (self.view.content_height() - self.view.height).max(0.0);
        self.scroll_to(end, true);
        true
    }

    /// Bring item `key` to `options.align`; with `keep_previous_peek`, as a new
    /// turn, the one before it peeking in above and the turn held there while it grows.
    fn scroll_to_element(
        &mut self,
        key: ElementKey,
        options: MessageScrollerScrollOptions,
        keep_previous_peek: bool,
    ) -> bool {
        let Some(item) = self.view.item(key).cloned() else {
            return false;
        };
        let margin = options.scroll_margin.unwrap_or(self.props.scroll_margin);
        let margin = if keep_previous_peek {
            margin + self.props.scroll_previous_item_peek
        } else {
            margin
        };
        let target = self.view.top_for(&item, options.align, margin);
        self.set_spacer_height(target + self.view.height - self.view.content_height());
        self.prepend_restore = Some((key, item.top - self.view.top));
        self.mode = if keep_previous_peek {
            Mode::AnchoredToMessage
        } else {
            Mode::SettlingJump
        };
        self.streaming_turn = keep_previous_peek.then_some(key);
        self.scroll_to(target, false);
        true
    }

    fn reanchor_to_anchored_message(&mut self) -> bool {
        match self.streaming_turn {
            Some(key) if self.mode == Mode::AnchoredToMessage && self.view.item(key).is_some() => {
                self.scroll_to_element(key, MessageScrollerScrollOptions::default(), true)
            }
            _ => false,
        }
    }

    fn scroll_to_message(&mut self, id: &str, options: MessageScrollerScrollOptions) -> bool {
        let key = key_of(id);
        if self.registered.contains_key(&key) {
            self.default_applied = true;
            if self.scroll_to_element(key, options, false) {
                self.pending = None;
            } else {
                // Built but not laid out yet: placed after the next layout.
                self.pending = Some((id.to_owned(), options));
            }
            return true;
        }
        if self.item_count == 0 {
            self.pending = Some((id.to_owned(), options));
            self.default_applied = true;
            return true;
        }
        false
    }

    fn flush_pending_scroll_to_message(&mut self) -> bool {
        let Some((id, options)) = self.pending.clone() else {
            return false;
        };
        if !self.scroll_to_element(key_of(&id), options, false) {
            return false;
        }
        self.pending = None;
        self.default_applied = true;
        true
    }

    fn apply_prepend_restore(&mut self) -> bool {
        let Some((key, was)) = self.prepend_restore else {
            return false;
        };
        let Some(item) = self.view.item(key).cloned() else {
            return false;
        };
        let delta = item.top - self.view.top - was;
        if delta.abs() <= SCROLL_EPSILON {
            return false;
        }
        self.scroll_to(self.view.top + delta, false);
        self.prepend_restore = Some((key, item.top - self.view.top));
        true
    }

    /// Remember the first message in view and where it shows, to keep it there
    /// when earlier ones come in above.
    fn capture_prepend_anchor(&mut self) {
        let v = &self.view;
        self.prepend_restore = v
            .items
            .iter()
            .find(|i| i.bottom() - v.top > 0.0 && i.top - v.top < v.height)
            .map(|i| (i.key, i.top - v.top));
    }

    fn apply_default_scroll_position(&mut self) -> bool {
        if self.default_applied || self.item_count == 0 {
            return false;
        }
        let applied = match self.props.default_scroll_position {
            MessageScrollerDefaultScrollPosition::Start => self.scroll_to_start(),
            MessageScrollerDefaultScrollPosition::End => self.scroll_to_end(),
            MessageScrollerDefaultScrollPosition::LastAnchor => {
                match self.view.items.iter().rev().find(|i| i.anchor).cloned() {
                    Some(anchor) if self.view.content_height() - anchor.top > self.view.height => {
                        self.scroll_to_element(anchor.key, MessageScrollerScrollOptions::default(), true)
                    }
                    _ => self.scroll_to_end(),
                }
            }
        };
        if applied {
            self.default_applied = true;
        }
        applied
    }
}

/// A thread's scroll state, shared by its frame, viewport, items and button
/// (Vue's `MessageScrollerProvider`). Clones share it.
#[derive(Clone)]
pub struct MessageScrollerState {
    handle: ScrollHandle,
    engine: Rc<RefCell<Engine>>,
    scrollable: Signal<MessageScrollerScrollable>,
    visibility: Signal<MessageScrollerVisibility>,
    autoscrolling: Signal<bool>,
}

impl MessageScrollerState {
    #[must_use]
    pub fn new(props: MessageScrollerProps) -> Self {
        let handle = ScrollHandle::new();
        let scrollable = Signal::labeled("message scroller scrollable", MessageScrollerScrollable::default());
        let visibility = Signal::labeled("message scroller visibility", MessageScrollerVisibility::default());
        let autoscrolling = Signal::labeled("message scroller autoscrolling", false);
        let engine = Rc::new_cyclic(|me| {
            RefCell::new(Engine {
                props,
                handle: handle.clone(),
                me: me.clone(),
                scrollable: scrollable.clone(),
                visibility: visibility.clone(),
                autoscrolling: autoscrolling.clone(),
                registered: HashMap::new(),
                view: View::default(),
                laid: false,
                keys: Vec::new(),
                size: (0.0, 0.0, 0.0),
                shown_top: 0.0,
                gap: DEFAULT_GAP,
                spacer: 0.0,
                mode: if props.auto_scroll {
                    Mode::FollowingBottom
                } else {
                    Mode::FreeScrolling
                },
                streaming_turn: None,
                first_item: None,
                item_count: 0,
                last_scroll_top: 0.0,
                default_applied: false,
                prepend_restore: None,
                pending: None,
                handled_anchors: HashSet::new(),
                autoscroll_run: 0,
                dirty: false,
            })
        });
        let weak = Rc::downgrade(&engine);
        handle.on_layout(move |l| {
            weak.upgrade()
                .and_then(|e| e.try_borrow_mut().ok().map(|mut e| e.laid_out(l)))
                .unwrap_or(false)
        });
        Self {
            handle,
            engine,
            scrollable,
            visibility,
            autoscrolling,
        }
    }

    /// Whether there is more of the thread above and below.
    #[must_use]
    pub fn scrollable(&self) -> MessageScrollerScrollable {
        self.scrollable.get()
    }

    /// The messages in view and the anchored turn the reader is in.
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

    /// Bring message `id` into view; before the thread has any item, once it has.
    pub fn scroll_to_message(&self, id: &str, options: MessageScrollerScrollOptions) -> bool {
        self.engine
            .try_borrow_mut()
            .is_ok_and(|mut e| e.scroll_to_message(id, options))
    }

    /// Change how much of the turn before stays in view above a new one.
    pub fn set_scroll_previous_item_peek(&self, peek: f32) {
        if let Ok(mut e) = self.engine.try_borrow_mut() {
            e.props.scroll_previous_item_peek = peek;
        }
    }

    fn register(&self, id: &str, anchor: bool) -> ElementKey {
        let key = key_of(id);
        if let Ok(mut e) = self.engine.try_borrow_mut() {
            e.registered.insert(key, (id.to_owned(), anchor));
        }
        key
    }

    /// The spacer's height and the margin that cancels the gap above it.
    fn spacer(&self) -> (f32, f32) {
        self.engine
            .try_borrow()
            .map_or((0.0, DEFAULT_GAP), |e| (e.spacer, e.gap))
    }
}

/// The frame: it clips the [`message_scroller_viewport`] and holds the
/// [`message_scroller_button`] over it.
#[must_use]
pub fn message_scroller() -> Div {
    div()
        .named("message-scroller")
        .relative()
        .flex_col()
        .size_full()
        .min_h(0.0)
        .overflow_hidden()
}

/// The scrolling column of [`message_scroller_item`]s (Vue's viewport and
/// content in one box), with the spacer below them that lets the last turn
/// scroll to the top. The thread's own padding and gap go on it.
#[must_use]
pub fn message_scroller_viewport(state: &MessageScrollerState, items: impl IntoIterator<Item = Div>) -> Div {
    let (spacer, gap) = state.spacer();
    div()
        .named("message-scroller-viewport")
        .size_full()
        .min_h(0.0)
        .min_w(0.0)
        .overflow_y_scroll()
        .scroll_fade_b()
        .when(state.autoscrolling.get(), Styled::scrollbar_none)
        .track_scroll(&state.handle)
        .flex_col()
        .gap(8.0)
        .children(items)
        .when(spacer > 0.0, |d| {
            d.child(
                div()
                    .key(key_of(SPACER))
                    .shrink_0()
                    .h_px(spacer)
                    .mt(-gap / gui::styled::UNIT),
            )
        })
}

/// One message of the thread, `id` unique in it. An `anchor` is a turn the
/// thread settles near the top when it comes in.
#[must_use]
pub fn message_scroller_item(state: &MessageScrollerState, id: &str, anchor: bool) -> Div {
    div()
        .named("message-scroller-item")
        .key(state.register(id, anchor))
        .min_w(0.0)
        .shrink_0()
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
pub fn message_scroller_button(state: &MessageScrollerState, direction: MessageScrollerDirection) -> Div {
    let t = theme();
    let scrollable = state.scrollable();
    let active = match direction {
        MessageScrollerDirection::Start => scrollable.start,
        MessageScrollerDirection::End => scrollable.end,
    };
    let arrow = icon(Icon::ArrowDown).when(direction == MessageScrollerDirection::Start, |i| i.rotate(180.0));
    let s = state.clone();
    let control = button(ButtonVariant::Secondary, ButtonSize::IconSm)
        .named("message-scroller-button")
        .border()
        .border_color(t.border)
        .bg(t.background)
        .text_color(t.foreground)
        .hover(move |s| s.bg(t.muted).text_color(t.foreground))
        .transition_props(Properties::TRANSFORM | Properties::OPACITY)
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
                .disabled_style(|s| s.opacity(0.0))
        })
        .disabled(!active)
        .child(arrow)
        .on_click(move |_| {
            match direction {
                MessageScrollerDirection::Start => s.scroll_to_start(),
                MessageScrollerDirection::End => s.scroll_to_end(),
            };
        });
    let row = div()
        .absolute()
        .left(0.0)
        .right(0.0)
        .flex()
        .justify_center()
        .pass_through()
        .child(control);
    match direction {
        MessageScrollerDirection::End => row.bottom(4.0),
        MessageScrollerDirection::Start => row.top(4.0),
    }
}
