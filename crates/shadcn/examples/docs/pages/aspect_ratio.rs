use gui::prelude::*;
use gui::{ScenarioError, image};
use shadcn::{aspect_ratio, theme};

use crate::{Render, Shot, demo, page, photo};

pub fn render() -> Render {
    let name = "photo-1588345921523-c2dcdb7f1dcd";
    let source = photo(name);
    Box::new(move || {
        let t = theme();
        page(
            "Aspect Ratio",
            demo(
                "AspectRatioDemo.vue",
                aspect_ratio(16.0 / 9.0)
                    .bg(t.muted)
                    .rounded_lg()
                    .child(source.clone().map(|s| {
                        image(s)
                            .size_full()
                            .rounded_lg()
                            // `dark:brightness-[0.2] dark:grayscale`.
                            .when(t.dark, |i| i.brightness(0.2).grayscale())
                    })),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
