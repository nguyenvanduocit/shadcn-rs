//! Hover card (`ui/hover-card`): a card that opens while the pointer rests
//! on its trigger (after 700ms) and closes 300ms after it leaves both the
//! trigger and the card.
//!
//! The parts nest as in Vue: [`hover_card`] is the anchor and keeps whether
//! the card is open, its own (`default_open`) or the caller's (`open`,
//! `v-model:open`), and its delays (`open_delay`, `close_delay`); it holds
//! the [`hover_card_trigger`] and the [`hover_card_content`]. The behaviour
//! is `primitives::hover_card`'s (the delays, the grace area the pointer
//! crosses from the trigger to the card, Escape, a press outside and a
//! scroll closing it); this module draws it.
//!
//! ```ignore
//! hover_card()
//!     .child(hover_card_trigger(button(ButtonVariant::Link, ButtonSize::Default).child("@nuxt")))
//!     .child(hover_card_content().width(320.0).child("…"))
//! ```

use std::time::Duration;

use gui::prelude::*;
use gui::{Bind, Div, Element, Model, Side, SideAlign, component, div, provide, use_model};
use primitives::context::use_root;
use primitives::hover_card::{CLOSE_DELAY, HoverCard, HoverCardOptions, OPEN_DELAY};
use primitives::popper::Popper;
use primitives::presence::Presence;

use gui::style::Origin;

use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`hover_card`] is given.
#[derive(Clone, PartialEq)]
pub struct HoverCardProps {
    open: Bind<bool>,
    open_delay: Duration,
    close_delay: Duration,
}

/// The hover card of a [`hover_card`] root.
type HoverCardState = HoverCard<PartProps<HoverCardProps>>;

/// The anchor the card is placed against: wrap the trigger (and the
/// content) in it. Closed, and kept by itself, until told otherwise.
#[must_use]
pub fn hover_card() -> Part<HoverCardProps> {
    let props = HoverCardProps {
        open: Bind::default(),
        open_delay: OPEN_DELAY,
        close_delay: CLOSE_DELAY,
    };
    Part::new(
        props,
        div().named("hover-card").display(Display::Flex).inline(true),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let card = HoverCard::new(
                    use_model(props, |p: &PartProps<HoverCardProps>| &p.props.open),
                    move || {
                        props.with(|p| HoverCardOptions {
                            open_delay: p.props.open_delay,
                            close_delay: p.props.close_delay,
                            ..HoverCardOptions::default()
                        })
                    },
                );
                provide(card);
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
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
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
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
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn hover_card_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let card = use_root::<HoverCardState>("hover_card_trigger", "hover_card");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match card {
                    Some(c) => c.trigger(root),
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
/// (`HoverCardContent`); style and fill it as a `Div`.
#[must_use]
pub fn hover_card_content() -> Part<HoverCardContentProps> {
    let t = theme();
    // `origin-(--reka-hover-card-content-transform-origin)`: the side it
    // was placed on, flipped or not.
    let root = popper(div(), Origin::Anchor)
        .named("hover-card-content")
        .background(t.popover)
        .text_color(t.popover_foreground)
        .width(256.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding(16.0)
        .shadow(crate::popover::POPUP_SHADOW);
    let props = HoverCardContentProps {
        side: Side::Bottom,
        align: SideAlign::Center,
        offset: 4.0,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let card = use_root::<HoverCardState>("hover_card_content", "hover_card");
            move || {
                let Some(c) = card else {
                    return Element::Empty;
                };
                if c.presence() != Presence::Open {
                    return Element::Empty;
                }
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let at = Popper::DEFAULT.side(p.side).align(p.align).side_offset(p.offset);
                c.floating(root, &at).into_element()
            }
        })
    })
}
