//! Textarea (`ui/textarea`): a multi-line text field that grows with its text.
//!
//! An [`Input`] that wraps its lines: it keeps its own text
//! (`default_value`) or the caller's (`model`, `v-model`), and Enter types a
//! newline.

use gui::{Color, Properties, Style, Styled};

use crate::input::Input;
use crate::theme::theme;

/// A textarea (`<Textarea>`): empty, and keeping its own text, until told
/// otherwise. Style it as any node; its own classes come first.
#[must_use]
pub fn textarea() -> Input {
    let t = theme();
    let style = Style::default()
        .transition_props(Properties::COLOR | Properties::SHADOW)
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
        .shadow_xs();
    Input::new("textarea", style).multiline()
}
