//! Scroll Area (`ui/scroll-area`): a box that scrolls its content behind a
//! thin scrollbar of its own.
//!
//! The behaviour is `primitives::scroll_area` (reka's `ScrollArea`): only
//! the axes with a bar scroll; the viewport is a Tab stop and scrolls by
//! Chrome's keys; a bar shows while the pointer is over the area and the
//! content overflows, and hides 600 ms after it leaves (reka's `hover`,
//! shadcn's default, unless [`policy`](Part::policy) says otherwise); the
//! thumb is dragged, the track pressed and the wheel turned over a bar. This
//! file draws the root, the viewport, the bars and their thumbs.

use std::time::Duration;

use gui::prelude::*;
use gui::{Axis, Color, Properties, component, div};
use primitives::direction::use_direction;
use primitives::scroll_area::{Bars, HIDE_DELAY, Policy, ScrollArea, ScrollAreaOptions};

use crate::focus::StateLook;
use crate::part::Part;
use crate::theme::theme;

/// What a scroll area is given: which axes have a bar, when they show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollAreaProps {
    bars: Bars,
    policy: Policy,
    hide_delay: Duration,
}

impl ScrollAreaProps {
    const fn options(self) -> ScrollAreaOptions {
        ScrollAreaOptions {
            policy: self.policy,
            hide_delay: self.hide_delay,
            bars: self.bars,
        }
    }
}

fn area(bars: Bars) -> Part<ScrollAreaProps> {
    let root = div().named("scroll-area").position(Position::Relative);
    let props = ScrollAreaProps {
        bars,
        policy: Policy::Hover,
        hide_delay: HIDE_DELAY,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let area = ScrollArea::new(move || props.with(|p| p.props.options()), use_direction(None));
            let t = theme();
            // `size-full rounded-[inherit] transition-[color,box-shadow]
            // focus-visible:ring-[3px] ring-ring/50`.
            let viewport = area.viewport(
                div()
                    .named("scroll-area-viewport")
                    .size(pct(100.0))
                    .transition(Properties::COLOR | Properties::SHADOW)
                    .with_interaction(|d, i| d.focus_ring_on(i, &t)),
            );
            move || {
                let mut root = area.root(props.with(|p| p.slot.0.clone()));
                let shape = root.style_mut().shape;
                let mut root = root.into_element();
                let content = root.children_mut().map(std::mem::take).unwrap_or_default();
                let bars = props.with(|p| p.props.bars);
                let viewport = viewport.clone().shape(shape).children(content);
                if let Some(kids) = root.children_mut() {
                    kids.push(viewport.into_element());
                    if bars.vertical() {
                        kids.push(scroll_bar(area, Axis::Vertical).into_element());
                    }
                    if bars.horizontal() {
                        kids.push(scroll_bar(area, Axis::Horizontal).into_element());
                    }
                    kids.push(scroll_corner(area).into_element());
                }
                root
            }
        })
    })
}

/// One bar and its thumb: its own instance, so a bar showing or hiding
/// renders it alone.
fn scroll_bar(area: ScrollArea, axis: Axis) -> gui::ComponentElement {
    component((area, axis), |cx| {
        let (area, axis) = cx.props().with_untracked(|p| *p);
        let t = theme();
        // `flex touch-none p-px transition-colors select-none`, then
        // `h-full w-2.5 border-l border-l-transparent` down, `h-2.5 flex-col
        // border-t border-t-transparent` across.
        let bar = div()
            .named("scroll-area-scrollbar")
            .display(Display::Flex)
            .padding(1.0)
            .transition(Properties::COLOR)
            .border_color(Color::TRANSPARENT);
        let bar = match axis {
            Axis::Vertical => bar.width(10.0).border_left(1.0),
            Axis::Horizontal => bar.height(10.0).flex_direction(FlexDirection::Column).border_top(1.0),
        };
        // `relative flex-1 bg-border rounded-full`.
        let thumb = div()
            .named("scroll-area-thumb")
            .position(Position::Relative)
            .flex_grow(1.0)
            .flex_basis(Length::Px(0.0))
            .background(t.border)
            .radius(9999.0);
        move || {
            let thumb = area.thumb(thumb.clone(), axis);
            area.scrollbar(bar.clone().child(thumb), axis)
        }
    })
}

/// The square where the two bars meet (shadcn gives it no look).
fn scroll_corner(area: ScrollArea) -> gui::ComponentElement {
    component(area, |cx| {
        let area = cx.props().with_untracked(|p| *p);
        move || area.corner(div().named("scroll-area-corner")).into_element()
    })
}

/// A vertically scrolling area; give it a height and put the content in with `.child`.
#[must_use]
pub fn scroll_area() -> Part<ScrollAreaProps> {
    area(Bars::Vertical)
}

/// An area with a horizontal scrollbar too (`<ScrollBar orientation="horizontal" />`).
#[must_use]
pub fn scroll_area_horizontal() -> Part<ScrollAreaProps> {
    area(Bars::Both)
}

impl Part<ScrollAreaProps> {
    /// When the bars show (reka's `type`; `hover` unless set).
    #[must_use]
    pub fn policy(self, policy: Policy) -> Self {
        self.with_props(|p| p.policy = policy)
    }

    /// How long a bar waits before it hides (reka's `scroll-hide-delay`,
    /// 600 ms unless set).
    #[must_use]
    pub fn hide_delay(self, delay: Duration) -> Self {
        self.with_props(|p| p.hide_delay = delay)
    }
}
