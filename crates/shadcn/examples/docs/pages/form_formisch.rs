//! Formisch's form demos (`Formisch*.vue`): valibot schemas, checked on
//! submit and again on every input after it.

use gui::ScenarioError;
use shadcn::toaster;

use super::form_cards::{self, Cards, Files, Library};
use crate::{Render, Shot, page};

const FILES: Files = Files {
    demo: "FormischDemo.vue",
    input: "FormischInput.vue",
    textarea: "FormischTextarea.vue",
    select: "FormischSelect.vue",
    switch: "FormischSwitch.vue",
    radio_group: "FormischRadioGroup.vue",
    checkbox: "FormischCheckbox.vue",
    array: "FormischArray.vue",
    complex: "FormischComplex.vue",
};

pub fn render() -> Render {
    let cards = Cards::new(Library::Formisch);
    Box::new(move || page("Form (Formisch)", cards.view(&FILES).child(toaster())))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    form_cards::script(s)
}
