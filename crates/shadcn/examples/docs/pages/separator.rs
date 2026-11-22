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
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .child(
                        div()
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::Column)
                            .gap(4.0)
                            .child(
                                text("Radix Primitives")
                                    .text_size(14.0)
                                    .line_height(20.0)
                                    .font_weight(500)
                                    .line_height(LineHeight::Relative(1.0)),
                            )
                            .child(
                                text("An open-source UI component library.")
                                    .text_size(14.0)
                                    .line_height(20.0)
                                    .text_color(t.muted_foreground),
                            ),
                    )
                    .child(separator(Orientation::Horizontal).margin_y(16.0))
                    .child(
                        div()
                            .display(Display::Flex)
                            .height(20.0)
                            .align_items(Align::Center)
                            .gap(16.0)
                            .text_size(14.0)
                            .line_height(20.0)
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
