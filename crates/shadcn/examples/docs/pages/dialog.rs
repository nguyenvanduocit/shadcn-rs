use gui::prelude::*;
use gui::{Element, Modifiers, NamedKey, ScenarioError, TextField, div};
use shadcn::{
    ButtonSize, ButtonVariant, Checks, FieldOrientation, FormEntry, FormField, OnChange, button, dialog, dialog_close,
    dialog_content, dialog_description, dialog_footer, dialog_header, dialog_title, dialog_trigger, field,
    field_description, field_error, field_group, field_label, form_submit, input, label, toaster,
};

use super::form_cards::{Json, Pre, submitted};
use crate::{MOTION_MS, Render, Shot, demo, motion_frames, page};

/// `DialogForm.vue`'s schema, `z.string().min(2).max(50)`, with zod 3's
/// messages. vee-validate leaves a field never typed in `undefined`, which
/// zod calls "Required"; a `TextField` is never undefined, so empty is.
fn username_errors(v: &str) -> Vec<String> {
    let n = v.chars().count();
    let message = if n == 0 {
        "Required"
    } else if n < 2 {
        "String must contain at least 2 character(s)"
    } else if n > 50 {
        "String must contain at most 50 character(s)"
    } else {
        return Vec::new();
    };
    vec![message.to_owned()]
}

pub fn render() -> Render {
    let name = TextField::new("Pedro Duarte");
    let username_field = TextField::new("@peduarte");
    let link = TextField::new("https://ui.shadcn.com/docs/installation");
    // vee-validate checks on every change of the value and on blur.
    let checks = Checks {
        on_blur: true,
        on_change: OnChange::Check,
    };
    let username = FormField::new(TextField::new(""), checks, |v: &String| username_errors(v));
    let docs = TextField::new("https://www.shadcn-vue.com/docs/installation");
    Box::new(move || {
        let labelled = |name: &str, f: &TextField| div().grid().gap(3.0).child(label(name)).child(input().field(*f));
        page(
            "Dialog",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "DialogDemo.vue",
                    dialog()
                        .flex()
                        .child(dialog_trigger(
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Open Dialog"),
                        ))
                        .child(
                            dialog_content()
                                .max_w_px(425.0)
                                .child(
                                    dialog_header()
                                        .child(dialog_title("Edit profile"))
                                        .child(dialog_description(
                                            "Make changes to your profile here. Click save when you're done.",
                                        )),
                                )
                                .child(
                                    div()
                                        .grid()
                                        .gap(4.0)
                                        .child(labelled("Name", &name))
                                        .child(labelled("Username", &username_field)),
                                )
                                .child(
                                    dialog_footer()
                                        .child(dialog_close(
                                            button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel"),
                                        ))
                                        .child(
                                            button(ButtonVariant::Default, ButtonSize::Default).child("Save changes"),
                                        ),
                                ),
                        ),
                ))
                .child(demo(
                    "DialogCloseButton.vue",
                    dialog()
                        .flex()
                        .child(dialog_trigger(
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Share"),
                        ))
                        .child(
                            dialog_content()
                                .max_w(112.0)
                                .child(
                                    dialog_header()
                                        .child(dialog_title("Share link"))
                                        .child(dialog_description(
                                            "Anyone who has this link will be able to view this.",
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(2.0)
                                        .child(div().grid().flex_1().gap(2.0).child(input().field(link).read_only())),
                                )
                                .child(dialog_footer().justify_start().child(dialog_close(
                                    button(ButtonVariant::Secondary, ButtonSize::Default).child("Close"),
                                ))),
                        ),
                ))
                .child(demo("DialogForm.vue", dialog_form(&username)))
                .child(demo(
                    "DialogResponsive.vue",
                    // `useMediaQuery('(min-width: 640px)')` holds at the gallery's width: the dialog, not the drawer.
                    dialog()
                        .flex()
                        .child(dialog_trigger(
                            button(ButtonVariant::Outline, ButtonSize::Default)
                                .named("responsive-trigger")
                                .child("Open Dialog"),
                        ))
                        .child(
                            dialog_content()
                                .max_w(112.0)
                                .child(
                                    dialog_header()
                                        .child(dialog_title("Share Link"))
                                        .child(dialog_description(
                                            "Anyone who has this link will be able to view this.",
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(2.0)
                                        .child(div().grid().flex_1().gap(2.0).child(input().field(docs).read_only())),
                                )
                                .child(dialog_footer().pt(4.0).child(dialog_close(
                                    button(ButtonVariant::Outline, ButtonSize::Default).child("Close"),
                                ))),
                        ),
                ))
                .child(toaster()),
        )
    })
}

/// `DialogForm.vue`: a dialog whose username field is checked as typed, and
/// whose "Save changes" submits it, toasting the values when they pass.
fn dialog_form(username: &FormField<TextField>) -> Element {
    let invalid = username.invalid();
    let submit = *username;
    dialog()
        .flex()
        .child(dialog_trigger(
            button(ButtonVariant::Outline, ButtonSize::Default).child("Edit Profile"),
        ))
        .child(
            dialog_content()
                .max_w_px(425.0)
                .child(
                    dialog_header()
                        .child(dialog_title("Edit profile"))
                        .child(dialog_description(
                            "Make changes to your profile here. Click save when you're done.",
                        )),
                )
                .child(
                    field_group().child(
                        field(FieldOrientation::Vertical, invalid)
                            .child(field_label("Username"))
                            .child(input().field(username.control()).placeholder("shadcn"))
                            .child(field_description("This is your public display name."))
                            .child(field_error(username.errors())),
                    ),
                )
                .child(
                    dialog_footer().child(
                        button(ButtonVariant::Default, ButtonSize::Default)
                            .on_click(move |_| {
                                let fields: [&dyn FormEntry; 1] = [&submit];
                                if form_submit(&fields) {
                                    let values = Json::Object(vec![("username", Json::Str(submit.control().text()))]);
                                    submitted(&values, Pre::Neutral950);
                                }
                            })
                            .child("Save changes"),
                    ),
                ),
        )
        .into_element()
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &MOTION_MS, |s| s.click_text("Open Dialog"))?;
    motion_frames(s, "closing", &MOTION_MS, |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Open Dialog")?;
    s.frame("open")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab into dialog")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closed")?;
    s.click_text("Open Dialog")?;
    s.settle()?;
    s.click(20.0, 600.0);
    s.frame("overlay press closed")?;
    s.click_text("Share")?;
    s.frame("share open")?;
    // The link is `read-only`: it takes focus and a selection, not edits.
    let (x, y) = s.locate("https://ui.shadcn.com/docs/installation")?;
    s.click(x, y);
    s.type_text("x");
    s.press(NamedKey::Backspace, Modifiers::default());
    s.shortcut(
        'a',
        Modifiers {
            command: true,
            control: true,
            ..Modifiers::default()
        },
    );
    s.frame("read-only link selected, typing changed nothing")?;
    s.click_text("Close")?;
    s.frame("close button closed")?;
    s.click_text("Edit Profile")?;
    s.settle()?;
    s.click_text("Save changes")?;
    s.frame("form required")?;
    let (x, y) = s.locate("shadcn")?;
    s.click(x, y);
    s.type_text("a");
    s.frame("form too short")?;
    s.type_text("da");
    s.click_text("Save changes")?;
    s.settle()?;
    s.frame("form submitted, toast")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.settle()?;
    let trigger = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("responsive-trigger"))
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("responsive-trigger".to_owned()))?;
    s.click(trigger.0, trigger.1);
    s.frame("responsive open")?;
    Ok(())
}
