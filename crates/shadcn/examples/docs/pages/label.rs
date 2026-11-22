use gui::prelude::*;
use gui::{Signal, div};
use shadcn::{checkbox, label};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let terms = Signal::labeled("terms", false);
    Box::new(move || {
        let flip = terms;
        page(
            "Label",
            demo(
                "LabelDemo.vue",
                div()
                    .flex()
                    .items_center()
                    .gap(2.0)
                    .child(checkbox().model(terms))
                    // `for="terms"`: a click on the label flips the checkbox.
                    .child(label("Accept terms and conditions").on_click(move |_| flip.update(|v| *v = !*v))),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), gui::ScenarioError> {
    crate::still(s)
}
