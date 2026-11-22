//! The Introduction: what shadcn-rs is, as the crate's README says it, and
//! where to go from here.

use gui::prelude::*;
use gui::{Div, Icon, div, icon, text};
use shadcn::{card, card_description, card_header, card_title, theme};

use super::catalog::{Catalog, Route};
use super::fidelity::Verdicts;
use super::prose::{h1, markdown};
use super::{Nav, Part};
use shadcn::focus::StateLook;

const README: &str = include_str!("../../../README.md");

/// A card that opens `route`.
fn link_card(nav: &Nav, route: Route, glyph: Icon, title: &str, about: String) -> Div {
    let t = theme();
    let go = nav.clone();
    card()
        .flex(Flex::grow(1.0))
        .gap(12.0)
        .padding_y(20.0)
        .cursor(Cursor::Pointer)
        .focusable()
        .with_interaction(|mut d, i| {
            let ring = d.style_mut().ring;
            let shown = Ring::new(2.0, t.ring);
            d.background_while(move || i.hovered(), t.accent)
                .ring(move || if i.focus_visible() { Some(shown) } else { ring })
        })
        .on_click(move |_| go.go(route))
        .child(
            card_header()
                .child(icon(glyph).size(20.0).margin_bottom(8.0))
                .child(card_title(title))
                .child(card_description(about)),
        )
}

/// The Introduction page.
pub fn parts(catalog: &Catalog, verdicts: &Verdicts, nav: &Nav) -> Vec<Part> {
    let mut sections = markdown(README).into_iter();
    let opening = sections.next().map(|(_, body)| body);
    let judged = verdicts.light.as_ref().map_or(0, Vec::len);
    let fidelity = if judged == 0 {
        "No verdict has run yet: how to judge every demo.".to_owned()
    } else {
        format!("{judged} demos scored against shadcn-vue's own render.")
    };
    let first = catalog.components.first().copied().unwrap_or(Route::Introduction);
    let cards = div()
        .display(Display::Flex)
        .gap(16.0)
        .child(link_card(
            nav,
            first,
            Icon::Layers,
            "Components",
            format!(
                "{} components, each with its demos live and the Rust behind them.",
                catalog.components.len()
            ),
        ))
        .child(link_card(
            nav,
            Route::Theming,
            Icon::Sun,
            "Theming",
            "The colour tokens every component reads, light and dark.".to_owned(),
        ))
        .child(link_card(
            nav,
            Route::Fidelity,
            Icon::FlaskConical,
            "Fidelity",
            fidelity,
        ));
    let header = div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(16.0)
        .child(h1("Introduction"))
        .when_some(opening, |d, o| d.child(o))
        .child(cards);
    let t = theme();
    let mut parts = vec![Part::new("top", None, header)];
    parts.extend(
        sections
            .enumerate()
            .map(|(i, (title, body))| Part::keyed(gui::key_of(&format!("readme-{i}")), title, body)),
    );
    parts.push(Part::new(
        "source",
        None,
        text("From crates/shadcn/README.md.")
            .text_size(12.0)
            .line_height(16.0)
            .text_color(t.muted_foreground),
    ));
    parts
}
