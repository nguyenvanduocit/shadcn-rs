//! vee-validate forms laid out with shadcn's `Form*` parts and with `Field`
//! around single controls (`FormDemo`, `InputForm`, `InputFormAutoAnimate`,
//! `NativeSelectFormDemo`, `NumberFieldFormDemo`).

use gui::prelude::*;
use gui::{ScenarioError, Signal, TextField, div};
use shadcn::{
    Button, ButtonSize, ButtonVariant, FieldOrientation, FormField, NumberFormat, Unset, button, field,
    field_description, field_error, field_group, field_label, form_control, form_description, form_item, form_label,
    form_message, form_submit, input, native_select, native_select_option, number_field, number_field_content,
    number_field_decrement, number_field_increment, number_field_input, textarea, toaster,
};

use super::form_cards::{Json, Library, Pre, failing, is_email, submitted};
use crate::{Render, Shot, demo, page};

/// zod's message for a missing value.
const REQUIRED: &str = "Required";

/// A text field of a vee-validate form with no initial values, checked by
/// `rule` once there is text.
fn text_field(rule: fn(&str) -> Vec<String>) -> FormField<Unset<TextField>> {
    FormField::new(
        Unset::new(TextField::new("")),
        Library::Vee.checks(),
        move |v: &Option<String>| v.as_deref().map_or_else(|| vec![REQUIRED.to_owned()], rule),
    )
}

fn username(v: &str) -> Vec<String> {
    failing(&[(v.chars().count() >= 2, "Username must be at least 2 characters.")])
}

fn email(v: &str) -> Vec<String> {
    failing(&[(is_email(v), "Please enter a valid email address.")])
}

fn bio(v: &str) -> Vec<String> {
    let n = v.chars().count();
    failing(&[
        (n >= 10, "Bio must be at least 10 characters."),
        (n <= 160, "Bio must not be longer than 160 characters."),
    ])
}

/// A `FormItem` of one text field: its label, control, description and
/// first error; `area` makes the control a textarea.
fn text_item(
    name: &str,
    f: &FormField<Unset<TextField>>,
    placeholder: &str,
    description: &str,
    area: bool,
) -> gui::Div {
    let errors = f.errors();
    let invalid = !errors.is_empty();
    let control = f.control().control();
    let control = if area {
        textarea().field(control).placeholder(placeholder.to_owned())
    } else {
        input().field(control).placeholder(placeholder.to_owned())
    };
    form_item()
        .child(form_label(name.to_owned(), invalid))
        .child(form_control(control, invalid))
        .child(form_description(description.to_owned()))
        .child(form_message(errors.first().map(String::as_str)))
}

/// The form's submit button; in a `space-y-*` form it keeps its own width.
fn submit_button(on_submit: impl Fn() + 'static) -> Button {
    button(ButtonVariant::Default, ButtonSize::Default)
        .align_self(Align::Start)
        .child("Submit")
        .on_click(move |_| on_submit())
}

