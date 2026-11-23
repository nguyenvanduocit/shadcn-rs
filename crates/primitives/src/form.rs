//! The form model: fields, their checks, what a form submits, and reset
//! (reka's form integration — `VisuallyHiddenInput`, `useFormControl`,
//! `BubbleSelect` — and the field state of the libraries shadcn's demos
//! validate with; research §6 "Tích hợp form", §4 item 7).
//!
//! There is no native form under a `gui` window, so the guarantees reka
//! gets from hidden `<input>` twins are built as types:
//!
//! - A [`FormField`] holds one control's state ([`FormValue`]), its rule
//!   (every message its value fails) and the errors the last check found;
//!   [`Checks`] says which events check it besides a submit.
//! - A form is the fields its caller lists ([`form_submit`],
//!   [`form_reset`], [`form_data`] take them as a slice).
//! - [`form_submit`] checks every field, so each shows what it fails, not
//!   only the first.
//! - [`form_data`] is what a native form would send: each named field's
//!   value encoded as reka's twins encode it ([`FormData::pairs`]); a
//!   disabled field sends nothing.
//! - **Reset** puts every field back to the value it started at, whoever
//!   keeps the value (the control, or the caller through its model), and
//!   forgets its errors. reka resets only touched, dirty and error state and
//!   leaves custom controls showing their values, so control and form
//!   disagree until the next change (research §6); here they never do.

use std::cell::Cell;
use std::rc::Rc;

use gui::value::{IntoValue, Value};
use gui::{FocusHandle, Signal, TextField, watch};

use crate::availability::Availability;

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
        self.text()
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
/// `initialValues`): its value is `None`, which a schema reads as missing,
/// until the user first changes it.
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
    #[track_caller]
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

/// A value as a native form would send it (reka's hidden input twins).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormData {
    /// One value: `name=value`; empty sends `name=`.
    Text(String),
    /// Several: `name[0]=…`, `name[1]=…`; none sends nothing.
    List(Vec<String>),
    /// A checkbox, switch or toggle: `name=value` while on (`on` unless the
    /// control sets its own), nothing while off.
    Toggle(Option<String>),
}

impl FormData {
    /// The pairs it sends under `name`.
    #[must_use]
    pub fn pairs(&self, name: &str) -> Vec<(String, String)> {
        match self {
            Self::Text(v) => vec![(name.to_owned(), v.clone())],
            Self::List(vs) => vs
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("{name}[{i}]"), v.clone()))
                .collect(),
            Self::Toggle(Some(v)) => vec![(name.to_owned(), v.clone())],
            Self::Toggle(None) => Vec::new(),
        }
    }
}

/// A value a form can send.
pub trait Submit {
    fn form_data(&self) -> FormData;
}

impl Submit for String {
    fn form_data(&self) -> FormData {
        FormData::Text(self.clone())
    }
}

impl Submit for bool {
    fn form_data(&self) -> FormData {
        FormData::Toggle(self.then(|| "on".to_owned()))
    }
}

impl Submit for f64 {
    fn form_data(&self) -> FormData {
        FormData::Text(if self.is_nan() { String::new() } else { self.to_string() })
    }
}

impl Submit for Vec<String> {
    fn form_data(&self) -> FormData {
        FormData::List(self.clone())
    }
}

/// No value sends an empty one (`name=`), as reka's twin does for `null`.
impl<T: Submit> Submit for Option<T> {
    fn form_data(&self) -> FormData {
        match self {
            Some(v) => v.form_data(),
            None => FormData::Text(String::new()),
        }
    }
}

/// A field's check: every message its value fails, none when it is valid.
type Rule<V> = Rc<dyn Fn(&V) -> Vec<String>>;

/// How a named field's value is sent.
type Encode<V> = Rc<dyn Fn(&V) -> FormData>;

/// A field's name and how its value is sent under it.
type Named<V> = Option<(Rc<str>, Encode<V>)>;

