//! `StepperForm.vue`: a vee-validate form in three steps, each step's own
//! schema checked before the stepper moves on.

use gui::prelude::*;
use gui::{Div, InputType, ScenarioError, TextField, div};
use shadcn::{
    ButtonSize, ButtonVariant, FieldOrientation, FormEntry, FormField, SelectSize, SelectState, StepState,
    StepperState, Toaster, Unset, button, field, field_error, field_group, field_label, form_control, form_submit,
    input, select_content, select_group, select_item, select_trigger, stepper_description, stepper_item, stepper_title,
    theme, toaster,
};

use super::form_cards::{Json, Library, Pre, failing, is_email, submitted};
use super::stepper::{line, round_trigger};
use crate::{Render, Shot, demo, page};

const STEPS: [(usize, &str, &str); 3] = [
    (1, "Your details", "Provide your name and email"),
    (2, "Your password", "Choose a password"),
    (3, "Your Favorite Drink", "Choose a drink"),
];

/// zod's message for a missing value.
const REQUIRED: &str = "Required";

type Text = FormField<Unset<TextField>>;

/// A text field with no initial value, checked by `rule` once there is text.
fn text_field(rule: impl Fn(&str) -> Vec<String> + 'static) -> Text {
    FormField::new(
        Unset::new(TextField::new("")),
        Library::Vee.checks(),
        move |v: &Option<String>| v.as_deref().map_or_else(|| vec![REQUIRED.to_owned()], &rule),
    )
}

/// The fields of the three steps; their values stay as the steps change
/// (`keep-values`).
#[derive(Clone)]
struct Fields {
    full_name: Text,
    email: Text,
    password: Text,
    confirm: Text,
    drink_select: SelectState,
    drink: FormField<Unset<gui::Signal<Option<String>>>>,
}

impl Fields {
    fn new() -> Self {
        let password = text_field(|v| {
            let n = v.chars().count();
            failing(&[
                (n >= 2, "String must contain at least 2 character(s)"),
                (n <= 50, "String must contain at most 50 character(s)"),
            ])
        });
        let typed = password.control().control().clone();
        let drink_select = SelectState::new(None);
        Self {
            full_name: text_field(|_| Vec::new()),
            email: text_field(|v| failing(&[(is_email(v), "Invalid email")])),
            // The object's refine names the confirmation, once both are given.
            confirm: text_field(move |v| failing(&[(v == typed.text(), "Passwords must match!")])),
            password,
            drink: FormField::new(
                Unset::new(drink_select.value.clone()),
                Library::Vee.checks(),
                |v: &Option<Option<String>>| match v.as_ref().and_then(Option::as_deref) {
                    Some("coffee" | "tea" | "soda") => Vec::new(),
                    Some(_) => vec!["Invalid input".to_owned()],
                    None => vec![REQUIRED.to_owned()],
                },
            ),
            drink_select,
        }
    }

    /// The fields step `step`'s schema checks.
    fn of(&self, step: usize) -> Vec<&dyn FormEntry> {
        match step {
            1 => vec![&self.full_name, &self.email],
            2 => vec![&self.password, &self.confirm],
            _ => vec![&self.drink],
        }
    }

    /// Whether step `step` passes its schema now (vee-validate's `meta.valid`).
    fn valid(&self, step: usize) -> bool {
        match step {
            1 => self.full_name.valid() && self.email.valid(),
            2 => self.password.valid() && self.confirm.valid(),
            _ => self.drink.valid(),
        }
    }

    fn json(&self) -> Json {
        let text = |f: &Text| Json::Str(f.control().control().text());
        Json::Object(vec![
            ("fullName", text(&self.full_name)),
            ("email", text(&self.email)),
            ("password", text(&self.password)),
            ("confirmPassword", text(&self.confirm)),
            (
                "favoriteDrink",
                Json::Str(self.drink_select.value.get().unwrap_or_default()),
            ),
        ])
    }
}

/// The step's round button ([`round_trigger`]); a step ahead is disabled
/// while the current one fails its schema.
fn trigger(stepper: &StepperState, step: usize, current_valid: bool) -> Div {
    let ahead = step > stepper.step.get();
    round_trigger(stepper, step).when(
        stepper.state_of(step) != StepState::Completed && ahead && !current_valid,
        |b| b.disabled(true),
    )
}

/// A labelled input holding `kind` (its `type`), with its errors.
fn text_input(name: &str, f: &Text, kind: InputType) -> gui::Element {
    let invalid = f.invalid();
    field(FieldOrientation::Vertical, invalid)
        .child(field_label(name.to_owned()))
        .child(form_control(input(f.control().control()).input_type(kind), invalid))
        .child(field_error(f.errors()))
        .into_element()
}

