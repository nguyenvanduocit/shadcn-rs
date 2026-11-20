//! Textarea (`ui/textarea`): a multi-line text field that grows with its text.

use gui::{Color, Properties, Styled, TextField, TextInput, text_input};

use crate::focus::focus_ring;
use crate::theme::theme;

/// A textarea editing `field`; Enter types a newline.
#[must_use]
pub fn textarea(field: &TextField) -> TextInput {
    let t = theme();
    text_input(field)
        .multiline()
        .named("textarea")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .placeholder_color(t.muted_foreground)
        .selection_color(t.primary.opacity(0.35))
        .text_color(t.foreground)
        .border_color(t.input)
        .flex()
        .min_h(16.0)
        .w_full()
        .rounded_md()
        .border()
        .bg(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .px(3.0)
        .py(2.0)
        // `text-base md:text-sm`: a desktop window is taken to be past `md`.
        .text_sm()
        .shadow_xs()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
}
