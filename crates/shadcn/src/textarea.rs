//! Textarea (`ui/textarea`): a multi-line text field that grows with its text.
//!
//! An [`Input`] that wraps its lines: it keeps its own text
//! (`default_value`) or the caller's (`model`, `v-model`), and Enter types a
//! newline.

use gui::prelude::*;
use gui::{Color, Properties, Style, Styled};

use crate::input::Input;
use crate::theme::theme;

/// A textarea (`<Textarea>`): empty, and keeping its own text, until told
/// otherwise. Style it as any node; its own classes come first.
#[must_use]
pub fn textarea() -> Input {
    let t = theme();
    let style = Style::default()
        .transition(Properties::COLOR | Properties::SHADOW)
        .text_color(t.foreground)
        .border_color(t.input)
        .display(Display::Flex)
        .min_height(64.0)
        .width(pct(100.0))
        .radius(8.0)
        .border(1.0)
        .background(if t.dark {
            t.input.opacity(0.3)
        } else {
            Color::TRANSPARENT
        })
        .padding_x(12.0)
        .padding_y(8.0)
        // `text-base md:text-sm`: a desktop window is taken to be past `md`.
        .text_size(14.0)
        .line_height(20.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        });
    Input::new("textarea", style).multiline()
}
