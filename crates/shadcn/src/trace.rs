//! Trace (`ui/trace`): what an agent did, one line per step (ran a command,
//! thought, used sources), each with an icon, a title and a status, and steps
//! that open to show their output.
//!
//! A line takes its muted colour from [`trace`] or the
//! [`trace_collapsible_trigger`] it sits in, so hovering the trigger
//! brightens the line, as Vue's `in-[button:hover]:text-foreground` does.

use gui::prelude::*;
use gui::{Animation, Div, Icon, Signal, Text, div, icon, text};

use crate::collapsible::{collapsible, collapsible_content, collapsible_trigger};
use crate::focus::focus_ring;
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
        .flex_col()
        .w_full()
        .min_w(0.0)
        .gap(2.0)
        .text_color(theme().muted_foreground)
}

/// One step: a [`trace_icon`], a [`trace_title`] and maybe a [`trace_description`].
#[must_use]
pub fn trace_line(status: Option<TraceStatus>) -> Div {
    let t = theme();
    div()
        .named("trace-line")
        .flex()
        .self_start()
        .max_w_full()
        .min_w(0.0)
        .items_center()
        .gap(2.0)
        .text_sm()
        .transition_colors()
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
        .flex()
        .self_start()
        .max_w_full()
        .min_w(0.0)
        .rounded_md()
        .text_left()
        .focusable()
        .cursor_pointer()
        .when(!failed, |d| d.hover(move |s| s.text_color(t.foreground)))
        .focus_visible(move |s| focus_ring(s, &t))
        .child(line)
}

/// The 16px slot an icon or spinner sits in.
#[must_use]
pub fn trace_icon(child: impl IntoElement) -> Div {
    div()
        .named("trace-icon")
        .flex()
        .size(4.0)
        .shrink_0()
        .items_center()
        .justify_center()
        .child(child)
}

/// A [`trace_icon`] after the title (a chevron, an external-link arrow):
/// Vue's `[[data-slot=trace-title]~&]:-ml-1` pulls it 4px towards the title.
#[must_use]
pub fn trace_end_icon(child: impl IntoElement) -> Div {
    trace_icon(child).ml(-1.0)
}

/// What the step did, cut to one line.
#[must_use]
pub fn trace_title(title: impl Into<String>) -> Text {
    text(title).named("trace-title").truncate()
}

/// A detail after the title (a duration, a count), fainter.
#[must_use]
pub fn trace_description(description: impl Into<String>) -> Text {
    text(description)
        .named("trace-description")
        .flex_none()
        .tabular_nums()
        .text_color(theme().muted_foreground.opacity(0.7))
}

/// A step that opens; holds a [`trace_collapsible_trigger`] and its
/// [`trace_collapsible_content`].
#[must_use]
pub fn trace_collapsible() -> Div {
    collapsible().named("trace-collapsible").flex_col().w_full().min_w(0.0)
}

/// The step's `line` and a chevron that turns down while `open`; pressing
/// it flips `open`. Red when the line's step failed (`failed`).
#[must_use]
pub fn trace_collapsible_trigger(open: &Signal<bool>, failed: bool, line: impl IntoElement) -> Div {
    let t = theme();
    let turned = if open.get() { 90.0 } else { 0.0 };
    collapsible_trigger(open)
        .named("trace-collapsible-trigger")
        .flex()
        .self_start()
        .max_w_full()
        .min_w(0.0)
        .items_center()
        .gap(1.0)
        .rounded_md()
        .cursor_pointer()
        .text_left()
        .transition_colors()
        .text_color(if failed { t.destructive } else { t.muted_foreground })
        .when(!failed, |d| d.hover(move |s| s.text_color(t.foreground)))
        .focus_visible(move |s| focus_ring(s, &t))
        .child(line)
        .child(
            trace_icon(icon(Icon::ChevronRight))
                .transition_transform()
                .rotate(turned),
        )
}

/// What the step produced, indented under a rule; `None` once closed and
/// rolled up. `body` fills the ruled box; the box around it rolls down from no
/// height as it opens and back up as it closes, 200ms (`animate-collapsible-*`).
#[must_use]
pub fn trace_collapsible_content(open: &Signal<bool>, body: impl FnOnce(Div) -> Div) -> Option<Div> {
    let t = theme();
    collapsible_content(open).map(|c| {
        c.named("trace-collapsible-content")
            .min_w(0.0)
            .overflow_hidden()
            .animate(Animation::accordion_down().after_mount())
            .animate_out(Animation::accordion_up())
            .duration_ms(200.0)
            .child(body(
                div()
                    .min_w(0.0)
                    .mt(2.0)
                    .mb(1.0)
                    // `ml-[calc(--spacing(2)-1px)]`: the rule under the icon's centre.
                    .ml(1.75)
                    .border_l()
                    .border_color(t.border)
                    .pr(1.0)
                    .pl(4.0)
                    .leading_relaxed()
                    .text_color(t.muted_foreground),
            ))
    })
}
