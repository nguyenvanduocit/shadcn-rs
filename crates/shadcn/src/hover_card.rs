//! Hover card (`ui/hover-card`): a card that opens while the pointer rests
//! on its trigger (after 700ms) and closes 300ms after it leaves both the
//! trigger and the card.
//!
//! The parts nest as in Vue: [`hover_card`] is the anchor and keeps whether
//! the card is open, its own (`default_open`) or the caller's (`open`,
//! `v-model:open`), and its delays (`open_delay`, `close_delay`); it holds
//! the [`hover_card_trigger`] and the [`hover_card_content`].
//!
//! ```ignore
//! hover_card()
//!     .child(hover_card_trigger(button(ButtonVariant::Link, ButtonSize::Default).child("@nuxt")))
//!     .child(hover_card_content().w(80.0).child("…"))
//! ```

use std::time::Duration;

use gui::clock;
use gui::prelude::*;
use gui::{
    Bind, Div, Element, Model, NamedKey, Placement, Scope, Side, SideAlign, Signal, UseModel, component, deferred, div,
    provide, use_context, use_model,
};

use gui::style::Origin;

use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// An open model that follows the pointer after a delay: the one rule behind
/// the hover card and the tooltip. A newer request replaces a pending one.
/// Made in a root's setup, so a pending change belongs to the instance and
/// goes with it.
pub(crate) struct HoverIntent<P: 'static> {
    pub(crate) open: UseModel<P, bool>,
    /// The instance's scope, which owns each pending change's.
    owner: Scope,
    /// The scope of the change waiting for its delay.
    pending: Signal<Option<Scope>>,
}

impl<P: 'static> Clone for HoverIntent<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for HoverIntent<P> {}

impl<P: 'static> HoverIntent<P> {
    /// In a root's setup: the intent over `open`.
    #[track_caller]
    pub(crate) fn new(open: UseModel<P, bool>) -> Self {
        Self {
            open,
            owner: Scope::new(),
            pending: Signal::labeled("hover-pending", None),
        }
    }

    /// Whether it is open (tracked).
    pub(crate) fn is_open(self) -> bool {
        self.open.get()
    }

    /// Drop a change still waiting for its delay.
    pub(crate) fn cancel(self) {
        if let Some(s) = self.pending.get_untracked() {
            self.pending.set(None);
            s.dispose();
        }
    }

    /// Open or close now, dropping any pending change.
    pub(crate) fn set_now(self, open: bool) {
        self.cancel();
        self.open.set(open);
    }

    /// Open or close once `delay` has passed, unless something else is asked first.
    pub(crate) fn set_after(self, open: bool, delay: Duration) {
        if delay.is_zero() {
            self.set_now(open);
            return;
        }
        self.cancel();
        if !self.owner.is_alive() {
            return;
        }
        let s = self.owner.run(Scope::new);
        s.run(|| {
            clock::after(delay, move || self.open.set(open));
        });
        self.pending.set(Some(s));
    }
}

/// Reka's `openDelay`: how long the pointer rests on the trigger before the card opens.
const OPEN_DELAY: Duration = Duration::from_millis(700);
/// Reka's `closeDelay`: how long after the pointer leaves before the card closes.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

/// What [`hover_card`] is given.
#[derive(Clone, PartialEq)]
pub struct HoverCardProps {
    open: Bind<bool>,
    open_delay: Duration,
    close_delay: Duration,
}

/// What the parts below a [`hover_card`] read.
#[derive(Clone, Copy)]
struct HoverCardContext {
    intent: HoverIntent<PartProps<HoverCardProps>>,
    props: Signal<PartProps<HoverCardProps>>,
}

impl HoverCardContext {
    /// The pointer entered (`true`) or left the trigger or the card.
    fn hovered(self, inside: bool) {
        let (open, close) = self.props.with_untracked(|p| (p.props.open_delay, p.props.close_delay));
        if inside {
            self.intent.set_after(true, open);
        } else {
            self.intent.set_after(false, close);
        }
    }
}

