//! Marker (`ui/marker`): a quiet line in a thread (a branch switch, files
//! explored, "conversation compacted"), plain, between two rules, or above a
//! bottom border.

use gui::prelude::*;
use gui::{Div, div};

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
        .relative()
        .flex()
        .w_full()
        .items_center()
        .gap(2.0)
        .min_h(4.0)
        .text_sm()
        .text_left()
        .text_color(t.muted_foreground);
    match variant {
        MarkerVariant::Default => d.child(icon).child(content),
        MarkerVariant::Separator => d.child(rule().mr(1.0)).child(icon).child(content).child(rule().ml(1.0)),
        MarkerVariant::Border => d.border_b().border_color(t.border).pb(2.0).child(icon).child(content),
    }
}

fn rule() -> Div {
    div().h_px(1.0).min_w(0.0).flex_1().bg(theme().border)
}

/// A marker the user can press (Vue renders it as a `<button>` or `<a>`):
/// it takes focus and brightens on hover; give it `.on_click`.
#[must_use]
pub fn marker_button(marker: Div) -> Div {
    let t = theme();
    marker
        .focusable()
        .cursor_pointer()
        .rounded_md()
        .hover(move |s| s.text_color(t.foreground))
        .focus_visible(move |s| s.ring(3.0, t.ring.opacity(0.5)))
}

/// The 16px slot an icon or spinner sits in.
#[must_use]
pub fn marker_icon(child: impl IntoElement) -> Div {
    div()
        .named("marker-icon")
        .flex()
        .items_center()
        .justify_center()
        .size(4.0)
        .shrink_0()
        .child(child)
}

/// The marker's words; between rules they keep their width and centre.
#[must_use]
pub fn marker_content(variant: MarkerVariant) -> Div {
    let d = div().named("marker-content").min_w(0.0);
    match variant {
        MarkerVariant::Separator => d.flex_none().text_center(),
        MarkerVariant::Default | MarkerVariant::Border => d,
    }
}

/// A marker that is a link (Vue's `<a>` marker): a [`marker_button`], underlined.
#[must_use]
pub fn marker_link(marker: Div) -> Div {
    marker_button(marker).underline()
}
