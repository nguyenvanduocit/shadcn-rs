use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, TextField, div, text};
use shadcn::{label, textarea, theme};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    let message = TextField::new("");
    let disabled = TextField::new("");
    let described = TextField::new("");
    Box::new(move || {
        let t = theme();
        page(
            "Textarea",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "TextareaDemo.vue",
                    textarea(&message).placeholder("Type your message here."),
                ))
                .child(demo(
                    "TextareaDisabled.vue",
                    textarea(&disabled)
                        .placeholder("Type your message here.")
                        .disabled(true),
                ))
                .child(section(
                    "With label and description",
                    div()
                        .grid()
                        .gap(3.0)
                        .max_w_px(448.0)
                        .child(label("Your message"))
                        .child(textarea(&described).placeholder("Type your message here."))
                        .child(
                            text("Type your message and press enter to send.")
                                .text_sm()
                                .text_color(t.muted_foreground),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus")?;
    s.type_text("First line of the message.");
    s.press(NamedKey::Enter, Modifiers::default());
    s.type_text("Second line, after Enter.");
    s.press(NamedKey::Enter, Modifiers::default());
    s.type_text(
        "A third line long enough to wrap across the width of the box, so the textarea grows with what is typed in it.",
    );
    s.frame("typed three lines")?;
    s.click(0.0, 0.0);
    let (x, y) = s.locate("Type your message here.")?;
    s.click(x, y);
    s.type_text("ignored");
    s.frame("disabled ignores typing")?;
    Ok(())
}
