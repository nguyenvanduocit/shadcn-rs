//! Input (`ui/input`): a one-line text field, and the same frame around a
//! time input (`type="time"`) and a file picker (`type="file"`).

use gui::prelude::*;
use gui::{Color, Div, Properties, Signal, TextField, TextInput, TimeField, div, text, text_input, time_input};

use crate::button_group::with_style;
use crate::focus::focus_ring;
use crate::theme::{Theme, theme};

/// A text input editing `field`.
#[must_use]
pub fn input(field: &TextField) -> TextInput {
    let t = theme();
    frame(text_input(field).named("input"), &t)
        .placeholder_color(t.muted_foreground)
        .selection_color(t.primary.opacity(0.35))
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
}

/// A time input (`<Input type="time" step="1">`, its picker indicator
/// hidden as shadcn's demos hide it): the hour, minute and second fields of
/// `field` in the input's frame, the ring showing while one holds focus.
#[must_use]
pub fn input_time(field: &TimeField) -> Div {
    let t = theme();
    // Chrome's `light-dark(highlight, …)` for the focused field, as it draws
    // it on the reference's macOS (measured: the light page's highlight, and
    // the dark page's).
    let highlight = if t.dark {
        Color::hex(0x0055_79a1)
    } else {
        Color::hex(0x00b3_d7fe)
    };
    let d = frame(time_input(field, move |s| s.bg(highlight).text_color(Color::BLACK)), &t);
    // `focus-visible:` matches the input while one of its fields holds focus.
    if field.focused() {
        with_style(d, |s| focus_ring(s, &t))
    } else {
        d
    }
}

/// A file input showing the file in `chosen`: the browser's "Choose File"
/// button, styled by the `file:` classes (`inline-flex h-7 text-sm
/// font-medium text-foreground`, 4px before the name), then the file's name.
/// `gui` has no native file dialog yet, so a click picks nothing; whoever
/// opens one sets `chosen`.
#[must_use]
pub fn input_file(chosen: &Signal<Option<String>>) -> Div {
    let t = theme();
    // One line from the content box's top: the 28px button overhangs the
    // 26px box below, and the name sits on the button's baseline.
    let line = || div().h(7.0).flex().items_center();
    frame(div().named("input-file"), &t)
        .flex()
        .items_start()
        .focusable()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        .child(line().mr(1.0).font_medium().child("Choose File"))
        .child(line().child(text(chosen.get().unwrap_or_else(|| "No file chosen".to_owned()))))
}

/// The box both draw: `border-input h-9 w-full min-w-0 rounded-md border
/// bg-transparent px-3 py-1 md:text-sm shadow-xs dark:bg-input/30`.
fn frame<S: Styled>(el: S, t: &Theme) -> S {
    el.transition_props(Properties::COLOR | Properties::SHADOW)
        .text_color(t.foreground)
        .border_color(t.input)
        .h(9.0)
        .w_full()
        .min_w(0.0)
        .rounded_md()
        .border()
        .bg(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .px(3.0)
        .py(1.0)
        .text_sm()
        .shadow_xs()
}
