use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, div, text};
use shadcn::{
    PinInputState, PinInputType, input_otp_group, label, pin_input, pin_input_separator, pin_input_slot, theme,
};

use crate::{Render, Shot, demo, page};

fn group(state: &PinInputState, range: std::ops::Range<usize>) -> gui::Div {
    input_otp_group(range.map(|i| pin_input_slot(state, i, false)))
}

pub fn render() -> Render {
    let simple = PinInputState::new(6, PinInputType::Text);
    let digits = PinInputState::new(6, PinInputType::Number);
    let pairs = PinInputState::new(6, PinInputType::Text);
    let spaced = PinInputState::new(4, PinInputType::Text);
    Box::new(move || {
        let t = theme();
        let labelled = |name: &str, body: gui::Div| div().grid().gap(2.0).child(label(name.to_owned())).child(body);
        page(
            "Pin Input",
            div()
                .flex()
                .flex_wrap()
                .gap(6.0)
                .child(demo(
                    "PinInputDemo.vue",
                    pin_input()
                        .child(group(&simple, 0..3))
                        .child(pin_input_separator())
                        .child(group(&simple, 3..6)),
                ))
                .child(labelled("Digits Only", pin_input().child(group(&digits, 0..6))))
                .child(labelled(
                    "With Separator",
                    pin_input()
                        .child(group(&pairs, 0..2))
                        .child(pin_input_separator())
                        .child(group(&pairs, 2..4))
                        .child(pin_input_separator())
                        .child(group(&pairs, 4..6)),
                ))
                .child(labelled(
                    "With Spacing",
                    pin_input().child(
                        input_otp_group((0..4).map(|i| pin_input_slot(&spaced, i, true).rounded_md().border()))
                            .gap(2.0),
                    ),
                ))
                .child(
                    text(format!("Simple: \"{}\"", simple.code()))
                        .w_full()
                        .text_sm()
                        .text_color(t.muted_foreground),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let first = s
        .last()
        .nodes
        .iter()
        .find(|n| n.name == Some("pin-input-slot"))
        .map(|n| n.bounds);
    let Some(b) = first else {
        return Err(ScenarioError::TextNotFound("pin-input-slot".to_owned()));
    };
    s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
    s.frame("first focused")?;
    s.type_text("abcd");
    s.frame("typed abcd, focus on 5th")?;
    s.press(NamedKey::Backspace, Modifiers::default());
    s.frame("backspace on empty clears d and steps back")?;
    s.press(NamedKey::Backspace, Modifiers::default());
    s.press(NamedKey::Backspace, Modifiers::default());
    s.frame("two more backspaces")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab to next slot")?;
    Ok(())
}
