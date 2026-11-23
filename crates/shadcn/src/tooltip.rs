//! Tooltip (`ui/tooltip`): a short label that shows while the pointer rests
//! on its trigger, or while the trigger holds focus, with an arrow pointing
//! at it.
//!
//! The parts nest as in Vue: [`tooltip`] is the anchor and keeps whether the
//! tooltip is open, its own (`default_open`) or the caller's (`open`,
//! `v-model:open`); it holds the [`tooltip_trigger`] and the
//! [`tooltip_content`]. How long the pointer rests before it opens comes
//! from the nearest [`tooltip_provider`] (shadcn's `TooltipProvider`, 0 ms),
//! else from the window's, which has shadcn's 0 ms too; a tooltip's own
//! `delay_duration` wins over both. The behaviour is
//! `primitives::tooltip`'s (one open in the window, the skip-delay window,
//! the grace area onto the content, keyboard focus, Escape); this module
//! draws it.
//!
//! ```ignore
//! tooltip()
//!     .child(tooltip_trigger(button(ButtonVariant::Outline, ButtonSize::Default).child("Hover")))
//!     .child(tooltip_content().child(text("Add to library")))
//! ```

use std::f32::consts::FRAC_1_SQRT_2;
use std::time::Duration;

use gui::prelude::*;
use gui::{
    Bind, Color, Div, Element, Model, Path, PathBuilder, Side, SideAlign, component, div, figures, provide, px,
    use_model,
};
use primitives::context::use_root;
use primitives::popper::Popper;
use primitives::presence::Presence;
use primitives::tooltip::{Tooltip, TooltipOptions, TooltipOverrides, TooltipProvider};

use gui::style::Origin;

use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// shadcn's `TooltipProvider` defaults: reka's, with no delay.
const SHADCN: TooltipOptions = TooltipOptions {
    delay: Duration::ZERO,
    skip_delay: primitives::tooltip::SKIP_DELAY,
    hoverable: true,
    closing_trigger: true,
    keyboard_focus_only: false,
    disabled: false,
};

/// What [`tooltip_provider`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipProviderProps {
    options: TooltipOptions,
}

/// The tooltips inside share its delays (`TooltipProvider`; shadcn's
/// opens at once): a plain `div` its caller lays out. One tooltip is open
/// in the window whatever its provider.
#[must_use]
pub fn tooltip_provider() -> Part<TooltipProviderProps> {
    Part::new(
        TooltipProviderProps { options: SHADCN },
        div().named("tooltip-provider"),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                provide(TooltipProvider::new(move || props.with(|p| p.props.options)));
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

impl Part<TooltipProviderProps> {
    /// How long the pointer rests before a tooltip opens (`:delay-duration`).
    #[must_use]
    pub fn delay_duration(self, delay: Duration) -> Self {
        self.with_props(|p| p.options.delay = delay)
    }

    /// How long after one closes another opens at once
    /// (`:skip-delay-duration`, 300ms).
    #[must_use]
    pub fn skip_delay_duration(self, delay: Duration) -> Self {
        self.with_props(|p| p.options.skip_delay = delay)
    }
}

/// What [`tooltip`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct TooltipProps {
    open: Bind<bool>,
    delay: Option<Duration>,
}

/// The tooltip of a [`tooltip`] root.
type TooltipState = Tooltip<PartProps<TooltipProps>>;

/// The anchor the tooltip is placed against: wrap the trigger (and the
/// content) in it. Closed, and kept by itself, until told otherwise.
#[must_use]
pub fn tooltip() -> Part<TooltipProps> {
    Part::new(
        TooltipProps::default(),
        div().named("tooltip").display(Display::Flex).inline(true),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let provider = TooltipProvider::nearest_or_window(SHADCN);
                let tip = Tooltip::new(
                    use_model(props, |p: &PartProps<TooltipProps>| &p.props.open),
                    provider,
                    move || {
                        props.with(|p| TooltipOverrides {
                            delay: p.props.delay,
                            ..TooltipOverrides::default()
                        })
                    },
                );
                provide(tip);
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

impl Part<TooltipProps> {
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

    /// How long the pointer rests on the trigger before it opens
    /// (`:delay-duration`; its provider's by default).
    #[must_use]
    pub fn delay_duration(self, delay: Duration) -> Self {
        self.with_props(|p| p.delay = Some(delay))
    }
}

/// Make `trigger` show the tooltip while the pointer rests on it, or while
/// it holds focus; pressing it hides the tooltip (`TooltipTrigger as-child`).
/// Give `trigger` its children first: a [`Button`](crate::Button) decides
/// its padding from them as it becomes the box.
#[must_use]
pub fn tooltip_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let tip = use_root::<TooltipState>("tooltip_trigger", "tooltip");
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match tip {
                    Some(c) => c.trigger(root),
                    None => root,
                }
                .into_element()
            }
        })
    })
}

