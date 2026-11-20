use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    ButtonSize, ButtonVariant, FieldOrientation, InputOtpPattern, InputOtpState, button, field, field_description,
    field_error, field_group, field_label, input_otp, input_otp_group, input_otp_separator, input_otp_slot,
};

use crate::{Render, Shot, demo, page};

fn slots(state: &InputOtpState, range: std::ops::Range<usize>) -> gui::Div {
    input_otp_group(range.map(|i| input_otp_slot(state, i, false)))
}

pub fn render() -> Render {
    let plain = InputOtpState::new(6, InputOtpPattern::Any);
    let pairs = InputOtpState::new(6, InputOtpPattern::Any);
    let controlled = InputOtpState::new(6, InputOtpPattern::Any);
    let pattern = InputOtpState::new(6, InputOtpPattern::DigitsAndChars);
    let form = InputOtpState::new(6, InputOtpPattern::Any);
    let submitted = Signal::labeled("submitted", false);
    Box::new(move || {
        let entered = controlled.value.get();
        let form_error = (submitted.get() && form.value.get().chars().count() < 6)
            .then_some("Your one-time password must be 6 characters.");
        let submit = submitted.clone();
        page(
            "Input OTP",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "InputOTPDemo.vue",
                    input_otp(&plain)
                        .child(slots(&plain, 0..3))
                        .child(input_otp_separator())
                        .child(slots(&plain, 3..6)),
                ))
                .child(demo(
                    "InputOTPSeparatorDemo.vue",
                    input_otp(&pairs)
                        .child(slots(&pairs, 0..2))
                        .child(input_otp_separator())
                        .child(slots(&pairs, 2..4))
                        .child(input_otp_separator())
                        .child(slots(&pairs, 4..6)),
                ))
                .child(demo(
                    "InputOTPControlledDemo.vue",
                    div()
                        .flex_col()
                        .gap(2.0)
                        .child(input_otp(&controlled).child(slots(&controlled, 0..6)))
                        .child(
                            text(if entered.is_empty() {
                                "Enter your one-time password.".to_owned()
                            } else {
                                format!("You entered: {entered}")
                            })
                            .text_sm()
                            .text_center(),
                        ),
                ))
                .child(demo(
                    "InputOTPPatternDemo.vue",
                    input_otp(&pattern).child(slots(&pattern, 0..6)),
                ))
                .child(demo(
                    "InputOTPFormDemo.vue",
                    div()
                        .w(96.0)
                        .flex_col()
                        .gap(6.0)
                        .child(
                            field_group().child(
                                field(FieldOrientation::Vertical, form_error.is_some())
                                    .child(field_label("One-Time Password"))
                                    .child(input_otp(&form).child(input_otp_group(
                                        (0..6).map(|i| input_otp_slot(&form, i, form_error.is_some())),
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
