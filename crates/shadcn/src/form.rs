//! Form (`ui/form`): the parts shadcn lays a validated field out with: an
//! item holding a label, the control, a description and the message; and
//! the state that decides when a field's errors show.
//!
//! shadcn's demos validate with a form library (vee-validate, TanStack Form,
//! Formisch) and a schema. Here a [`FormField`] holds one control's state,
//! its rule (the schema's checks for that field, every failing message) and
//! the errors last found; [`Checks`] says which events check it, which is all
//! those libraries differ in. [`form_submit`] checks every field as a submit
//! does.
//!
//! A field holds its value where its control is bound to it (a
//! [`TextField`] an input edits, a `Signal` a checkbox or select is bound to
//! with `.model(…)`), so a reset writes the one value the control shows.

use std::cell::Cell;
use std::rc::Rc;

use gui::element::Node;
use gui::prelude::*;
use gui::{Div, Fluent, FocusHandle, Signal, Styled, TextField, div, text, watch};

use crate::focus::StateLook;
use crate::focus::destructive_ring;
use crate::label::label;
use crate::theme::theme;

/// What a change of a field's value does to its errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnChange {
    /// They stay until the next check.
    Keep,
    /// They go until the next check (TanStack Form's submit errors).
    Clear,
    /// The field is checked again once it has been checked (Formisch's
    /// `revalidate: 'input'`).
    Recheck,
    /// The field is checked (vee-validate's `validateOnModelUpdate`).
    Check,
}

/// When a field is checked besides a submit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checks {
    /// When its control loses focus.
    pub on_blur: bool,
    pub on_change: OnChange,
}

/// A control's state a [`FormField`] reads, watches and resets: a `Copy`
/// handle the control is bound to.
pub trait FormValue: Copy + 'static {
    type Value: Clone + PartialEq + 'static;
    /// The value now; reading it subscribes the reader.
    fn value(&self) -> Self::Value;
    fn set_value(&self, value: Self::Value);
    /// The focus it blurs from, for [`Checks::on_blur`]; none for a control
    /// that is not one focus stop.
    fn focus(&self) -> Option<FocusHandle> {
        None
    }
}

impl FormValue for TextField {
    type Value = String;
    fn value(&self) -> String {
        self.edit.with(|e| e.text.clone())
    }
    fn set_value(&self, value: String) {
        self.set_text(value);
    }
    fn focus(&self) -> Option<FocusHandle> {
        Some(self.focus)
    }
}

/// A value held in a signal: a checkbox's or switch's `bool`, a select's or
/// radio group's `Option<String>`, a number field's `f64`.
impl<T: Clone + PartialEq + 'static> FormValue for Signal<T> {
    type Value = T;
    fn value(&self) -> T {
        self.get()
    }
    fn set_value(&self, value: T) {
        self.set(value);
    }
}

/// A control whose form gives it no initial value (`useForm` without
/// `initialValues`): its value is `None`, which zod reads as missing
/// ("Required"), until the user first changes it.
pub struct Unset<C: FormValue> {
    control: C,
    /// What the control shows while there is no value.
    blank: Signal<C::Value>,
    entered: Signal<bool>,
}

impl<C: FormValue> Clone for Unset<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: FormValue> Copy for Unset<C> {}

impl<C: FormValue> Unset<C> {
    /// `control` unset, until its value first changes; made in the
    /// caller's setup, whose scope its watcher lives in.
    #[must_use]
    pub fn new(control: C) -> Self {
        let blank = control.value();
        let entered = Signal::new(false);
        let left = blank.clone();
        watch(
            move || control.value(),
            move |value, _| {
                if *value != left {
                    entered.set(true);
                }
            },
        );
        Self {
            control,
            blank: Signal::new(blank),
            entered,
        }
    }

    /// The control's own state, to bind the control to.
    #[must_use]
    pub fn control(&self) -> C {
        self.control
    }
}

impl<C: FormValue> FormValue for Unset<C> {
    type Value = Option<C::Value>;

    fn value(&self) -> Option<C::Value> {
        let value = self.control.value();
        self.entered.get().then_some(value)
    }

    fn set_value(&self, value: Option<C::Value>) {
        let shown = value.clone().unwrap_or_else(|| self.blank.get_untracked());
        self.control.set_value(shown);
        self.entered.set(value.is_some());
    }

    fn focus(&self) -> Option<FocusHandle> {
        self.control.focus()
    }
}

/// A field's check: every message its value fails, none when it is valid.
type Rule<V> = Rc<dyn Fn(&V) -> Vec<String>>;

/// One field of a form: its control's state, the rule it is checked by and
/// the errors the last check found (vee-validate's `useField`): a `Copy`
/// handle, made in the caller's setup, whose scope its watcher lives in.
/// It watches its control for the events its [`Checks`] name.
pub struct FormField<C: FormValue> {
    control: C,
    initial: Signal<C::Value>,
    rule: Signal<Rule<C::Value>>,
    errors: Signal<Vec<String>>,
    /// Whether it was checked since the form was made or reset.
    checked: Signal<bool>,
    /// The value the watcher last saw, so a reset is no change.
    seen: Signal<C::Value>,
}

impl<C: FormValue> Clone for FormField<C> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<C: FormValue> Copy for FormField<C> {}

