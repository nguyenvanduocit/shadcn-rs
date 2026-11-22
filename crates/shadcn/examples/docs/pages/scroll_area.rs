use gui::prelude::*;
use gui::{ScenarioError, div, text};
use shadcn::{Orientation, scroll_area, separator, theme};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    // `v1.2.0-beta.50` down to `.1`, as the demo lists them.
    let tags: Vec<String> = (1..=50).rev().map(|n| format!("v1.2.0-beta.{n}")).collect();
    Box::new(move || {
        let t = theme();
        page(
            "Scroll Area",
            demo(
                "ScrollAreaDemo.vue",
                scroll_area()
                    .height(288.0)
                    .width(192.0)
                    .radius(8.0)
                    .border(1.0)
                    .border_color(t.border)
                    .child(
                        div()
                            .padding(16.0)
                            .child(
                                text("Tags")
                                    .margin_bottom(16.0)
                                    .text_size(14.0)
                                    .line_height(20.0)
                                    .line_height(LineHeight::Relative(1.0))
                                    .font_weight(500),
                            )
                            .children(tags.iter().flat_map(|tag| {
                                [
                                    text(tag.clone()).text_size(14.0).line_height(20.0).into_element(),
                                    separator(Orientation::Horizontal).margin_y(8.0).into_element(),
                                ]
                            })),
                    ),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("v1.2.0-beta.48")?;
    s.hover(x, y);
    s.wheel(400.0);
    s.frame("wheeled 400")?;
    s.wheel(100_000.0);
    s.frame("wheeled to end")?;
    Ok(())
}
