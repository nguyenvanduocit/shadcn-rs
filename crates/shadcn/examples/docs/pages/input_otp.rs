use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, FieldOrientation, InputOtpPattern, button, field, field_description, field_error,
    field_group, field_label, input_otp, input_otp_group, input_otp_separator, input_otp_slot,
};

use crate::{Render, Shot, demo, page};

fn slots(range: std::ops::Range<usize>) -> gui::Div {
    input_otp_group(range.map(input_otp_slot))
}

pub fn render() -> Render {
    let controlled = Signal::labeled("otp", String::new());
    let form = Signal::labeled("otp form", String::new());
    let submitted = Signal::labeled("submitted", false);
    Box::new(move || {
        let entered = controlled.get();
        let form_error = (submitted.get() && form.with(|v| v.chars().count() < 6))
            .then_some("Your one-time password must be 6 characters.");
        let submit = submitted;
        page(
            "Input OTP",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "InputOTPDemo.vue",
                    input_otp()
                        .child(slots(0..3))
                        .child(input_otp_separator())
                        .child(slots(3..6)),
                ))
                .child(demo(
                    "InputOTPSeparatorDemo.vue",
                    input_otp()
                        .child(slots(0..2))
                        .child(input_otp_separator())
                        .child(slots(2..4))
                        .child(input_otp_separator())
                        .child(slots(4..6)),
                ))
                .child(demo(
                    "InputOTPControlledDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(8.0)
                        .child(input_otp().model(controlled).child(slots(0..6)))
                        .child(
                            text(if entered.is_empty() {
                                "Enter your one-time password.".to_owned()
                            } else {
                                format!("You entered: {entered}")
                            })
                            .text_size(14.0)
                            .line_height(20.0)
                            .text_align(TextAlign::Center),
                        ),
                ))
                .child(demo(
                    "InputOTPPatternDemo.vue",
                    input_otp().pattern(InputOtpPattern::DigitsAndChars).child(slots(0..6)),
                ))
                .child(demo(
                    "InputOTPFormDemo.vue",
                    div()
                        .width(384.0)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(24.0)
                        .child(
                            field_group().child(
                                field(FieldOrientation::Vertical, form_error.is_some())
                                    .child(field_label("One-Time Password"))
                                    .child(input_otp().model(form).child(input_otp_group(
                                        (0..6).map(|i| input_otp_slot(i).invalid(form_error.is_some())),
                                    )))
                                    .child(field_description(
                                        "Please enter the one-time password sent to your phone.",
                                    ))
                                    .child(field_error(form_error)),
                            ),
                        )
                        .child(
                            div().child(
                                button(ButtonVariant::Default, ButtonSize::Default)
                                    .child("Submit")
                                    .on_click(move |_| submit.set(true)),
                            ),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let first = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("input-otp-slot"))
        .map(|n| n.bounds);
    let Some(b) = first else {
        return Err(ScenarioError::TextNotFound("input-otp-slot".to_owned()));
    };
    s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
    s.frame("focused")?;
    s.type_text("12a3");
    s.frame("typed 12a3")?;
    s.press(NamedKey::Backspace, Modifiers::default());
    s.frame("backspace")?;
    s.type_text("34567");
    s.frame("full")?;
    s.click_text("Submit")?;
    s.frame("form invalid")?;
    Ok(())
}