impl<C: FormValue> FormField<C> {
    /// A field of `control`, checked by `rule` (every message that fails,
    /// none when the value is valid) on a submit and on `checks`.
    #[must_use]
    pub fn new(control: C, checks: Checks, rule: impl Fn(&C::Value) -> Vec<String> + 'static) -> Self {
        let initial = control.value();
        let rule: Rule<C::Value> = Rc::new(rule);
        let field = Self {
            control,
            initial: Signal::new(initial.clone()),
            rule: Signal::new(rule),
            errors: Signal::labeled("form field errors", Vec::new()),
            checked: Signal::new(false),
            seen: Signal::new(initial),
        };
        watch(
            move || control.value(),
            move |value, _| {
                if field.seen.with_untracked(|seen| seen == value) {
                    return;
                }
                field.seen.set(value.clone());
                match checks.on_change {
                    OnChange::Keep => {}
                    OnChange::Clear => field.errors.set(Vec::new()),
                    OnChange::Recheck => {
                        if field.checked.get_untracked() {
                            field.check_value(value);
                        }
                    }
                    OnChange::Check => {
                        field.check_value(value);
                    }
                }
            },
        );
        if checks.on_blur
            && let Some(focus) = control.focus()
        {
            let was = Cell::new(false);
            watch(
                move || focus.is_focused(),
                move |focused, _| {
                    if was.replace(*focused) && !*focused {
                        field.check();
                    }
                },
            );
        }
        field
    }

    fn check_value(&self, value: &C::Value) -> bool {
        let found = self.rule.with_untracked(|rule| rule(value));
        let valid = found.is_empty();
        self.checked.set(true);
        self.errors.set(found);
        valid
    }

    /// The control's state, to bind the control to.
    #[must_use]
    pub fn control(&self) -> C {
        self.control
    }

    /// The errors the last check found (reactive).
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        self.errors.get()
    }

    /// Whether the value passes its rule now, checked or not (reactive):
    /// vee-validate's `meta.valid`, which lets a form move on.
    #[must_use]
    pub fn valid(&self) -> bool {
        let value = self.control.value();
        self.rule.with_untracked(|rule| rule(&value)).is_empty()
    }

    /// Whether the last check found an error (reactive): the field's
    /// `data-invalid` and its control's `aria-invalid`.
    #[must_use]
    pub fn invalid(&self) -> bool {
        self.errors.with(|e| !e.is_empty())
    }
}

/// What a form does to each of its fields, whatever their value.
pub trait FormEntry {
    /// Check the value now and hold what fails; true when nothing does.
    fn check(&self) -> bool;
    /// Put the initial value back and forget the errors.
    fn reset(&self);
}

impl<C: FormValue> FormEntry for FormField<C> {
    fn check(&self) -> bool {
        let value = gui::untracked(|| self.control.value());
        self.check_value(&value)
    }

    fn reset(&self) {
        let initial = self.initial.get_untracked();
        self.seen.set(initial.clone());
        self.control.set_value(initial);
        self.checked.set(false);
        self.errors.set(Vec::new());
    }
}

/// Check every field, as a submit does; true when none has an error.
pub fn form_submit(fields: &[&dyn FormEntry]) -> bool {
    // Every field is checked, so each shows what it fails, not only the first.
    fields.iter().map(|f| f.check()).filter(|valid| !valid).count() == 0
}

/// Reset every field, as a form's reset does.
pub fn form_reset(fields: &[&dyn FormEntry]) {
    for f in fields {
        f.reset();
    }
}

/// One field of a form: its parts stacked.
#[must_use]
pub fn form_item() -> Div {
    div().named("form-item").display(Display::Grid).gap(8.0)
}

/// The field's name, destructive while it is `invalid`.
#[must_use]
pub fn form_label(name: impl Into<String>, invalid: bool) -> Div {
    let t = theme();
    label(name)
        .named("form-label")
        .when(invalid, |d| d.text_color(t.destructive))
}

/// A control a form marks `invalid` (`FormControl`'s `aria-invalid`).
pub trait FormControl: Sized {
    /// Marked as `aria-invalid` marks it: a destructive border, and a
    /// destructive ring while focused.
    #[must_use]
    fn invalid(self, invalid: bool) -> Self;
}

impl<K> FormControl for Node<K> {
    fn invalid(self, invalid: bool) -> Self {
        let t = theme();
        self.when(invalid, |c| {
            c.border_color(t.destructive).with_interaction(|c, i| {
                c.ring_while(
                    move || i.focus_visible(),
                    &destructive_ring(gui::Style::default(), &t).border_color(t.destructive),
                )
            })
        })
    }
}

impl FormControl for crate::input::Input {
    fn invalid(self, invalid: bool) -> Self {
        self.when(invalid, |c| c.map(move |el| el.invalid(true)))
    }
}

impl FormControl for crate::checkbox::Checkbox {
    fn invalid(self, invalid: bool) -> Self {
        if invalid {
            self.class(|el| el.invalid(true))
        } else {
            self
        }
    }
}

/// `control` (an input, a textarea, a select trigger, a checkbox) marked
/// `invalid` as `aria-invalid` marks it.
#[must_use]
pub fn form_control<C: FormControl>(control: C, invalid: bool) -> C {
    control.invalid(invalid)
}

/// Help text under the control.
#[must_use]
pub fn form_description(description: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("form-description")
        .text_color(t.muted_foreground)
        .text_size(14.0)
        .line_height(20.0)
        .child(text(description))
}

/// The field's error, when it has one.
#[must_use]
pub fn form_message(error: Option<&str>) -> Option<Div> {
    let t = theme();
    error.map(|e| {
        div()
            .named("form-message")
            .text_color(t.destructive)
            .text_size(14.0)
            .line_height(20.0)
            .child(text(e))
    })
}
