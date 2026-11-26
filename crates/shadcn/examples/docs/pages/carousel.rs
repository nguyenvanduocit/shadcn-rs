use std::time::Duration;

use gui::prelude::*;
use gui::{Div, Element, Model, ScenarioError, Signal, div, text};
use shadcn::{
    CarouselProps, Orientation, Part, card, card_content, carousel, carousel_content, carousel_item, carousel_next,
    carousel_previous, carousel_snaps, theme,
};

use crate::{Render, Shot, demo, page};

/// How many slides each demo has.
const SLIDES: usize = 5;

/// The slide every carousel demo shows: its number on a card. `square` is
/// `aspect-square`; `number` styles the `<span>` (its text size).
fn slide(n: usize, square: bool, number: impl FnOnce(gui::Text) -> gui::Text) -> Div {
    div().padding(4.0).child(
        card().child(
            card_content()
                .display(Display::Flex)
                .when(square, |e| e.aspect_ratio(1.0))
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .padding(24.0)
                .child(number(text(n.to_string()).font_weight(600))),
        ),
    )
}

/// `<Carousel>` with its `<CarouselContent>` of numbered slides and both arrows.
fn numbered(
    root: Part<CarouselProps>,
    track: impl FnOnce(Part<()>) -> Part<()>,
    item: impl Fn(Part<()>) -> Part<()>,
    content: impl Fn(usize) -> Div,
) -> Part<CarouselProps> {
    let slides = (1..=SLIDES).map(|n| item(carousel_item()).child(content(n)));
    root.width(pct(100.0))
        .max_width(320.0)
        .child(track(carousel_content()).children(slides))
        .child(carousel_previous())
        .child(carousel_next())
}

fn big(n: usize) -> Div {
    slide(n, true, |t: gui::Text| t.text_size(36.0).line_height(40.0))
}

pub fn render() -> Render {
    // The API demo shows the snap, so it keeps it.
    let api = Signal::labeled("carousel index", 0_usize);
    Box::new(move || {
        let t = theme();
        let demos: [Element; 6] = [
            demo("CarouselDemo.vue", numbered(carousel(), |d| d, |d| d, big)).into_element(),
            demo(
                "CarouselApi.vue",
                div()
                    .child(numbered(carousel().model(Model::bind(api)), |d| d, |d| d, big))
                    .child(
                        div()
                            .padding_y(8.0)
                            .text_align(TextAlign::Center)
                            .text_size(14.0)
                            .line_height(20.0)
                            .text_color(t.muted_foreground)
                            .child(format!(
                                "Slide {} of {}",
                                api.get().saturating_add(1),
                                carousel_snaps(SLIDES, 1.0)
                            )),
                    ),
            )
            .into_element(),
            demo(
                "CarouselPlugin.vue",
                // `stopOnMouseEnter`, and the demo's own `@mouseleave` plays it again.
                numbered(carousel().autoplay(Duration::from_secs(2), true), |d| d, |d| d, big),
            )
            .into_element(),
            demo(
                "CarouselSize.vue",
                numbered(
                    carousel().basis(1.0 / 3.0),
                    |d| d,
                    |d| d,
                    |n| slide(n, true, |t: gui::Text| t.text_size(30.0).line_height(36.0)),
                ),
            )
            .into_element(),
            demo(
                "CarouselSpacing.vue",
                numbered(
                    carousel().basis(1.0 / 3.0),
                    |d| d.margin_left(-4.0),
                    |d| d.padding_left(4.0),
                    |n| slide(n, true, |t: gui::Text| t.text_size(24.0).line_height(32.0)),
                )
                .max_width(384.0),
            )
            .into_element(),
            demo(
                "CarouselOrientation.vue",
                numbered(
                    carousel().orientation(Orientation::Vertical).basis(0.5),
                    |d| d.margin_top(-4.0).height(200.0),
                    |d| d.padding(4.0),
                    |n| slide(n, false, |t: gui::Text| t.text_size(30.0).line_height(36.0)),
                ),
            )
            .into_element(),
        ];
        page(
            "Carousel",
            div()
                .flex(Flex::grow(1.0))
                .min_height(0.0)
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(96.0)
                .padding_x(48.0)
                .padding_y(48.0)
                .children(demos),
        )
    })
}

/// The centre of the `nth` node named `name` in the last frame.
fn named(s: &Shot, name: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name))
        .nth(nth)
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .map(|b| (b.x + b.width / 2.0, b.y + b.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(name.to_owned()))
}

/// Scroll the page until the `nth` carousel's next button is in the window.
fn scroll_to(s: &mut Shot, nth: usize) -> Result<(f32, f32), ScenarioError> {
    for _ in 0..40 {
        let (x, y) = named(s, "carousel-next", nth)?;
        if (120.0..520.0).contains(&y) {
            return Ok((x, y));
        }
        s.hover(400.0, 320.0);
        s.wheel(if y < 120.0 { -100.0 } else { 100.0 });
        s.settle()?;
    }
    named(s, "carousel-next", nth)
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("first slides, previous disabled")?;
    let (x, y) = named(s, "carousel-next", 0)?;
    s.click(x, y);
    s.at(0.1);
    s.frame("moving to 2")?;
    s.at(1.0);
    s.frame("on 2")?;
    s.click(x, y);
    s.click(x, y);
    s.click(x, y);
    s.at(2.0);
    s.frame("on 5, next disabled")?;
    let (x, y) = named(s, "carousel-previous", 0)?;
    s.click(x, y);
    s.at(3.0);
    s.frame("back on 4")?;
    autoplay(s)?;
    // CarouselSize: three slides a view, so three snaps.
    let (x, y) = scroll_to(s, 3)?;
    s.click(x, y);
    s.click(x, y);
    s.at(11.0);
    s.frame("size on its last snap, 3 to 5 showing, next disabled")?;
    // CarouselOrientation: slides stacked, moved down.
    let (x, y) = scroll_to(s, 5)?;
    s.click(x, y);
    s.at(12.0);
    s.frame("vertical moved one slide down")?;
    Ok(())
}

/// CarouselPlugin: autoplay advances a slide every 2 s on the scenario's
/// clock, stops while the pointer is over it and plays again once it leaves.
fn autoplay(s: &mut Shot) -> Result<(), ScenarioError> {
    let (x, y) = scroll_to(s, 2)?;
    // Off the carousel, so it plays from now (3 s): it moved to 2 at 2 s.
    s.hover(900.0, 600.0);
    s.settle()?;
    s.frame("autoplay on 2")?;
    s.at(5.5);
    s.frame("autoplay advanced to 3 at 5 s")?;
    s.hover(x - 160.0, y);
    s.at(9.0);
    s.frame("hovered at 5.5 s, autoplay stopped on 3")?;
    s.hover(900.0, 600.0);
    s.at(10.0);
    s.frame("left at 9 s, still on 3 until 11 s")?;
    Ok(())
}