pub fn render() -> Render {
    // FormDemo.vue
    let (demo_user, demo_email, demo_bio) = (text_field(username), text_field(email), text_field(bio));
    // InputForm.vue
    let input_user = text_field(username);
    // InputFormAutoAnimate.vue
    let (animate_user, animate_email) = (text_field(username), text_field(email));
    // NativeSelectFormDemo.vue
    let country_select = Signal::labeled("native select value", String::new());
    let country = FormField::new(
        Unset::new(country_select),
        Library::Vee.checks(),
        |v: &Option<String>| {
            v.as_deref().map_or_else(
                || vec![REQUIRED.to_owned()],
                |c| failing(&[(c.chars().count() >= 2, "Select a country.")]),
            )
        },
    );
    let countries = [
        native_select_option("", "Select a country"),
        native_select_option("us", "United States"),
        native_select_option("uk", "United Kingdom"),
        native_select_option("ca", "Canada"),
    ];
    // NumberFieldFormDemo.vue
    let payment_value = Signal::labeled("payment", 10.0);
    let payment = FormField::new(payment_value, Library::Vee.checks(), |v: &f64| {
        failing(&[
            (*v >= 10.0, "Min 10 euros to send payment"),
            (*v <= 5000.0, "Max 5000 euros to send payment"),
        ])
    });
    Box::new(move || {
        let form_demo = {
            let (u, e, b) = (demo_user, demo_email, demo_bio);
            div()
                .width(pct(100.0))
                .max_width(384.0)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(text_item(
                    "Username",
                    &demo_user,
                    "shadcn",
                    "This is your public display name.",
                    false,
                ))
                .child(text_item(
                    "Email",
                    &demo_email,
                    "your@email.com",
                    "We'll never share your email with anyone else.",
                    false,
                ))
                .child(text_item(
                    "Bio",
                    &demo_bio,
                    "Tell us a little bit about yourself",
                    "You can @mention other users and organizations.",
                    true,
                ))
                .child(
                    submit_button(move || {
                        form_submit(&[&u, &e, &b]);
                    })
                    .width(pct(100.0)),
                )
        };
        let input_form = {
            let u = input_user;
            div()
                .width(Length::Fraction(2.0 / 3.0))
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(text_item(
                    "Username",
                    &input_user,
                    "shadcn",
                    "This is your public display name.",
                    false,
                ))
                .child(submit_button(move || {
                    form_submit(&[&u]);
                }))
        };
        let animate_form = {
            let (u, e) = (animate_user, animate_email);
            div()
                .width(Length::Fraction(2.0 / 3.0))
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(text_item(
                    "Username",
                    &animate_user,
                    "shadcn",
                    "This is your public display name.",
                    false,
                ))
                .child(text_item(
                    "Email",
                    &animate_email,
                    "your@email.com",
                    "We'll never share your email with anyone else.",
                    false,
                ))
                .child(submit_button(move || {
                    form_submit(&[&u, &e]);
                }))
        };
        let native_select_form = {
            let invalid = country.invalid();
            let c = country;
            div().width(pct(100.0)).max_width(384.0).child(
                field_group()
                    .child(
                        field(FieldOrientation::Vertical, invalid)
                            .child(field_label("Country"))
                            .child(native_select(countries.clone()).model(country_select).invalid(invalid))
                            .child(field_description("Select a country"))
                            .child(field_error(country.errors())),
                    )
                    .child(
                        field(FieldOrientation::Vertical, false).child(
                            submit_button(move || {
                                if form_submit(&[&c]) {
                                    let value = c.control().control().get();
                                    submitted(&Json::Object(vec![("country", Json::Str(value))]), Pre::Neutral900);
                                }
                            })
                            .width(pct(100.0)),
                        ),
                    ),
            )
        };
        let number_form = {
            let invalid = payment.invalid();
            let p = payment;
            div()
                .width(Length::Fraction(2.0 / 3.0))
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(
                    field(FieldOrientation::Vertical, invalid)
                        .width(300.0)
                        .child(field_label("Payment"))
                        .child(
                            number_field()
                                .model(payment_value)
                                .min(0.0)
                                .format(NumberFormat::currency("EUR", true))
                                .gap(8.0)
                                .child(
                                    number_field_content()
                                        .child(number_field_decrement())
                                        .child(number_field_input())
                                        .child(number_field_increment()),
                                ),
                        )
                        .child(field_description("Enter value between 10 and 5000."))
                        .child(field_error(payment.errors())),
                )
                .child(submit_button(move || {
                    if form_submit(&[&p]) {
                        submitted(
                            &Json::Object(vec![("payment", Json::Num(p.control().get()))]),
                            Pre::Neutral950,
                        );
                    }
                }))
        };
        page(
            "Form",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("FormDemo.vue", form_demo))
                .child(demo("InputForm.vue", input_form))
                .child(demo("InputFormAutoAnimate.vue", animate_form))
                .child(demo("NativeSelectFormDemo.vue", native_select_form))
                .child(demo("NumberFieldFormDemo.vue", number_form))
                .child(toaster()),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Submit")?;
    s.frame("submitted untouched: required")?;
    let (x, y) = s.locate("your@email.com")?;
    s.click(x, y);
    s.type_text("not-an-email");
    s.frame("email typed invalid")?;
    let (x, y) = s.locate("shadcn")?;
    s.click(x, y);
    s.type_text("r");
    s.frame("username short, email blurred")?;
    s.type_text("emit");
    s.frame("username fixed")?;
    Ok(())
}
