use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, Signal, div, text};
use shadcn::{
    Part, PinInputProps, PinInputType, input_otp_group, label, pin_input, pin_input_separator, pin_input_slot, theme,
};

use crate::{Render, Shot, demo, page};

fn group(range: std::ops::Range<usize>) -> gui::Div {
    input_otp_group(range.map(pin_input_slot))
}

pub fn render() -> Render {
    let simple = Signal::labeled("pin", Vec::<Option<char>>::new());
    Box::new(move || {
        let t = theme();
        let labelled = |name: &str, body: Part<PinInputProps>| {
            div()
                .display(Display::Grid)
                .gap(8.0)
                .child(label(name.to_owned()))
                .child(body)
        };
        let code: String = simple.with(|v| v.iter().flatten().collect());
        page(
            "Pin Input",
            div()
                .display(Display::Flex)
                .flex_wrap(true)
                .gap(24.0)
                .child(demo(
                    "PinInputDemo.vue",
                    pin_input()
                        .model(simple)
                        .child(group(0..3))
                        .child(pin_input_separator())
                        .child(group(3..6)),
                ))
                .child(labelled(
                    "Digits Only",
                    pin_input().kind(PinInputType::Number).child(group(0..6)),
                ))
                .child(labelled(
                    "With Separator",
                    pin_input()
                        .child(group(0..2))
                        .child(pin_input_separator())
                        .child(group(2..4))
                        .child(pin_input_separator())
                        .child(group(4..6)),
                ))
                .child(labelled(
                    "With Spacing",
                    pin_input().child(
                        input_otp_group((0..4).map(|i| pin_input_slot(i).invalid(true).radius(8.0).border(1.0)))
                            .gap(8.0),
                    ),
                ))
                .child(
                    text(format!("Simple: \"{code}\""))
                        .width(pct(100.0))
                        .text_size(14.0)
                        .line_height(20.0)
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
