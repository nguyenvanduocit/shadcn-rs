//! Marker (`ui/marker`): a quiet line in a thread (a branch switch, files
//! explored, "conversation compacted"), plain, between two rules, or above a
//! bottom border.

use gui::prelude::*;
use gui::{Div, div};

use crate::focus::StateLook;
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MarkerVariant {
    #[default]
    Default,
    /// Centred between two rules.
    Separator,
    /// Above a bottom border.
    Border,
}

/// A marker: an optional [`marker_icon`] (pass `Element::Empty` for none)
/// then its [`marker_content`]. The separator's two rules are Vue's
/// `::before` and `::after`, so the marker lays out its parts itself.
#[must_use]
pub fn marker(variant: MarkerVariant, icon: impl IntoElement, content: impl IntoElement) -> Div {
    let t = theme();
    let d = div()
        .named("marker")
        .position(Position::Relative)
        .display(Display::Flex)
        .width(pct(100.0))
        .align_items(Align::Center)
        .gap(8.0)
        .min_height(16.0)
        .text_size(14.0)
        .line_height(20.0)
        .text_align(TextAlign::Left)
        .text_color(t.muted_foreground);
    match variant {
        MarkerVariant::Default => d.child(icon).child(content),
        MarkerVariant::Separator => d
            .child(rule().margin_right(4.0))
            .child(icon)
            .child(content)
            .child(rule().margin_left(4.0)),
        MarkerVariant::Border => d
            .border_bottom(1.0)
            .border_color(t.border)
            .padding_bottom(8.0)
            .child(icon)
            .child(content),
    }
}

fn rule() -> Div {
    div()
        .height(1.0)
        .min_width(0.0)
        .flex(Flex::grow(1.0))
        .background(theme().border)
}

/// A marker the user can press (Vue renders it as a `<button>` or `<a>`):
/// it takes focus and brightens on hover; give it `.on_click`.
#[must_use]
pub fn marker_button(marker: Div) -> Div {
    let t = theme();
    marker
        .focusable()
        .cursor(Cursor::Pointer)
        .radius(8.0)
        .with_interaction(|mut d, i| {
            let ring = d.style_mut().ring;
            let shown = Ring::new(3.0, t.ring.opacity(0.5));
            d.text_color_while(move || i.hovered(), t.foreground)
                .ring(move || if i.focus_visible() { Some(shown) } else { ring })
        })
}

/// The 16px slot an icon or spinner sits in.
#[must_use]
pub fn marker_icon(child: impl IntoElement) -> Div {
    div()
        .named("marker-icon")
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .size(16.0)
        .flex_shrink(0.0)
        .child(child)
}

/// The marker's words; between rules they keep their width and centre.
#[must_use]
pub fn marker_content(variant: MarkerVariant) -> Div {
    let d = div().named("marker-content").min_width(0.0);
    match variant {
        MarkerVariant::Separator => d.flex(Flex::NONE).text_align(TextAlign::Center),
        MarkerVariant::Default | MarkerVariant::Border => d,
    }
}

/// A marker that is a link (Vue's `<a>` marker, `[a]:underline
/// [a]:underline-offset-3`): a [`marker_button`], underlined.
#[must_use]
pub fn marker_link(marker: Div) -> Div {
    marker_button(marker).underline(true).underline_offset(3.0)
}
