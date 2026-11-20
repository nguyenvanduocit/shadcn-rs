use gui::prelude::*;
use gui::{div, text};
use shadcn::{Orientation, separator, theme};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    Box::new(|| {
        let t = theme();
        page(
            "Separator",
            demo(
                "SeparatorDemo.vue",
                div()
                    .flex_col()
                    .child(
                        div()
                            .flex_col()
                            .gap(1.0)
                            .child(text("Radix Primitives").text_sm().font_medium().leading_none())
                            .child(
                                text("An open-source UI component library.")
                                    .text_sm()
                                    .text_color(t.muted_foreground),
                            ),
                    )
                    .child(separator(Orientation::Horizontal).my(4.0))
                    .child(
                        div()
                            .flex()
                            .h(5.0)
                            .items_center()
                            .gap(4.0)
                            .text_sm()
                            .child("Blog")
                            .child(separator(Orientation::Vertical))
                            .child("Docs")
                            .child(separator(Orientation::Vertical))
                            .child("Source"),
                    ),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), gui::ScenarioError> {
    crate::still(s)
}
