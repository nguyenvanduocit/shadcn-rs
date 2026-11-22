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
                    .background(t.muted)
                    .radius(10.0)
                    .child(source.clone().map(|s| {
                        image(s)
                            .size(pct(100.0))
                            .radius(10.0)
                            // `dark:brightness-[0.2] dark:grayscale`.
                            .when(t.dark, |i| i.filters([Filter::Brightness(0.2), Filter::Grayscale(1.0)]))
                    })),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
