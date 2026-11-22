//! Tooltip (`ui/tooltip`): a short label that shows while the pointer rests
//! on its trigger, or while the trigger holds focus reached by keyboard,
//! with an arrow pointing at it.
//!
//! The parts nest as in Vue: [`tooltip`] is the anchor and keeps whether the
//! tooltip is open, its own (`default_open`) or the caller's (`open`,
//! `v-model:open`), and how long the pointer rests before it opens
//! (`delay_duration`; shadcn's `TooltipProvider` makes it 0); it holds the
//! [`tooltip_trigger`] and the [`tooltip_content`].
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
    Bind, Color, Div, Element, FocusHandle, Model, NamedKey, Path, PathBuilder, Placement, Side, SideAlign, Signal,
    component, deferred, div, provide, px, relative, shapes, use_context, use_model, watch,
};

use gui::style::Origin;

use crate::hover_card::HoverIntent;
use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`tooltip`] is given.
#[derive(Clone, PartialEq, Default)]
pub struct TooltipProps {
    open: Bind<bool>,
    delay: Duration,
}

/// What the parts below a [`tooltip`] read.
#[derive(Clone, Copy)]
struct TooltipContext {
    intent: HoverIntent<PartProps<TooltipProps>>,
    props: Signal<PartProps<TooltipProps>>,
}

/// The anchor the tooltip is placed against: wrap the trigger (and the
/// content) in it. Closed, kept by itself, and opening as soon as the pointer
/// arrives, until told otherwise.
#[must_use]
pub fn tooltip() -> Part<TooltipProps> {
    Part::new(TooltipProps::default(), div().named("tooltip").inline_flex(), |p| {
        component(p, |cx| {
            let props = cx.props();
            provide(TooltipContext {
                intent: HoverIntent::new(use_model(props, |p| &p.props.open)),
                props,
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
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
    pub fn on_open_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// How long the pointer rests on the trigger before it opens
    /// (`:delay-duration`; 0, as shadcn's `TooltipProvider` sets it).
    #[must_use]
    pub fn delay_duration(self, delay: Duration) -> Self {
        self.with_props(|p| p.delay = delay)
    }
}

/// Make `trigger` show the tooltip while the pointer rests on it, or while
/// it holds focus reached by keyboard; pressing it hides the tooltip
/// (`TooltipTrigger as-child`).
#[must_use]
pub fn tooltip_trigger(trigger: Div) -> Part<()> {
    Part::new((), trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tip = use_context::<TooltipContext>();
            let handle = FocusHandle::new();
            if let Some(c) = tip {
                // Reka opens on a focus the keyboard made, and closes on blur.
                watch(
                    move || handle.is_focused() && gui::focus::focus_visible(),
                    move |focused, _| c.intent.set_now(*focused),
                );
            }
            move || {
                let root = props.with(|p| p.slot.0.clone());
                match tip {
                    Some(c) => root
                        .track_focus(handle)
                        .on_hover(move |inside| {
                            if inside {
                                let delay = c.props.with_untracked(|p| p.props.delay);
                                c.intent.set_after(true, delay);
                            } else {
                                c.intent.set_now(false);
                            }
                        })
                        .on_mouse_down(move |_| c.intent.set_now(false)),
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
/// (`TooltipContent`); style and fill it as a `Div`. Escape closes it.
#[must_use]
pub fn tooltip_content() -> Part<TooltipContentProps> {
    let t = theme();
    // `animate-in fade-in-0 zoom-in-95`, closing as a popper's content does.
    let root = popper(div(), Origin::CENTER)
        .named("tooltip-content")
        .bg(t.foreground)
        .text_color(t.background)
        .rounded_md()
        .px(3.0)
        .py(1.5)
        .text_xs()
        .text_balance();
    let props = TooltipContentProps {
        side: Side::Top,
        align: SideAlign::Center,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let tip = use_context::<TooltipContext>();
            move || {
                let Some(c) = tip else {
                    return Element::Empty;
                };
                if !c.intent.is_open() {
                    return Element::Empty;
                }
                let t = theme();
                let (p, root) = props.with(|p| (p.props, p.slot.0.clone()));
                let content = root
                    .on_key_down(move |e| {
                        let escape = e.is(NamedKey::Escape);
                        if escape {
                            c.intent.set_now(false);
                        }
                        escape
                    })
                    .child(tooltip_arrow(p.side, t.foreground));
                deferred(
                    Placement::Anchored {
                        side: p.side,
                        align: p.align,
                        offset: 4.0,
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

/// `TooltipArrow` as shadcn styles it (`size-2.5 rotate-45 rounded-xs
/// translate-y-[calc(-50%_-_2px)]`): a 10px square turned to a diamond whose
/// centre sits 2px inside the edge facing the trigger, so half a diamond
/// points out of the tooltip.
fn tooltip_arrow(side: Side, color: Color) -> gui::Shapes {
    const SQUARE: f32 = 10.0;
    const RADIUS: f32 = 2.0;
    const INSET: f32 = 2.0;
    let half_diagonal = SQUARE * FRAC_1_SQRT_2;
    let box_size = half_diagonal * 2.0;
    let out = px(INSET - half_diagonal);
    // Back by half its box, on Tailwind's scale, to centre it on the edge.
    let shift = half_diagonal / gui::styled::UNIT;
    let arrow = shapes(vec![diamond(SQUARE / 2.0, RADIUS, half_diagonal).fill(color)])
        .named("tooltip-arrow")
        .pass_through()
        .absolute()
        .size_px(box_size);
    match side {
        Side::Top => arrow.bottom_len(out).left_len(relative(0.5)).ml(-shift),
        Side::Bottom => arrow.top_len(out).left_len(relative(0.5)).ml(-shift),
        Side::Left => arrow.right_len(out).top_len(relative(0.5)).mt(-shift),
        Side::Right => arrow.left_len(out).top_len(relative(0.5)).mt(-shift),
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
