//! Switch (`ui/switch`): an on/off control with a sliding thumb.

use gui::prelude::*;
use gui::style::Calc;
use gui::{Color, Div, Signal, div};

use crate::focus::focus_ring;
use crate::theme::theme;

/// A switch bound to `checked`; a click, or Space while focused, flips it.
/// Disable it with `.disabled(true)`.
#[must_use]
pub fn switch(checked: &Signal<bool>) -> Div {
    let t = theme();
    let on = checked.get();
    let flip = checked.clone();
    let track = match (on, t.dark) {
        (true, _) => t.primary,
        (false, false) => t.input,
        (false, true) => t.input.opacity(0.8),
    };
    let thumb = match (on, t.dark) {
        (_, false) => t.background,
        (false, true) => t.foreground,
        (true, true) => t.primary_foreground,
    };
    div()
        .named("switch")
        .inline_flex()
        .h_px(18.4)
        .w(8.0)
        .shrink_0()
        .items_center()
        .rounded_full()
        .border()
        .border_color(Color::TRANSPARENT)
        .shadow_xs()
        .bg(track)
        .transition_all()
        .focusable()
        .on_click(move |_| flip.update(|c| !c))
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        .child(
            div()
                .named("switch-thumb")
                .pass_through()
                .block()
                .size(4.0)
                .rounded_full()
                .bg(thumb)
                .transition_transform()
                // `translate-x-[calc(100%-2px)]`: the thumb's width less 2px.
                .when(on, |d| {
                    d.translate_x(Calc {
                        fraction: 1.0,
                        px: -2.0,
                        ch: 0.0,
                    })
                }),
        )
}
