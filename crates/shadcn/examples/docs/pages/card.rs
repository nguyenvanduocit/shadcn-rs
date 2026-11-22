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
                    .width(pct(100.0))
                    .max_width(384.0)
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
                                .display(Display::Grid)
                                .width(pct(100.0))
                                .align_items(Align::Center)
                                .gap(16.0)
                                .child(
                                    div()
                                        .display(Display::Flex)
                                        .flex_direction(FlexDirection::Column)
                                        .gap(6.0)
                                        .child(label("Email"))
                                        .child(input().field(email).placeholder("m@example.com")),
                                )
                                .child(
                                    div()
                                        .display(Display::Flex)
                                        .flex_direction(FlexDirection::Column)
                                        .gap(6.0)
                                        .child(
                                            div()
                                                .display(Display::Flex)
                                                .align_items(Align::Center)
                                                .child(label("Password"))
                                                .child(
                                                    text("Forgot your password?")
                                                        .margin_left(Length::Auto)
                                                        .text_size(14.0)
                                                        .line_height(20.0)
                                                        .underline(true),
                                                ),
                                        )
                                        .child(input().field(password)),
                                ),
                        ),
                    )
                    .child(
                        card_footer()
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::Column)
                            .gap(8.0)
                            .child(
                                button(ButtonVariant::Default, ButtonSize::Default)
                                    .width(pct(100.0))
                                    .child("Login"),
                            )
                            .child(
                                button(ButtonVariant::Outline, ButtonSize::Default)
                                    .width(pct(100.0))
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