pub fn render() -> Render {
    let toasts = Toaster::new();
    let stepper = StepperState::new(1);
    let fields = Fields::new();
    Box::new(move || {
        let t = theme();
        let step = stepper.step.get();
        let valid = fields.valid(step);
        let last = STEPS.len();
        let header = div()
            .flex()
            .w_full()
            .gap(2.0)
            .children(STEPS.map(|(n, title, description)| {
                let active = stepper.state_of(n) == StepState::Active;
                stepper_item()
                    .relative()
                    .flex()
                    .w_full()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .when(n != last, |d| d.child(line(&stepper, n)))
                    .child(trigger(&stepper, n, valid))
                    .child(
                        div()
                            .mt(5.0)
                            .flex_col()
                            .items_center()
                            .text_center()
                            // `text-sm lg:text-base`: the reference renders at desktop width.
                            .child(
                                stepper_title(title)
                                    .text_base()
                                    .font_semibold()
                                    .when(active, |d| d.text_color(t.primary)),
                            )
                            // `text-xs lg:text-sm`; its `text-primary` loses to
                            // `text-muted-foreground` in the stylesheet's order.
                            .child(stepper_description(description).text_sm()),
                    )
            }));
        let body = match step {
            1 => field_group()
                .mt(4.0)
                .child(text_input("Full Name", &fields.full_name, InputType::Text))
                .child(text_input("Email", &fields.email, InputType::Text)),
            2 => field_group()
                .mt(4.0)
                .child(text_input("Password", &fields.password, InputType::Password))
                .child(text_input("Confirm Password", &fields.confirm, InputType::Password)),
            _ => {
                let s = &fields.drink_select;
                let invalid = fields.drink.invalid();
                let drinks = select_content(s).child(
                    select_group()
                        .child(select_item(s, "coffee", "Coffee", false))
                        .child(select_item(s, "tea", "Tea", false))
                        .child(select_item(s, "soda", "Soda", false)),
                );
                field_group().mt(4.0).child(
                    field(FieldOrientation::Vertical, invalid)
                        .child(field_label("Drink"))
                        .child(form_control(
                            select_trigger(s, "Select a drink", SelectSize::Default, drinks).w_full(),
                            invalid,
                        ))
                        .child(field_error(fields.drink.errors())),
                )
            }
        };
        let back = {
            let stepper = stepper.clone();
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .child("Back")
                .disabled(step <= 1)
                .on_click(move |_| stepper.go(stepper.step.get().saturating_sub(1)))
        };
        // Next moves on when the step passes, else submits to show why not;
        // Submit on the last step sends the whole form.
        let forward = {
            let (stepper, fields, toasts) = (stepper.clone(), fields.clone(), toasts.clone());
            let label = if step == last { "Submit" } else { "Next" };
            button(ButtonVariant::Default, ButtonSize::Sm)
                .child(label)
                .on_click(move |_| {
                    let step = stepper.step.get();
                    let passes = form_submit(&fields.of(step));
                    if step == last && passes {
                        submitted(&toasts, &fields.json(), Pre::Neutral950);
                    } else if passes {
                        stepper.go(step.saturating_add(1));
                    }
                })
        };
        page(
            "Form (stepper)",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "StepperForm.vue",
                    div().w_full().flex_col().child(header).child(body).child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mt(4.0)
                            .child(back)
                            .child(div().flex().items_center().gap(3.0).child(forward)),
                    ),
                ))
                .child(toaster(&toasts)),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.click_text("Next")?;
    s.frame("next on empty step: required")?;
    let (x, y) = s.locate("Full Name")?;
    s.click(x, y + 28.0);
    s.type_text("Ada Lovelace");
    let (x, y) = s.locate("Email")?;
    s.click(x, y + 28.0);
    s.type_text("ada@example.com");
    s.frame("step 1 filled")?;
    s.click_text("Next")?;
    s.frame("step 2")?;
    let (x, y) = s.locate("Password")?;
    s.click(x, y + 28.0);
    s.type_text("secret");
    let (x, y) = s.locate("Confirm Password")?;
    s.click(x, y + 28.0);
    s.type_text("secreT");
    s.frame("passwords differ")?;
    s.press(gui::NamedKey::Backspace, gui::Modifiers::default());
    s.type_text("t");
    // The error under the confirmation goes, and Next moves up with it.
    s.settle()?;
    s.click_text("Next")?;
    s.frame("step 3")?;
    s.click_text("Submit")?;
    s.frame("submit without a drink")?;
    s.click_text("Select a drink")?;
    s.frame("drinks open")?;
    s.click_text("Tea")?;
    s.frame("tea chosen")?;
    s.click_text("Submit")?;
    s.frame("submitted")?;
    Ok(())
}
