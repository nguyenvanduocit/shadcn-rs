//! Trace (`ui/trace`): what an agent did, one line per step (ran a command,
//! thought, used sources), each with an icon, a title and a status, and steps
//! that open to show their output.
//!
//! A line takes its muted colour from [`trace`] or the
//! [`trace_collapsible_trigger`] it sits in, so hovering the trigger
//! brightens the line, as Vue's `in-[button:hover]:text-foreground` does.

use gui::prelude::*;
use gui::{Animation, Div, Element, Icon, Text, component, div, icon, text};

use crate::collapsible::{CollapsibleContext, CollapsibleProps, collapsible};
use crate::focus::StateLook;
use crate::part::Part;
use crate::theme::theme;

/// A step's state; `None` on [`trace_line`] is a finished step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceStatus {
    /// In progress. Vue runs a shimmer across the title; `gui` text has one
    /// colour per run, so the title shows plain.
    Running,
    /// Failed: the line turns red.
    Error,
    /// Not started: the line is faded.
    Pending,
}

/// A column of steps.
#[must_use]
pub fn trace() -> Div {
    div()
        .named("trace")
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
        .gap(8.0)
        .text_color(theme().muted_foreground)
}

/// One step: a [`trace_icon`], a [`trace_title`] and maybe a [`trace_description`].
#[must_use]
pub fn trace_line(status: Option<TraceStatus>) -> Div {
    let t = theme();
    div()
        .named("trace-line")
        .display(Display::Flex)
        .align_self(Align::Start)
        .max_width(pct(100.0))
        .min_width(0.0)
        .align_items(Align::Center)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .transition(Properties::COLORS)
        .when(status == Some(TraceStatus::Error), |d| d.text_color(t.destructive))
        .when(status == Some(TraceStatus::Pending), |d| d.opacity(0.6))
}

/// A step that is a button or a link (Vue's `<button>` or `<a>` holding a
/// [`trace_line`] in a [`trace`]): it fits its line, takes focus, and
/// brightens the line while hovered, unless the line failed (`failed`).
/// Give it `.on_click`.
#[must_use]
pub fn trace_action(failed: bool, line: impl IntoElement) -> Div {
    let t = theme();
    div()
        .named("trace-action")
        .display(Display::Flex)
        .align_self(Align::Start)
        .max_width(pct(100.0))
        .min_width(0.0)
        .radius(8.0)
        .text_align(TextAlign::Left)
        .focusable()
        .cursor(Cursor::Pointer)
        .with_interaction(|d, i| {
            d.when(!failed, |d| d.text_color_while(move || i.hovered(), t.foreground))
                .focus_ring_on(i, &t)
        })
        .child(line)
}

/// The 16px slot an icon or spinner sits in.
#[must_use]
pub fn trace_icon(child: impl IntoElement) -> Div {
    div()
        .named("trace-icon")
        .display(Display::Flex)
        .size(16.0)
        .flex_shrink(0.0)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .child(child)
}

/// A [`trace_icon`] after the title (a chevron, an external-link arrow):
/// Vue's `[[data-slot=trace-title]~&]:-ml-1` pulls it 4px towards the title.
#[must_use]
pub fn trace_end_icon(child: impl IntoElement) -> Div {
    trace_icon(child).margin_left(-4.0)
}

/// What the step did, cut to one line.
#[must_use]
pub fn trace_title(title: impl Into<String>) -> Text {
    text(title)
        .named("trace-title")
        .truncate(true)
        .nowrap(true)
        .overflow_x(Overflow::Hidden)
        .min_width(0.0)
}

/// A detail after the title (a duration, a count), fainter.
#[must_use]
pub fn trace_description(description: impl Into<String>) -> Text {
    text(description)
        .named("trace-description")
        .flex(Flex::NONE)
        .tabular_nums(true)
        .text_color(theme().muted_foreground.opacity(0.7))
}

/// A step that opens; holds a [`trace_collapsible_trigger`] and its
/// [`trace_collapsible_content`]. Closed at first; `.default_open(true)` or
/// `.model(…)` as any [`collapsible`].
#[must_use]
pub fn trace_collapsible() -> Part<CollapsibleProps> {
    collapsible()
        .map_root(|d| d.named("trace-collapsible"))
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(pct(100.0))
        .min_width(0.0)
}

/// The step's `line` and a chevron that turns down while open; pressing it
/// opens and closes the step. Red when the line's step failed (`failed`).
#[must_use]
#[track_caller]
pub fn trace_collapsible_trigger(failed: bool, line: impl IntoElement) -> Part<()> {
    let t = theme();
    let control = div()
        .named("trace-collapsible-trigger")
        .focusable()
        .display(Display::Flex)
        .align_self(Align::Start)
        .max_width(pct(100.0))
        .min_width(0.0)
        .align_items(Align::Center)
        .gap(4.0)
        .radius(8.0)
        .cursor(Cursor::Pointer)
        .text_align(TextAlign::Left)
        .transition(Properties::COLORS)
        .text_color(if failed { t.destructive } else { t.muted_foreground })
        .with_interaction(|d, i| {
            d.when(!failed, |d| d.text_color_while(move || i.hovered(), t.foreground))
                .focus_ring_on(i, &t)
        })
        .child(line);
    Part::new((), control, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CollapsibleContext::find(props, "trace_collapsible_trigger");
            move || {
                let control = props.with(|p| p.slot.0.clone());
                let Some(ctx) = ctx else {
                    return control.into_element();
                };
                let open = ctx.is_open();
                ctx.0
                    .trigger(control)
                    .child(
                        trace_icon(icon(Icon::ChevronRight))
                            .transition(Properties::TRANSFORM)
                            .rotate(if open { 90.0 } else { 0.0 }),
                    )
                    .into_element()
            }
        })
    })
}

/// What the step produced, indented under a rule, shown while open: what
/// goes in fills the ruled box; the box around it rolls down from no height
/// as it opens and back up as it closes, 200ms (`animate-collapsible-*`).
#[must_use]
#[track_caller]
pub fn trace_collapsible_content() -> Part<()> {
    let t = theme();
    let ruled = div()
        .min_width(0.0)
        .margin_top(8.0)
        .margin_bottom(4.0)
        // `ml-[calc(--spacing(2)-1px)]`: the rule under the icon's centre.
        .margin_left(7.0)
        .border_left(1.0)
        .border_color(t.border)
        .padding_right(4.0)
        .padding_left(16.0)
        .line_height(LineHeight::Relative(1.625))
        .text_color(t.muted_foreground);
    Part::new((), ruled, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = CollapsibleContext::find(props, "trace_collapsible_content");
            move || {
                // Outside a collapsible (reported): its box, shown, as
                // `collapsible_content` does: the look without the behaviour.
                if !ctx.as_ref().is_none_or(CollapsibleContext::is_open) {
                    return Element::Empty;
                }
                div()
                    .named("trace-collapsible-content")
                    .min_width(0.0)
                    .overflow(Overflow::Hidden)
                    .animation(Animation::accordion_down().after_mount())
                    .exit_animation(Animation::accordion_up())
                    .duration_ms(200.0)
                    .child(props.with(|p| p.slot.0.clone()))
                    .into_element()
            }
        })
    })
}
