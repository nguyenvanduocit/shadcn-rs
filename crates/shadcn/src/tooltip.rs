//! Tooltip (`ui/tooltip`): a short label that shows while the pointer rests
//! on its trigger, with an arrow pointing at it.
//!
//! The parts nest as in Vue: [`tooltip`] is the anchor; it holds the
//! [`tooltip_trigger`] and the [`tooltip_content`].

use std::f32::consts::FRAC_1_SQRT_2;
use std::time::Duration;

use gui::prelude::*;
use gui::{Color, Div, Element, NamedKey, Placement, Shape, Side, SideAlign, deferred, div, px, relative, shapes};

use gui::style::Origin;

use crate::hover_card::HoverIntent;
use crate::motion::popper;
use crate::theme::theme;

/// Whether a tooltip is open, and how long the pointer rests before it opens.
#[derive(Clone)]
pub struct TooltipState {
    intent: HoverIntent,
    delay: Duration,
}

impl Default for TooltipState {
    fn default() -> Self {
        Self::new()
    }
}

impl TooltipState {
    /// A closed tooltip that opens as soon as the pointer arrives
    /// (shadcn's `TooltipProvider` sets `delayDuration` to 0).
    #[must_use]
    pub fn new() -> Self {
        Self::with_delay(Duration::ZERO)
    }

    /// A closed tooltip that opens once the pointer has rested `delay` on its
    /// trigger (`<Tooltip :delay-duration="…">`).
    #[must_use]
    pub fn with_delay(delay: Duration) -> Self {
        Self {
            intent: HoverIntent::new(),
            delay,
        }
    }

    /// Whether the tooltip shows (reactive).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.intent.is_open()
    }

    pub fn close(&self) {
        self.intent.set_now(false);
    }
}

/// The anchor the tooltip is placed against: wrap the trigger (and the content) in it.
#[must_use]
pub fn tooltip() -> Div {
    div().named("tooltip").inline_flex()
}

/// Make `trigger` show the tooltip while the pointer rests on it; pressing it
/// hides the tooltip (`TooltipTrigger as-child`).
#[must_use]
pub fn tooltip_trigger(state: &TooltipState, trigger: Div) -> Div {
    let on_hover = state.clone();
    let on_press = state.clone();
    trigger
        .on_hover(move |inside| {
            if inside {
                on_hover.intent.set_after(true, on_hover.delay);
            } else {
                on_hover.close();
            }
        })
        .on_mouse_down(move |_| on_press.close())
}

/// The tooltip above its anchor, centred on it, 4px away; `build` adds the
/// classes and children a caller gives `TooltipContent`.
#[must_use]
pub fn tooltip_content(state: &TooltipState, build: impl FnOnce(Div) -> Div) -> Element {
    tooltip_content_placed(state, Side::Top, SideAlign::Center, build)
}

/// [`tooltip_content`] on `side` of its anchor, lined up by `align`
/// (`<TooltipContent side="…" align="…">`).
#[must_use]
pub fn tooltip_content_placed(
    state: &TooltipState,
    side: Side,
    align: SideAlign,
    build: impl FnOnce(Div) -> Div,
) -> Element {
    if !state.is_open() {
        return Element::Empty;
    }
    let t = theme();
    let on_key = state.clone();
    // `animate-in fade-in-0 zoom-in-95`, closing as a popper's content does.
    let content = popper(div(), Origin::CENTER)
        .named("tooltip-content")
        .bg(t.foreground)
        .text_color(t.background)
        .rounded_md()
        .px(3.0)
        .py(1.5)
        .text_xs()
        .text_balance()
        .on_key_down(move |e| {
            let escape = e.is(NamedKey::Escape);
            if escape {
                on_key.close();
            }
            escape
        });
    deferred(
        Placement::Anchored {
            side,
            align,
            offset: 4.0,
            match_width: false,
            fit_height: false,
        },
        build(content).child(tooltip_arrow(side, t.foreground)),
    )
    .into_element()
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
fn diamond(h: f32, r: f32, c: f32) -> Shape {
    let turn = |x: f32, y: f32| ((x - y).mul_add(FRAC_1_SQRT_2, c), (x + y).mul_add(FRAC_1_SQRT_2, c));
    // Each corner: where its rounding starts, the corner itself, where it ends.
    let corners = [
        ((h - r, -h), (h, -h), (h, r - h)),
        ((h, h - r), (h, h), (h - r, h)),
        ((r - h, h), (-h, h), (-h, h - r)),
        ((-h, r - h), (-h, -h), (r - h, -h)),
    ];
    let (sx, sy) = turn(r - h, -h);
    let mut shape = Shape::path().move_to(sx, sy);
    for (start, corner, end) in corners {
        let (ax, ay) = turn(start.0, start.1);
        let (cx, cy) = turn(corner.0, corner.1);
        let (ex, ey) = turn(end.0, end.1);
        shape = shape.line_to(ax, ay).quad_to(cx, cy, ex, ey);
    }
    shape.close()
}
