//! The Theming page: every token of a [`Palette`], light and dark, as a swatch.

use gui::prelude::*;
use gui::{Color, Div, Track, div, text};
use shadcn::theme::Palette;
use shadcn::{Theme, table, table_body, table_head, table_header, table_row, theme};

use super::Part;
use super::prose::{h1, h2, inline, lead, paragraph};

/// Every token of `t`, named as shadcn names it. The pattern names every
/// field, so a token added to [`Palette`] fails to build until it shows here.
pub(super) fn tokens(t: Palette<Color>) -> Vec<(String, Color)> {
    let Palette {
        background,
        foreground,
        card,
        card_foreground,
        popover,
        popover_foreground,
        primary,
        primary_foreground,
        secondary,
        secondary_foreground,
        muted,
        muted_foreground,
        accent,
        accent_foreground,
        destructive,
        border,
        input,
        ring,
        chart,
        sidebar,
        sidebar_foreground,
        sidebar_primary,
        sidebar_primary_foreground,
        sidebar_accent,
        sidebar_accent_foreground,
        sidebar_border,
        sidebar_ring,
        code,
        code_foreground,
    } = t;
    let named = [
        ("background", background),
        ("foreground", foreground),
        ("card", card),
        ("card-foreground", card_foreground),
        ("popover", popover),
        ("popover-foreground", popover_foreground),
        ("primary", primary),
        ("primary-foreground", primary_foreground),
        ("secondary", secondary),
        ("secondary-foreground", secondary_foreground),
        ("muted", muted),
        ("muted-foreground", muted_foreground),
        ("accent", accent),
        ("accent-foreground", accent_foreground),
        ("destructive", destructive),
        ("border", border),
        ("input", input),
        ("ring", ring),
    ];
    let charts = chart
        .iter()
        .enumerate()
        .map(|(i, c)| (format!("chart-{}", i.saturating_add(1)), *c));
    let sidebars = [
        ("sidebar", sidebar),
        ("sidebar-foreground", sidebar_foreground),
        ("sidebar-primary", sidebar_primary),
        ("sidebar-primary-foreground", sidebar_primary_foreground),
        ("sidebar-accent", sidebar_accent),
        ("sidebar-accent-foreground", sidebar_accent_foreground),
        ("sidebar-border", sidebar_border),
        ("sidebar-ring", sidebar_ring),
        ("code", code),
        ("code-foreground", code_foreground),
    ];
    named
        .into_iter()
        .map(|(n, c)| (n.to_owned(), c))
        .chain(charts)
        .chain(sidebars.into_iter().map(|(n, c)| (n.to_owned(), c)))
        .collect()
}

/// `c` as CSS writes shadcn's tokens: `oklch(L C H)`, with `/ A%` when translucent.
pub(super) fn oklch(c: Color) -> String {
    let [l, a, b] = c.to_oklab();
    let chroma = a.hypot(b);
    let hue = if chroma < 0.0005 {
        0.0
    } else {
        b.atan2(a).to_degrees().rem_euclid(360.0)
    };
    let num = |v: f32, places: usize| {
        let s = format!("{v:.places$}");
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s.is_empty() || s == "-" {
            "0".to_owned()
        } else {
            s.to_owned()
        }
    };
    let alpha = if c.a < 0.999 {
        format!(" / {}%", num(c.a * 100.0, 0))
    } else {
        String::new()
    };
    format!("oklch({} {} {}{alpha})", num(l, 3), num(chroma, 3), num(hue, 1))
}

/// A token's swatch on the background of the theme it belongs to, and its value.
fn swatch(on: Palette<Color>, c: Color) -> Div {
    div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(12.0)
        .padding(8.0)
        .radius(8.0)
        .background(on.background)
        .text_color(on.muted_foreground)
        .child(
            div()
                .size(32.0)
                .flex_shrink(0.0)
                .radius(8.0)
                .border(1.0)
                .border_color(on.foreground.opacity(0.15))
                .background(c),
        )
        .child(
            text(oklch(c))
                .font_family(FontFamily::GEIST_MONO)
                .text_size(12.0)
                .line_height(16.0)
                .truncate(true)
                .nowrap(true)
                .overflow_x(Overflow::Hidden)
                .min_width(0.0),
        )
}

/// The Theming page.
pub fn parts(doc: &str) -> Vec<Part> {
    let t = theme();
    let colors = Theme::default().colors;
    let (light, dark) = (colors.light.painted(), colors.dark.painted());
    let header = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .child(h1("Theming"))
        .child(lead(&doc.replace('\n', " ")));
    let usage = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(12.0)
        .child(h2("Using the tokens"))
        .child(paragraph(
            "A component reads its colours from `let t = theme();` and nothing else: `bg-primary/90` is \
             `.background(t.primary.opacity(0.9))`, `dark:` is `.when(t.dark, …)`.",
        ))
        .child(paragraph(
            "The theme is installed with `install(theme, VEGA)`; writing its scheme (`use_theme().update(|t| t.scheme = \
             Scheme::Dark)`) re-renders every component that read it, which is what the sun and moon in the header do.",
        ));
    let columns = [Track::Px(210.0), Track::Fr(1.0), Track::Fr(1.0)];
    let rows = tokens(light).into_iter().zip(tokens(dark)).map(|((name, l), (_, d))| {
        table_row(&columns, false)
            .align_items(Align::Center)
            .child(
                div()
                    .padding_x(8.0)
                    .child(inline(&format!("`{name}`")).text_size(14.0).line_height(20.0)),
            )
            .child(div().padding(4.0).child(swatch(light, l)))
            .child(div().padding(4.0).child(swatch(dark, d)))
    });
    let table = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(12.0)
        .child(h2("Tokens"))
        .child(
            div()
                .radius(10.0)
                .border(1.0)
                .border_color(t.border)
                .overflow(Overflow::Hidden)
                .child(
                    table()
                        .child(table_header([table_row(&columns, false)
                            .child(table_head("Token"))
                            .child(table_head("Light"))
                            .child(table_head("Dark"))]))
                        .child(table_body(rows)),
                ),
        );
    vec![
        Part::new("top", None, header),
        Part::new("usage", Some("Using the tokens"), usage),
        Part::new("tokens", Some("Tokens"), table),
    ]
}
