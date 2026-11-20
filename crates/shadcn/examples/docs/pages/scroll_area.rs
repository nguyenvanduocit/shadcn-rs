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
                    .h(72.0)
                    .w(48.0)
                    .rounded_md()
                    .border()
                    .border_color(t.border)
                    .child(
                        div()
                            .p(4.0)
                            .child(text("Tags").mb(4.0).text_sm().leading_none().font_medium())
                            .children(tags.iter().flat_map(|tag| {
                                [
                                    text(tag.clone()).text_sm().into_element(),
                                    separator(Orientation::Horizontal).my(2.0).into_element(),
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