/// What [`tooltip_content`] is given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipContentProps {
    side: Side,
    align: SideAlign,
}

impl Part<TooltipContentProps> {
    /// On `side` of its anchor (`side="…"`; above by default).
    #[must_use]
    pub fn side(self, side: Side) -> Self {
        self.with_props(|p| p.side = side)
    }

    /// Lined up with its anchor by `align` (`align="…"`; centred by default).
    #[must_use]
    pub fn align(self, align: SideAlign) -> Self {
        self.with_props(|p| p.align = align)
    }
}

/// The tooltip above its anchor, centred on it, 4px away, with its arrow
/// (`TooltipContent`); style and fill it as a `Div`.
#[must_use]
pub fn tooltip_content() -> Part<TooltipContentProps> {
    let t = theme();
    // `origin-(--reka-tooltip-content-transform-origin) animate-in fade-in-0
    // zoom-in-95`, closing as a popper's content does.
    let root = popper(div(), Origin::Anchor)
        .named("tooltip-content")
        .background(t.foreground)
        .text_color(t.background)
        .radius(8.0)
        .padding_x(12.0)
        .padding_y(6.0)
        .text_size(12.0)
        .line_height(16.0)
        .text_wrap(WrapStyle::Balance);
    let props = TooltipContentProps {
        side: Side::Top,
        align: SideAlign::Center,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tip = use_root::<TooltipState>("tooltip_content", "tooltip");
            move || {
                let Some(c) = tip else {
                    return Element::Empty;
                };
                if c.presence() != Presence::Open {
                    return Element::Empty;
                }
                let t = theme();
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let at = Popper::DEFAULT.side(p.side).align(p.align).side_offset(4.0);
                // The arrow points from the side it was placed on, flipped
                // or not (reka's `data-side`); the one asked for until
                // layout first places it.
                let side = c.placement().map_or(p.side, |placed| placed.side);
                c.floating(root.child(tooltip_arrow(side, t.foreground)), &at)
                    .into_element()
            }
        })
    })
}

/// `TooltipArrow` as shadcn styles it (`size-2.5 rotate-45 rounded-xs
/// translate-y-[calc(-50%_-_2px)]`): a 10px square turned to a diamond whose
/// centre sits 2px inside the edge facing the trigger, so half a diamond
/// points out of the tooltip.
fn tooltip_arrow(side: Side, color: Color) -> gui::Figures {
    const SQUARE: f32 = 10.0;
    const RADIUS: f32 = 2.0;
    const INSET: f32 = 2.0;
    let half_diagonal = SQUARE * FRAC_1_SQRT_2;
    let box_size = half_diagonal * 2.0;
    let out = px(INSET - half_diagonal);
    // Back by half its box, to centre it on the edge.
    let shift = half_diagonal;
    let arrow = figures(vec![diamond(SQUARE / 2.0, RADIUS, half_diagonal).fill(color)])
        .named("tooltip-arrow")
        .pass_through()
        .position(Position::Absolute)
        .size(box_size);
    match side {
        Side::Top => arrow.bottom(out).left(pct(50.0)).margin_left(-shift),
        Side::Bottom => arrow.top(out).left(pct(50.0)).margin_left(-shift),
        Side::Left => arrow.right(out).top(pct(50.0)).margin_top(-shift),
        Side::Right => arrow.left(out).top(pct(50.0)).margin_top(-shift),
    }
}

/// A square of half-side `h` with corners rounded by `r`, turned 45° and
/// centred at `(c, c)`.
fn diamond(h: f32, r: f32, c: f32) -> PathBuilder {
    let turn = |x: f32, y: f32| ((x - y).mul_add(FRAC_1_SQRT_2, c), (x + y).mul_add(FRAC_1_SQRT_2, c));
    // Each corner: where its rounding starts, the corner itself, where it ends.
    let corners = [
        ((h - r, -h), (h, -h), (h, r - h)),
        ((h, h - r), (h, h), (h - r, h)),
        ((r - h, h), (-h, h), (-h, h - r)),
        ((-h, r - h), (-h, -h), (r - h, -h)),
    ];
    let (sx, sy) = turn(r - h, -h);
    let mut shape = Path::builder().move_to(sx, sy);
    for (start, corner, end) in corners {
        let (ax, ay) = turn(start.0, start.1);
        let (cx, cy) = turn(corner.0, corner.1);
        let (ex, ey) = turn(end.0, end.1);
        shape = shape.line_to(ax, ay).quad_to(cx, cy, ex, ey);
    }
    shape.close()
}