/// The anchor the card is placed against: wrap the trigger (and the
/// content) in it. Closed, and kept by itself, until told otherwise.
#[must_use]
pub fn hover_card() -> Part<HoverCardProps> {
    let props = HoverCardProps {
        open: Bind::default(),
        open_delay: OPEN_DELAY,
        close_delay: CLOSE_DELAY,
    };
    Part::new(props, div().named("hover-card").inline_flex(), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(HoverCardContext {
                intent: HoverIntent::new(use_model(props, |p| &p.props.open)),
                props,
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl Part<HoverCardProps> {
    /// Open or not, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }

    /// Open from the start, when no model is given (`default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_default(open))
    }

    /// Call `f` each time it opens or closes (`@update:open`).
    #[must_use]
    pub fn on_open_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// How long the pointer rests on the trigger before it opens (`:open-delay`, 700ms).
    #[must_use]
    pub fn open_delay(self, delay: Duration) -> Self {
        self.with_props(|p| p.open_delay = delay)
    }

    /// How long after the pointer leaves before it closes (`:close-delay`, 300ms).
    #[must_use]
    pub fn close_delay(self, delay: Duration) -> Self {
        self.with_props(|p| p.close_delay = delay)
    }
}

/// Make `trigger` open the card while the pointer rests on it (`HoverCardTrigger as-child`).
#[must_use]
pub fn hover_card_trigger(trigger: Div) -> Part<()> {
    Part::new((), trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let card = use_context::<HoverCardContext>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match card {
                    Some(c) => root.on_hover(move |inside| c.hovered(inside)),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// What [`hover_card_content`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverCardContentProps {
    side: Side,
    align: SideAlign,
    offset: f32,
}

impl Part<HoverCardContentProps> {
    /// On `side` of its anchor (`side="…"`; below by default).
    #[must_use]
    pub fn side(self, side: Side) -> Self {
        self.with_props(|p| p.side = side)
    }

    /// Lined up with its anchor by `align` (`align="…"`; centred by default).
    #[must_use]
    pub fn align(self, align: SideAlign) -> Self {
        self.with_props(|p| p.align = align)
    }

    /// How far from its anchor (`:side-offset`, 4px; a negative one overlaps it).
    #[must_use]
    pub fn side_offset(self, offset: f32) -> Self {
        self.with_props(|p| p.offset = offset)
    }
}

/// The card, below its anchor and centred on it, 4px away
/// (`HoverCardContent`); style and fill it as a `Div`. Over the card it
/// stays open; Escape or a press outside closes it.
#[must_use]
pub fn hover_card_content() -> Part<HoverCardContentProps> {
    let t = theme();
    let root = popper(div(), Origin::CENTER)
        .named("hover-card-content")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .w(64.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .p(4.0)
        .shadow_md();
    let props = HoverCardContentProps {
        side: Side::Bottom,
        align: SideAlign::Center,
        offset: 4.0,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let card = use_context::<HoverCardContext>();
            move || {
                let Some(c) = card else {
                    return Element::Empty;
                };
                if !c.intent.is_open() {
                    return Element::Empty;
                }
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let content = root
                    .on_hover(move |inside| {
                        // Over the card it stays open; leaving it starts the close delay.
                        if inside {
                            c.intent.cancel();
                        } else {
                            c.hovered(false);
                        }
                    })
                    .on_key_down(move |e| {
                        let escape = e.is(NamedKey::Escape);
                        if escape {
                            c.intent.set_now(false);
                        }
                        escape
                    })
                    .on_click_outside(move || c.intent.set_now(false));
                deferred(
                    Placement::Anchored {
                        side: p.side,
                        align: p.align,
                        offset: p.offset,
                        match_width: false,
                        fit_height: false,
                    },
                    content,
                )
                .into_element()
            }
        })
    })
}
