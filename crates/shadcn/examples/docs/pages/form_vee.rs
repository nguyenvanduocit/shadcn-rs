//! vee-validate's form demos (`VeeValidate*Demo.vue`): zod schemas checked
//! on submit, on blur and on every change; and its password demo, whose
//! meter follows the requirements as they are met.

use gui::prelude::*;
use gui::{Color, Div, Icon, InputType, ScenarioError, TextField, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, FieldOrientation, FormField, InputGroupAlign, button, card, card_content,
    card_description, card_footer, card_header, card_title, field, field_error, field_group, field_label, form_reset,
    form_submit, input_group, input_group_addon, input_group_input, progress, theme, toaster,
};

use super::form_cards::{self, Cards, Files, Json, Library, Pre, failing, submitted};
use crate::{Render, Shot, demo, page};

const FILES: Files = Files {
    demo: "VeeValidateDemo.vue",
    input: "VeeValidateInputDemo.vue",
    textarea: "VeeValidateTextareaDemo.vue",
    select: "VeeValidateSelectDemo.vue",
    switch: "VeeValidateSwitchDemo.vue",
    radio_group: "VeeValidateRadioGroupDemo.vue",
    checkbox: "VeeValidateCheckboxDemo.vue",
    array: "VeeValidateArrayDemo.vue",
    complex: "VeeValidateComplexDemo.vue",
};

/// Tailwind's `green-500`, the colour of a met requirement.
fn green_500() -> Color {
    Color::hex(0x00_c9_50)
}

/// What a strong password has: the meter's steps and the schema's checks.
/// A requirement: what the meter lists, what the schema says when it
/// fails, and its test.
type Requirement = (&'static str, &'static str, fn(&str) -> bool);

const REQUIREMENTS: [Requirement; 5] = [
    ("At least 8 characters", "Password must be at least 8 characters", |v| {
        v.chars().count() >= 8
    }),
    (
        "One lowercase letter",
        "Password must contain at least one lowercase letter",
        |v| v.chars().any(|c| c.is_ascii_lowercase()),
    ),
    (
        "One uppercase letter",
        "Password must contain at least one uppercase letter",
        |v| v.chars().any(|c| c.is_ascii_uppercase()),
    ),
    ("One number", "Password must contain at least one number", |v| {
        v.chars().any(|c| c.is_ascii_digit())
    }),
    (
        "One special character",
        "Password must contain at least one special character",
        |v| v.chars().any(|c| "!@#$%^&*(),.?\":{}|<>".contains(c)),
    ),
];

/// The meter's track colour for `met` of the five requirements.
fn strength_color(met: usize) -> Color {
    match met {
        0 => Color::hex(0xe5_e5_e5),     // neutral-200
        1 | 2 => Color::hex(0xfb_2c_36), // red-500
        3 | 4 => Color::hex(0xed_b2_00), // yellow-500
        _ => green_500(),
    }
}

struct PasswordCard {
    password: FormField<TextField>,
}

impl PasswordCard {
    fn new() -> Self {
        Self {
            password: FormField::new(TextField::new(""), Library::Vee.checks(), |v: &String| {
                let checks: Vec<(bool, &str)> = REQUIREMENTS
                    .iter()
                    .map(|(_, message, test)| (test(v), *message))
                    .collect();
                failing(&checks)
            }),
        }
    }

    fn view(&self) -> Div {
        let t = theme();
        let f = &self.password;
        let invalid = f.invalid();
        let value = f.control().text();
        let met = REQUIREMENTS.iter().filter(|(_, _, test)| test(&value)).count();
        let all_met = met == REQUIREMENTS.len();
        let percent = 100.0 * f32::from(u8::try_from(met).unwrap_or(u8::MAX)) / 5.0;
        let rows = REQUIREMENTS.iter().map(|(label, _, test)| {
            let ok = test(&value);
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .gap(8.0)
                .text_size(14.0)
                .line_height(20.0)
                .child(
                    icon(Icon::Check)
                        .size(16.0)
                        .text_color(if ok { green_500() } else { t.muted_foreground }),
                )
                .child(text((*label).to_owned()).text_color(if ok { t.foreground } else { t.muted_foreground }))
        });
        let group = input_group(
            input_group_input(&f.control())
                .input_type(InputType::Password)
                .placeholder("Enter your password")
                .invalid(invalid),
        )
        .addon(
            input_group_addon(InputGroupAlign::InlineEnd).child(icon(Icon::Check).text_color(if all_met {
                green_500()
            } else {
                t.muted_foreground
            })),
        );
        let (submit, reset) = (*f, *f);
        card()
            .width(pct(100.0))
            .max_width(448.0)
            .child(
                card_header()
                    .border_bottom(1.0)
                    .border_color(t.border)
                    .padding_bottom(24.0)
                    .child(card_title("Create Password"))
                    .child(card_description("Choose a strong password to secure your account.")),
            )
            .child(
                card_content().child(
                    field_group().child(
                        field(FieldOrientation::Vertical, invalid)
                            .child(field_label("Password"))
                            .child(group)
                            .child(
                                div()
                                    .display(Display::Flex)
                                    .flex_direction(FlexDirection::Column)
                                    .gap(8.0)
                                    .child(progress(percent).background(strength_color(met)))
                                    .child(
                                        div()
                                            .display(Display::Flex)
                                            .flex_direction(FlexDirection::Column)
                                            .gap(6.0)
                                            .children(rows),
                                    ),
                            )
                            .child(field_error(f.errors())),
                    ),
                ),
            )
            .child(
                card_footer()
                    .border_top(1.0)
                    .border_color(t.border)
                    .padding_top(24.0)
                    .child(
                        field(FieldOrientation::Vertical, false)
                            .child(
                                button(ButtonVariant::Default, ButtonSize::Default)
                                    .child("Create Password")
                                    .on_click(move |_| {
                                        if form_submit(&[&submit]) {
                                            submitted(
                                                &Json::Object(vec![("password", Json::Str(submit.control().text()))]),
                                                Pre::Code,
                                            );
                                        }
                                    }),
                            )
                            .child(
                                button(ButtonVariant::Outline, ButtonSize::Default)
                                    .child("Reset")
                                    .on_click(move |_| form_reset(&[&reset])),
                            ),
                    ),
            )
    }
}

pub fn render() -> Render {
    let cards = Cards::new(Library::Vee);
    let password = PasswordCard::new();
    Box::new(move || {
        page(
            "Form (vee-validate)",
            cards
                .view(&FILES)
                .child(demo("VeeValidatePasswordDemo.vue", password.view()))
                .child(toaster()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    form_cards::script(s)
}
