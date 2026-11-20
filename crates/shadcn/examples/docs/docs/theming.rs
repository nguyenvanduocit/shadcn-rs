//! The Theming page: every token of [`Theme`], light and dark, as a swatch.

use gui::prelude::*;
use gui::{Color, Div, Track, div, text};
use shadcn::{Theme, table, table_body, table_head, table_header, table_row, theme};

use super::Part;
use super::prose::{h1, h2, inline, lead, paragraph};

/// Every token of `t`, named as shadcn names it. The pattern names every
/// field, so a token added to [`Theme`] fails to build until it shows here.
fn tokens(t: Theme) -> Vec<(String, Color)> {
    let Theme {
        dark: _,
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
fn oklch(c: Color) -> String {
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
fn swatch(on: Theme, c: Color) -> Div {
    div()
        .flex()
        .items_center()
        .gap(3.0)
        .p(2.0)
        .rounded_md()
        .bg(on.background)
        .text_color(on.muted_foreground)
        .child(
            div()
                .size(8.0)
                .shrink_0()
                .rounded_md()
                .border()
                .border_color(on.foreground.opacity(0.15))
                .bg(c),
        )
        .child(text(oklch(c)).font_mono().text_xs().truncate())
}

/// The Theming page.
pub fn parts(doc: &str) -> Vec<Part> {
    let t = theme();
    let (light, dark) = (Theme::light(), Theme::dark());
    let header = div()
        .flex_col()
        .gap(2.0)
        .child(h1("Theming"))
        .child(lead(&doc.replace('\n', " ")));
    let usage = div()
        .flex_col()
        .gap(3.0)
        .child(h2("Using the tokens"))
        .child(paragraph(
            "A component reads its colours from `let t = theme();` and nothing else: `bg-primary/90` is \
             `.bg(t.primary.opacity(0.9))`, `dark:` is `.when(t.dark, …)`.",
        ))
        .child(paragraph(
            "The theme is a `Signal<Theme>` handed to `provide_theme`; setting it to `Theme::dark()` re-renders every \
             component, which is what the sun and moon in the header do.",
        ));
    let columns = [Track::Px(210.0), Track::Fr(1.0), Track::Fr(1.0)];
    let rows = tokens(light).into_iter().zip(tokens(dark)).map(|((name, l), (_, d))| {
        table_row(&columns, false)
            .items_center()
            .child(div().px(2.0).child(inline(&format!("`{name}`")).text_sm()))
            .child(div().p(1.0).child(swatch(light, l)))
            .child(div().p(1.0).child(swatch(dark, d)))
    });
    let table = div().flex_col().gap(3.0).child(h2("Tokens")).child(
        div()
            .rounded_lg()
            .border()
            .border_color(t.border)
            .overflow_hidden()
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
