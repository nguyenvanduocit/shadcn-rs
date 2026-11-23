use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, div};
use shadcn::{
    NumberFieldProps, NumberFormat, Part, SignDisplay, label, number_field, number_field_content,
    number_field_decrement, number_field_increment, number_field_input,
};

use crate::{Render, Shot, demo, page, section};

/// The centre of the `nth` node named `name` in the last frame.
fn spot(s: &Shot, name: &str, nth: usize) -> Result<(f32, f32), ScenarioError> {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name))
        .nth(nth)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound(format!("{name} #{nth}")))
}

fn field(name: &str, root: Part<NumberFieldProps>) -> Part<NumberFieldProps> {
    root.child(label(name)).child(
        number_field_content()
            .child(number_field_decrement())
            .child(number_field_input())
            .child(number_field_increment()),
    )
}

pub fn render() -> Render {
    let plain = NumberFormat::default();
    Box::new(move || {
        let age = || number_field().default_value(18.0);
        page(
            "Number Field",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("NumberFieldDemo.vue", field("Age", age().min(0.0))))
                .child(demo("NumberFieldDisabledDemo.vue", field("Age", age().disabled(true))))
                .child(demo(
                    "NumberFieldCurrencyDemo.vue",
                    field(
                        "Balance",
                        number_field()
                            .default_value(1500.0)
                            .format(NumberFormat::currency("EUR", true)),
                    ),
                ))
                .child(demo(
                    "NumberFieldDecimalDemo.vue",
                    field(
                        "Number",
                        number_field().default_value(5.0).format(NumberFormat {
                            sign: SignDisplay::ExceptZero,
                            min_fraction: 1,
                            ..plain.clone()
                        }),
                    ),
                ))
                .child(demo(
                    "NumberFieldPercentageDemo.vue",
                    field(
                        "Percent",
                        number_field()
                            .default_value(0.05)
                            .step(0.01)
                            .format(NumberFormat::percent()),
                    ),
                ))
                .child(section(
                    "Bounded decimals, grouped thousands",
                    div()
                        .display(Display::Flex)
                        .gap(24.0)
                        .child(
                            field("Ratio", number_field().default_value(0.5).min(0.0).max(1.0).step(0.1)).width(160.0),
                        )
                        .child(field("Amount", number_field().default_value(1234.0)).width(160.0)),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // The demo fields collapse (gui inputs have no intrinsic width), so the
    // buttons are driven on the sized fields below them.
    let (x, y) = spot(s, "increment", 6)?;
    s.click(x, y);
    s.click(x, y);
    s.frame("amount plus twice: 1,236")?;
    let (x, y) = spot(s, "decrement", 5)?;
    for _ in 0..8 {
        s.click(x, y);
    }
    // Today the input, laid out after the − button, takes these clicks: gui
    // does not lift absolutely placed boxes above their in-flow siblings.
    s.frame("ratio minus to the minimum: 0, minus disabled")?;
    s.click(0.0, 0.0);
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("tab focus age")?;
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.frame("arrow up three times: 21")?;
    s.press(NamedKey::Backspace, Modifiers::default());
    s.press(NamedKey::Backspace, Modifiers::default());
    s.type_text("-5");
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("typed -5 and Enter: clamped to 0")?;
    let (x, y) = spot(s, "number-field-input", 5)?;
    s.click(x, y);
    for _ in 0..12 {
        s.press(NamedKey::ArrowUp, Modifiers::default());
    }
    s.frame("ratio up past max: 1, plus disabled")?;
    let (x, y) = spot(s, "number-field-input", 6)?;
    s.click(x, y);
    s.type_text("9");
    s.press(NamedKey::Tab, Modifiers::default());
    s.frame("typed 9 then blur: 12,369")?;
    let (x, y) = spot(s, "increment", 1)?;
    s.click(x, y);
    s.frame("disabled ignores plus")?;
    // The formats: a step in each keeps its way of writing the value.
    let (x, y) = spot(s, "number-field-input", 2)?;
    s.click(x, y);
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.frame("balance up: EUR 1,501.00")?;
    let (x, y) = spot(s, "number-field-input", 3)?;
    s.click(x, y);
    for _ in 0..6 {
        s.press(NamedKey::ArrowDown, Modifiers::default());
    }
    s.frame("number down six: -1.0")?;
    let (x, y) = spot(s, "number-field-input", 4)?;
    s.click(x, y);
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.frame("percent up a step: 6%")?;
    Ok(())
}
