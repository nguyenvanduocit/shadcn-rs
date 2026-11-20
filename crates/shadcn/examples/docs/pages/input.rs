use gui::prelude::*;
use gui::{ScenarioError, Signal, TextField, div, text};
use shadcn::{ButtonSize, ButtonVariant, button, input, input_file, label, theme};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    let email = TextField::new("");
    let name = TextField::new("Nguyễn Văn A");
    let disabled = TextField::new("");
    let subscribe = TextField::new("");
    let labelled = TextField::new("");
    let described = TextField::new("");
    let picture = Signal::labeled("picture", None::<String>);
    Box::new(move || {
        let t = theme();
        // `grid w-full max-w-sm items-center gap-1.5`.
        let stack = || div().grid().w_full().max_w_px(384.0).items_center().gap(1.5);
        page(
            "Input",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("InputDemo.vue", input(&email).placeholder("Email")))
                .child(
                    div()
                        .grid()
                        .max_w_px(384.0)
                        .gap(3.0)
                        .child(label("Name"))
                        .child(input(&name)),
                )
                .child(section(
                    "shadcn-vue demos",
                    div()
                        .flex_col()
                        .gap(6.0)
                        .child(demo(
                            "InputDisabled.vue",
                            input(&disabled).placeholder("Email").disabled(true),
                        ))
                        .child(demo(
                            "InputFile.vue",
                            stack().child(label("Picture")).child(input_file(&picture)),
                        ))
                        .child(demo(
                            "InputWithButton.vue",
                            // `space-x-2`: the button 0.5rem after the input.
                            div()
                                .flex()
                                .w_full()
                                .max_w_px(384.0)
                                .items_center()
                                .gap(2.0)
                                .child(input(&subscribe).placeholder("Email"))
                                .child(button(ButtonVariant::Default, ButtonSize::Default).child("Subscribe")),
                        ))
                        .child(demo(
                            "InputWithLabel.vue",
                            stack()
                                .child(label("Email"))
                                .child(input(&labelled).placeholder("Email")),
                        ))
                        .child(demo(
                            "InputWithText.vue",
                            stack()
                                .child(label("Email"))
                                .child(input(&described).placeholder("Email"))
                                .child(
                                    text("Enter your email address.")
                                        .text_sm()
                                        .text_color(t.muted_foreground),
                                ),
                        )),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Email")?;
    s.click(x, y);
    s.type_text("hello@example.com");
    s.frame("typed")?;
    Ok(())
}
