//! Hover card (`ui/hover-card`): a card that opens while the pointer rests
//! on its trigger (after 700ms) and closes 300ms after it leaves both the
//! trigger and the card.
//!
//! The parts nest as in Vue: [`hover_card`] is the anchor; it holds the
//! [`hover_card_trigger`] and the [`hover_card_content`].

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::{Div, Element, NamedKey, Placement, Side, SideAlign, Signal, deferred, div};

use gui::style::Origin;

use crate::motion::popper;
use crate::theme::theme;

/// An open flag that follows the pointer after a delay: the one rule behind
/// the hover card and the tooltip. A newer request replaces a pending one.
#[derive(Clone)]
pub(crate) struct HoverIntent {
    open: Signal<bool>,
    pending: Rc<Cell<Option<TimerId>>>,
}

impl HoverIntent {
    pub(crate) fn new() -> Self {
        Self {
            open: Signal::labeled("hover-open", false),
            pending: Rc::new(Cell::new(None)),
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Drop a change still waiting for its delay.
    pub(crate) fn cancel(&self) {
        if let Some(id) = self.pending.take() {
            clock::cancel(id);
        }
    }

    /// Open or close now, dropping any pending change.
    pub(crate) fn set_now(&self, open: bool) {
        self.cancel();
        if self.open.get() != open {
            self.open.set(open);
        }
    }

    /// Open or close once `delay` has passed, unless something else is asked first.
    pub(crate) fn set_after(&self, open: bool, delay: Duration) {
        if delay.is_zero() {
            self.set_now(open);
            return;
        }
        self.cancel();
        let me = self.clone();
        let id = clock::after(delay, move || {
            me.pending.set(None);
            if me.open.get() != open {
                me.open.set(open);
            }
        });
        self.pending.set(Some(id));
    }
}

/// Reka's `openDelay`: how long the pointer rests on the trigger before the card opens.
const OPEN_DELAY: Duration = Duration::from_millis(700);
/// Reka's `closeDelay`: how long after the pointer leaves before the card closes.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// Whether a hover card is open.
#[derive(Clone)]
pub struct HoverCardState {
    intent: HoverIntent,
    open_delay: Duration,
    close_delay: Duration,
}

impl Default for HoverCardState {
    fn default() -> Self {
        Self::new()
    }
}

impl HoverCardState {
    /// A closed hover card.
    #[must_use]
    pub fn new() -> Self {
        Self::with_delays(OPEN_DELAY, CLOSE_DELAY)
    }

    /// A closed hover card that opens `open` after the pointer rests on its
    /// trigger and closes `close` after it leaves (`:open-delay`, `:close-delay`).
    #[must_use]
    pub fn with_delays(open: Duration, close: Duration) -> Self {
        Self {
            intent: HoverIntent::new(),
            open_delay: open,
            close_delay: close,
        }
    }

    /// Whether the card shows (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.intent.is_open()
    }

    pub fn close(&self) {
        self.intent.set_now(false);
    }

    fn hovered(&self, inside: bool) {
        if inside {
            self.intent.set_after(true, self.open_delay);
        } else {
            self.intent.set_after(false, self.close_delay);
        }
    }
}

/// The anchor the card is placed against: wrap the trigger (and the content) in it.
#[must_use]
pub fn hover_card() -> Div {
    div().named("hover-card").inline_flex()
}

/// Make `trigger` open the card while the pointer rests on it (`HoverCardTrigger as-child`).
#[must_use]
pub fn hover_card_trigger(state: &HoverCardState, trigger: Div) -> Div {
    let state = state.clone();
    trigger.on_hover(move |inside| state.hovered(inside))
}

/// The card, below its anchor and centred on it, 4px away; `build` adds the
/// classes and children a caller gives `HoverCardContent`.
#[must_use]
pub fn hover_card_content(state: &HoverCardState, build: impl FnOnce(Div) -> Div) -> Element {
    hover_card_content_placed(state, Side::Bottom, SideAlign::Center, 4.0, build)
}

/// [`hover_card_content`] on `side` of its anchor, lined up by `align`,
/// `offset` away (`<HoverCardContent side align :side-offset>`; a negative
/// offset overlaps the anchor).
#[must_use]
pub fn hover_card_content_placed(
    state: &HoverCardState,
    side: Side,
    align: SideAlign,
    offset: f32,
    build: impl FnOnce(Div) -> Div,
) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    let on_hover = state.clone();
    let on_key = state.clone();
    let on_outside = state.clone();
    let content = popper(div(), Origin::CENTER)
        .named("hover-card-content")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .w(64.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(4.0)
        .shadow_md()
        .on_hover(move |inside| {
            // Over the card it stays open; leaving it starts the close delay.
            if inside {
                on_hover.intent.cancel();
            } else {
                on_hover.hovered(false);
            }
        })
        .on_key_down(move |e| {
            let escape = e.is(NamedKey::Escape);
            if escape {
                on_key.close();
            }
            escape
        })
        .on_click_outside(move || on_outside.close());
    deferred(
        Placement::Anchored {
            side,
            align,
            offset,
            match_width: false,
            fit_height: false,
        },
        build(content),
    )
    .into_element()
}
