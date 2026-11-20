//! TanStack Form's form demos (`TanStackForm*.vue`): zod schemas checked on
//! submit, a field's errors gone at its next change.

use gui::ScenarioError;
use shadcn::{Toaster, toaster};

use super::form_cards::{self, Cards, Files, Library};
use crate::{Render, Shot, page};

const FILES: Files = Files {
    demo: "TanStackFormDemo.vue",
    input: "TanStackFormInput.vue",
    textarea: "TanStackFormTextarea.vue",
    select: "TanStackFormSelect.vue",
    switch: "TanStackFormSwitch.vue",
    radio_group: "TanStackFormRadioGroup.vue",
    checkbox: "TanStackFormCheckbox.vue",
    array: "TanStackFormArray.vue",
    complex: "TanStackFormComplex.vue",
};

pub fn render() -> Render {
    let toasts = Toaster::new();
    let cards = Cards::new(Library::TanStack, &toasts);
    Box::new(move || page("Form (TanStack Form)", cards.view(&FILES).child(toaster(&toasts))))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    form_cards::script(s)
}