/// One field of a form: its control's state, the rule it is checked by and
/// the errors the last check found (vee-validate's `useField`): a `Copy`
/// handle, made in the caller's setup, whose scope its watchers live in.
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
    /// Its name and encoding, when the form sends it.
    name: Signal<Named<C::Value>>,
    /// Its control's availability: the control's own value or closure, so
    /// the two cannot disagree.
    availability: Signal<Value<Availability>>,
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
    #[track_caller]
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
            name: Signal::new(None),
            availability: Signal::new(Value::Fixed(Availability::Enabled)),
        };
        watch(
            move || control.value(),
            move |value, _| {
                // A disabled field is never checked (vee-validate skips
                // disabled fields): the change waits in `seen` for the field
                // to be enabled again.
                if !field.availability_now().submits() || field.seen.with_untracked(|seen| seen == value) {
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
        // Enabled again, the field is checked as its policy says: a change
        // made while it was disabled counts as one now, and a field checked
        // before shows what its value fails now.
        watch(
            move || field.availability_tracked().submits(),
            move |&submits, _| {
                if !submits {
                    return;
                }
                let value = gui::untracked(|| control.value());
                let changed = !field.seen.with_untracked(|seen| *seen == value);
                if changed {
                    field.seen.set(value.clone());
                }
                let checked = field.checked.get_untracked();
                match checks.on_change {
                    OnChange::Keep => {}
                    OnChange::Clear => {
                        if changed {
                            field.errors.set(Vec::new());
                        }
                    }
                    OnChange::Recheck => {
                        if checked {
                            field.check_value(&value);
                        }
                    }
                    OnChange::Check => {
                        if checked || changed {
                            field.check_value(&value);
                        }
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

    /// This field sent under `name` (a native field's `name`).
    #[must_use]
    pub fn named(self, name: impl Into<Rc<str>>) -> Self
    where
        C::Value: Submit,
    {
        let encode: Encode<C::Value> = Rc::new(|v: &C::Value| v.form_data());
        self.name.update(|n| *n = Some((name.into(), encode)));
        self
    }

    /// This field sent under `name` as `encode` says (a select sending its
    /// fallback while nothing is chosen).
    #[must_use]
    pub fn named_by(self, name: impl Into<Rc<str>>, encode: impl Fn(&C::Value) -> FormData + 'static) -> Self {
        let encode: Encode<C::Value> = Rc::new(encode);
        self.name.update(|n| *n = Some((name.into(), encode)));
        self
    }

    /// This field's control can be used as `availability` says: the value
    /// or closure the control itself reads (as [`crate::label::Label::label`]
    /// takes it), read when the form checks or sends, so a control the look
    /// shows disabled is never checked or sent. A disabled field is neither.
    #[must_use]
    pub fn availability<M>(self, availability: impl IntoValue<Availability, M>) -> Self {
        let availability = availability.into_value();
        self.availability.update(|a| *a = availability);
        self
    }

    /// Its control's availability now; reading it subscribes the reader.
    fn availability_tracked(&self) -> Availability {
        self.availability.with(|a| match a {
            Value::Fixed(a) => *a,
            Value::Bound(f) => f(),
        })
    }

    /// Its control's availability now (untracked).
    fn availability_now(&self) -> Availability {
        gui::untracked(|| self.availability_tracked())
    }

    /// Whether the form reads this field now (reactive): a disabled one has
    /// neither errors nor invalidity.
    fn counted(&self) -> bool {
        self.availability_tracked().submits()
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

    /// The errors the last check found (reactive); none while the field is
    /// disabled.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        if !self.counted() {
            return Vec::new();
        }
        self.errors.get()
    }

    /// Whether the value passes its rule now, checked or not (reactive):
    /// vee-validate's `meta.valid`, which lets a form move on. A disabled
    /// field is valid, as it is never checked.
    #[must_use]
    pub fn valid(&self) -> bool {
        if !self.counted() {
            return true;
        }
        let value = self.control.value();
        self.rule.with_untracked(|rule| rule(&value)).is_empty()
    }

    /// Whether the last check found an error (reactive): the field's
    /// invalid state, its control's `aria-invalid`; never while the field
    /// is disabled.
    #[must_use]
    pub fn invalid(&self) -> bool {
        self.counted() && self.errors.with(|e| !e.is_empty())
    }
}

/// What a form does to each of its fields, whatever their value.
pub trait FormEntry {
    /// Check the value now and hold what fails; true when nothing does.
    fn check(&self) -> bool;
    /// Put the initial value back and forget the errors.
    fn reset(&self);
    /// Add what it sends to `out`.
    fn send(&self, out: &mut Vec<(String, String)>);
}

impl<C: FormValue> FormEntry for FormField<C> {
    fn check(&self) -> bool {
        if !self.availability_now().submits() {
            return true;
        }
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

    fn send(&self, out: &mut Vec<(String, String)>) {
        if !self.availability_now().submits() {
            return;
        }
        self.name.with_untracked(|n| {
            if let Some((name, encode)) = n {
                let value = gui::untracked(|| self.control.value());
                out.extend(encode(&value).pairs(name));
            }
        });
    }
}

// A form is the fields its caller lists: `FormField`s are `Copy` handles
// made in the caller's setup, so the caller holds them, and what a submit
// checks (a whole form, a stepper's step, a dialog's fields) reads at the
// call site. There is no registry beside it: one way to submit, one to
// reset (ADR-0025 §9).

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

/// What a native form would send for `fields`: each named, enabled field's
/// pairs, in the order given.
#[must_use]
pub fn form_data(fields: &[&dyn FormEntry]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for f in fields {
        f.send(&mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use gui::Signal;

    use super::{Checks, FormData, FormEntry, FormField, OnChange, Unset, form_data, form_reset, form_submit};
    use crate::availability::Availability;

    const ON_SUBMIT: Checks = Checks {
        on_blur: false,
        on_change: OnChange::Recheck,
    };

    fn required(v: &str) -> Vec<String> {
        if v.is_empty() {
            vec!["Required".to_owned()]
        } else {
            Vec::new()
        }
    }

    #[test]
    fn pairs_as_reka_twins_send_them() {
        let name = "f";
        let pairs = |d: FormData| d.pairs(name);
        assert_eq!(pairs(FormData::Text("0".into())), [("f".to_owned(), "0".to_owned())]);
        assert_eq!(pairs(FormData::Text(String::new())), [("f".to_owned(), String::new())]);
        assert_eq!(
            pairs(FormData::List(vec!["a".into(), "b".into()])),
            [("f[0]".to_owned(), "a".to_owned()), ("f[1]".to_owned(), "b".to_owned())]
        );
        assert!(pairs(FormData::List(Vec::new())).is_empty());
        assert_eq!(
            pairs(FormData::Toggle(Some("on".into()))),
            [("f".to_owned(), "on".to_owned())]
        );
        assert!(pairs(FormData::Toggle(None)).is_empty(), "off sends nothing");
    }

    #[test]
    fn submit_checks_every_field_and_reset_restores_both_owners() {
        let name = Signal::new(String::new());
        let terms = Signal::new(false);
        let a = FormField::new(name, ON_SUBMIT, |v: &String| required(v)).named("name");
        let b = FormField::new(terms, ON_SUBMIT, |v: &bool| {
            if *v { Vec::new() } else { vec!["Accept".to_owned()] }
        })
        .named("terms");
        assert!(!form_submit(&[&a, &b]));
        assert_eq!(a.errors(), ["Required"]);
        assert_eq!(b.errors(), ["Accept"], "the second is checked too");
        name.set("Ada".into());
        terms.set(true);
        assert!(a.errors().is_empty(), "rechecked as it changed once checked");
        assert!(form_submit(&[&a, &b]));
        assert_eq!(
            form_data(&[&a, &b]),
            [
                ("name".to_owned(), "Ada".to_owned()),
                ("terms".to_owned(), "on".to_owned())
            ]
        );
        form_reset(&[&a, &b]);
        assert_eq!(name.get_untracked(), "", "the value is back where it started");
        assert!(!terms.get_untracked());
        assert!(a.errors().is_empty() && !a.invalid());
    }

    #[test]
    fn a_form_skips_disabled_fields() {
        let scope = gui::Scope::new();
        scope.run(|| {
            let email = Signal::new("a@b.c".to_owned());
            let field = FormField::new(email, ON_SUBMIT, |v: &String| required(v)).named("email");
            let off = FormField::new(Signal::new(String::new()), ON_SUBMIT, |v: &String| required(v))
                .named("off")
                .availability(Availability::Disabled);
            assert!(form_submit(&[&field, &off]), "a disabled field is not checked");
            assert_eq!(form_data(&[&field, &off]), [("email".to_owned(), "a@b.c".to_owned())]);
            // Bound to the control's own state, the field follows it.
            let disabled = Signal::new(true);
            let bound = FormField::new(Signal::new(String::new()), ON_SUBMIT, |v: &String| required(v))
                .named("bound")
                .availability(move || Availability::from_disabled(disabled.get()));
            assert!(form_submit(&[&bound]), "checked as its control is: disabled");
            assert!(form_data(&[&bound]).is_empty());
            disabled.set(false);
            assert!(!form_submit(&[&bound]), "enabled with its control, it is checked");
            assert_eq!(form_data(&[&bound]), [("bound".to_owned(), String::new())]);
            email.set(String::new());
            form_reset(&[&field, &off]);
            assert_eq!(email.get_untracked(), "a@b.c");
        });
        scope.dispose();
    }

    #[test]
    fn a_disabled_field_is_never_checked() {
        let scope = gui::Scope::new();
        scope.run(|| {
            const ON_CHANGE: Checks = Checks {
                on_blur: false,
                on_change: OnChange::Check,
            };
            let disabled = Signal::new(true);
            let text = Signal::new("x".to_owned());
            let field = FormField::new(text, ON_CHANGE, |v: &String| required(v))
                .availability(move || Availability::from_disabled(disabled.get()));
            // A value changed from code while disabled is not checked.
            text.set(String::new());
            assert!(field.errors().is_empty() && !field.invalid());
            assert!(field.valid(), "a disabled invalid field is valid");
            // Enabled again, the change it missed is checked.
            disabled.set(false);
            assert_eq!(field.errors(), ["Required"]);
            assert!(field.invalid() && !field.valid());
            // Disabled again, it contributes neither errors nor invalidity.
            disabled.set(true);
            assert!(field.errors().is_empty() && !field.invalid() && field.valid());
            text.set("y".into());
            text.set(String::new());
            disabled.set(false);
            assert_eq!(field.errors(), ["Required"], "a checked field is checked again");

            // Recheck: enabled again, a field never checked stays quiet,
            // a checked one is checked again.
            let off = Signal::new(true);
            let other = Signal::new(String::new());
            let recheck = FormField::new(other, ON_SUBMIT, |v: &String| required(v))
                .availability(move || Availability::from_disabled(off.get()));
            other.set("a".into());
            other.set(String::new());
            off.set(false);
            assert!(recheck.errors().is_empty(), "never checked, it is not checked");
            assert!(!recheck.check());
            off.set(true);
            other.set("a".into());
            assert!(!recheck.invalid());
            off.set(false);
            assert!(
                recheck.errors().is_empty() && recheck.valid(),
                "checked again with its new value"
            );
        });
        scope.dispose();
    }

    #[test]
    fn enabled_again_clear_clears_on_a_change_and_keep_keeps() {
        let scope = gui::Scope::new();
        scope.run(|| {
            let field_of = |on_change| {
                let off = Signal::new(false);
                let text = Signal::new(String::new());
                let field = FormField::new(
                    text,
                    Checks {
                        on_blur: false,
                        on_change,
                    },
                    |v: &String| required(v),
                )
                .availability(move || Availability::from_disabled(off.get()));
                assert!(!field.check());
                assert_eq!(field.errors(), ["Required"], "{on_change:?}: checked");
                (off, text, field)
            };

            // Clear: enabled again with its value unchanged, the errors stay;
            // changed while disabled, they go until the next check.
            let (off, text, field) = field_of(OnChange::Clear);
            off.set(true);
            off.set(false);
            assert_eq!(field.errors(), ["Required"], "Clear: no change, nothing cleared");
            off.set(true);
            text.set("a".into());
            assert!(field.errors().is_empty(), "disabled, it shows none");
            off.set(false);
            assert!(field.errors().is_empty(), "Clear: the change clears them");
            text.set(String::new());
            assert!(field.errors().is_empty(), "Clear: a change does not check");
            assert!(!field.check(), "the next check shows them again");

            // Keep: enabled again, its errors are as they were, a change
            // while disabled or not.
            let (off, text, field) = field_of(OnChange::Keep);
            off.set(true);
            text.set("a".into());
            off.set(false);
            assert_eq!(field.errors(), ["Required"], "Keep: they stay until the next check");
            assert!(field.check(), "checked, its new value passes");
            assert!(field.errors().is_empty());
        });
        scope.dispose();
    }

    #[test]
    fn an_unset_control_is_missing_until_changed() {
        let text = Signal::new(String::new());
        let unset = Unset::new(text);
        let field = FormField::new(unset, ON_SUBMIT, |v: &Option<String>| {
            if v.is_none() {
                vec!["Required".to_owned()]
            } else {
                Vec::new()
            }
        });
        assert!(!field.check());
        text.set("x".into());
        assert!(field.check());
        field.reset();
        assert_eq!(text.get_untracked(), "");
    }
}
