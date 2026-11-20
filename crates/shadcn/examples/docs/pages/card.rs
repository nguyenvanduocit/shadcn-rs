use gui::prelude::*;
use gui::{TextField, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, button, card, card_action, card_content, card_description, card_footer, card_header,
    card_title, input, label,
};

use crate::{Render, demo, page};

pub fn render() -> Render {
    let email = TextField::new("");
    let password = TextField::new("");
    Box::new(move || {
        page(
            "Card",
            demo(
                "CardDemo.vue",
                card()
                    .w_full()
                    .max_w_px(384.0)
                    .child(
                        card_header()
                            .child(card_title("Login to your account"))
                            .child(card_description("Enter your email below to login to your account"))
                            .child(
                                card_action().child(button(ButtonVariant::Link, ButtonSize::Default).child("Sign Up")),
                            ),
                    )
                    .child(
                        card_content().child(
                            div()
                                .grid()
                                .w_full()
                                .items_center()
                                .gap(4.0)
                                .child(
                                    div()
                                        .flex_col()
                                        .gap(1.5)
                                        .child(label("Email"))
                                        .child(input(&email).placeholder("m@example.com")),
                                )
                                .child(
                                    div()
                                        .flex_col()
                                        .gap(1.5)
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .child(label("Password"))
                                                .child(text("Forgot your password?").ml_auto().text_sm().underline()),
                                        )
                                        .child(input(&password)),
                                ),
                        ),
                    )
                    .child(
                        card_footer()
                            .flex_col()
                            .gap(2.0)
                            .child(
                                button(ButtonVariant::Default, ButtonSize::Default)
                                    .w_full()
                                    .child("Login"),
                            )
                            .child(
                                button(ButtonVariant::Outline, ButtonSize::Default)
                                    .w_full()
                                    .child("Login with Google"),
                            ),
                    ),
            ),
        )
    })
}

pub fn script(s: &mut crate::Shot) -> Result<(), gui::ScenarioError> {
    crate::still(s)
}
