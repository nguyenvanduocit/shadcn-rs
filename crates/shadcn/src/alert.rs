//! Alert (`ui/alert`): a callout with an optional icon, a title and a
//! description, in two variants.

use gui::prelude::*;
use gui::{Div, Icon, Text, Track, div, icon, text};

use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AlertVariant {
    #[default]
    Default,
    Destructive,
}

/// The alert box. With an `icon` it has the icon column on the left
/// (`has-[>svg]:grid-cols-[calc(var(--spacing)*4)_1fr] has-[>svg]:gap-x-3`);
/// add [`alert_title`] and [`alert_description`] with `.child`.
#[must_use]
pub fn alert(variant: AlertVariant, icon_name: Option<Icon>) -> Div {
    let t = theme();
    let icon_column = if icon_name.is_some() { 16.0 } else { 0.0 };
    let d = div()
        .named("alert")
        .position(Position::Relative)
        .width(pct(100.0))
        .radius(10.0)
        .border(1.0)
        .border_color(t.border)
        .padding_x(16.0)
        .padding_y(12.0)
        .text_size(14.0)
        .line_height(20.0)
        .display(Display::Grid)
        .grid_columns(vec![Track::Px(icon_column), Track::Fr(1.0)])
        .when(icon_name.is_some(), |d| d.column_gap(12.0))
        .row_gap(2.0)
        .align_items(Align::Start)
        .background(t.card);
    let d = match variant {
        AlertVariant::Default => d.text_color(t.card_foreground),
        AlertVariant::Destructive => d.text_color(t.destructive),
    };
    // `[&>svg]:size-4 [&>svg]:translate-y-0.5 [&>svg]:text-current`: the icon
    // takes the alert's colour; its 2px nudge down is a top margin, which
    // moves it the same without growing the 20px first row.
    d.when_some(icon_name, |d, i| d.child(icon(i).named("alert-icon").margin_top(2.0)))
}

#[must_use]
pub fn alert_title(title: impl Into<String>) -> Text {
    text(title)
        .named("alert-title")
        .grid_column_start(2)
        .line_clamp(1)
        .min_height(16.0)
        .font_weight(500)
        .tracking(-0.025)
}

/// The description under the title; pass the alert's `variant`, whose colour
/// it takes (`*:data-[slot=alert-description]:text-destructive/90`). A
/// paragraph in it is `leading_relaxed` (`[&_p]:leading-relaxed`).
#[must_use]
pub fn alert_description(variant: AlertVariant) -> Div {
    let t = theme();
    let color = match variant {
        AlertVariant::Default => t.muted_foreground,
        AlertVariant::Destructive => t.destructive.opacity(0.9),
    };
    div()
        .named("alert-description")
        .text_color(color)
        .grid_column_start(2)
        .text_size(14.0)
        .line_height(20.0)
}
