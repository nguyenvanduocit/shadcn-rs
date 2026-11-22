use gui::prelude::*;
use gui::{ScenarioError, div};
use shadcn::{card, card_content, card_header, skeleton};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    Box::new(|| {
        page(
            "Skeleton",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "SkeletonDemo.vue",
                    // `space-x-4` and `space-y-2` space the children as these gaps do.
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(skeleton().height(48.0).width(48.0).radius(9999.0))
                        .child(
                            div()
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .gap(8.0)
                                .child(skeleton().height(16.0).width(250.0))
                                .child(skeleton().height(16.0).width(200.0)),
                        ),
                ))
                .child(section(
                    "Cards",
                    div()
                        .display(Display::Flex)
                        .gap(16.0)
                        .align_items(Align::Start)
                        .children((0..2).map(|_| {
                            card()
                                .width(240.0)
                                .child(
                                    card_header()
                                        .child(
                                            skeleton()
                                                .grid_column_start(1)
                                                .height(16.0)
                                                .width(Length::Fraction(2.0 / 3.0)),
                                        )
                                        .child(
                                            skeleton()
                                                .grid_column_start(1)
                                                .height(16.0)
                                                .width(Length::Fraction(0.5)),
                                        ),
                                )
                                .child(card_content().child(skeleton().aspect_ratio(1.0).width(pct(100.0))))
                        })),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("pulse start, opacity 1")?;
    s.at(1.0);
    s.frame("pulse middle, opacity 0.5")?;
    Ok(())
}
