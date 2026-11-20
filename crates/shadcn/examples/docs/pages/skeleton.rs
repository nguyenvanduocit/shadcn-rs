use gui::prelude::*;
use gui::{ScenarioError, div, relative};
use shadcn::{card, card_content, card_header, skeleton};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    Box::new(|| {
        page(
            "Skeleton",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "SkeletonDemo.vue",
                    // `space-x-4` and `space-y-2` space the children as these gaps do.
                    div()
                        .flex()
                        .items_center()
                        .gap(4.0)
                        .child(skeleton().h(12.0).w(12.0).rounded_full())
                        .child(
                            div()
                                .flex_col()
                                .gap(2.0)
                                .child(skeleton().h(4.0).w_px(250.0))
                                .child(skeleton().h(4.0).w_px(200.0)),
                        ),
                ))
                .child(section(
                    "Cards",
                    div().flex().gap(4.0).items_start().children((0..2).map(|_| {
                        card()
                            .w_px(240.0)
                            .child(
                                card_header()
                                    .child(skeleton().col_start(1).h(4.0).w_len(relative(2.0 / 3.0)))
                                    .child(skeleton().col_start(1).h(4.0).w_len(relative(0.5))),
                            )
                            .child(card_content().child(skeleton().aspect_square().w_full()))
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
