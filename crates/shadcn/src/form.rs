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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gui::element::Node;
use gui::{Div, Effect, Fluent, FocusHandle, Signal, Styled, TextField, div, text};

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

/// A control's state a [`FormField`] reads, watches and resets.
pub trait FormValue: Clone + 'static {
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
/// `initialValues`): its value is `None`, which zod reads as missing
/// ("Required"), until the user first changes it.
#[derive(Clone)]
pub struct Unset<C: FormValue> {
    control: C,
    /// What the control shows while there is no value.
    blank: C::Value,
    entered: Signal<bool>,
    _watch: Rc<Effect>,
}

impl<C: FormValue> Unset<C> {
    #[must_use]
    pub fn new(control: C) -> Self {
        let blank = control.value();
        let entered = Signal::new(false);
        let watch = {
            let (control, entered, blank) = (control.clone(), entered.clone(), blank.clone());
            Effect::new(move || {
                if control.value() != blank {
                    entered.set(true);
                }
            })
        };
        Self {
            control,
            blank,
            entered,
            _watch: Rc::new(watch),
        }
    }

    /// The control's own state, to bind the control to.
    #[must_use]
    pub fn control(&self) -> &C {
        &self.control
    }
}

impl<C: FormValue> FormValue for Unset<C> {
    type Value = Option<C::Value>;

    fn value(&self) -> Option<C::Value> {
        let value = self.control.value();
        self.entered.get().then_some(value)
    }

    fn set_value(&self, value: Option<C::Value>) {
        self.control
            .set_value(value.clone().unwrap_or_else(|| self.blank.clone()));
        self.entered.set(value.is_some());
    }

    fn focus(&self) -> Option<FocusHandle> {
        self.control.focus()
    }
}

/// A field's check: every message its value fails, none when it is valid.
type Rule<V> = Rc<dyn Fn(&V) -> Vec<String>>;

/// One field of a form: its control's state, the rule it is checked by and
/// the errors the last check found. Create it once, outside the render
/// closure; it watches its control for the events its [`Checks`] name.
pub struct FormField<C: FormValue> {
    control: C,
    initial: C::Value,
    rule: Rule<C::Value>,
    errors: Signal<Vec<String>>,
    /// Whether it was checked since the form was made or reset.
    checked: Rc<Cell<bool>>,
    /// The value the watcher last saw, so a reset is no change.
    seen: Rc<RefCell<C::Value>>,
    _watch: Rc<Effect>,
}

impl<C: FormValue> Clone for FormField<C> {
    fn clone(&self) -> Self {
        Self {
            control: self.control.clone(),
            initial: self.initial.clone(),
            rule: Rc::clone(&self.rule),
            errors: self.errors.clone(),
            checked: Rc::clone(&self.checked),
            seen: Rc::clone(&self.seen),
            _watch: Rc::clone(&self._watch),
        }
    }
}

impl<C: FormValue> FormField<C> {
    /// A field of `control`, checked by `rule` (every message that fails,
    /// none when the value is valid) on a submit and on `checks`.
    #[must_use]
    pub fn new(control: C, checks: Checks, rule: impl Fn(&C::Value) -> Vec<String> + 'static) -> Self {
        let initial = control.value();
        let rule: Rule<C::Value> = Rc::new(rule);
        let errors = Signal::labeled("form field errors", Vec::new());
        let checked = Rc::new(Cell::new(false));
        let seen = Rc::new(RefCell::new(initial.clone()));
        let was_focused = Cell::new(false);
        let watch = {
            let (control, rule, errors, checked, seen) = (
                control.clone(),
                Rc::clone(&rule),
                errors.clone(),
                Rc::clone(&checked),
                Rc::clone(&seen),
            );
            Effect::new(move || {
                let value = control.value();
                let focused = control.focus().is_some_and(FocusHandle::is_focused);
                let blurred = was_focused.replace(focused) && !focused;
                let changed = *seen.borrow() != value;
                if changed {
                    seen.replace(value.clone());
                }
                let check = |errors: &Signal<Vec<String>>| {
                    checked.set(true);
                    errors.set(rule(&value));
                };
                if changed {
                    match checks.on_change {
                        OnChange::Keep => {}
                        OnChange::Clear => errors.set(Vec::new()),
                        OnChange::Recheck => {
                            if checked.get() {
                                check(&errors);
                            }
                        }
                        OnChange::Check => check(&errors),
                    }
                }
                if blurred && checks.on_blur {
                    check(&errors);
                }
            })
        };
        Self {
            control,
            initial,
            rule,
            errors,
            checked,
            seen,
            _watch: Rc::new(watch),
        }
    }

    /// The control's state, to bind the control to.
    #[must_use]
    pub fn control(&self) -> &C {
        &self.control
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
        (self.rule)(&self.control.value()).is_empty()
    }

    /// Whether the last check found an error (reactive): the field's
    /// `data-invalid` and its control's `aria-invalid`.
    #[must_use]
    pub fn invalid(&self) -> bool {
        !self.errors.get().is_empty()
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
        let found = (self.rule)(&self.control.value());
        let valid = found.is_empty();
        self.checked.set(true);
        self.errors.set(found);
        valid
    }

    fn reset(&self) {
        self.seen.replace(self.initial.clone());
        self.control.set_value(self.initial.clone());
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
    div().named("form-item").grid().gap(2.0)
}

/// The field's name, destructive while it is `invalid`.
#[must_use]
pub fn form_label(name: impl Into<String>, invalid: bool) -> Div {
    let t = theme();
    label(name)
        .named("form-label")
        .when(invalid, |d| d.text_color(t.destructive))
}

/// `control` (an input, a textarea, a select trigger) marked `invalid` as
/// `aria-invalid` marks it: a destructive border, and a destructive ring
/// while focused.
#[must_use]
pub fn form_control<K>(control: Node<K>, invalid: bool) -> Node<K> {
    let t = theme();
    control.when(invalid, |c| {
        c.border_color(t.destructive)
            .focus_visible(move |s| destructive_ring(s, &t).border_color(t.destructive))
    })
}

/// Help text under the control.
#[must_use]
pub fn form_description(description: impl Into<String>) -> Div {
    let t = theme();
    div()
        .named("form-description")
        .text_color(t.muted_foreground)
        .text_sm()
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
            .text_sm()
            .child(text(e))
    })
}
