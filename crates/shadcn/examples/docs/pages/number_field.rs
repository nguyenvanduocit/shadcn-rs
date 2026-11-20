use gui::prelude::*;
use gui::{Modifiers, NamedKey, ScenarioError, div};
use shadcn::{
    NumberFieldState, NumberFormat, SignDisplay, label, number_field, number_field_content, number_field_decrement,
    number_field_increment, number_field_input,
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

fn field(name: &str, state: &NumberFieldState) -> gui::Div {
    number_field().child(label(name)).child(
        number_field_content()
            .child(number_field_decrement(state))
            .child(number_field_input(state))
            .child(number_field_increment(state)),
    )
}

pub fn render() -> Render {
    let plain = NumberFormat::default();
    let age = NumberFieldState::new(18.0, Some(0.0), None, 1.0, plain);
    let off = NumberFieldState::new(18.0, None, None, 1.0, plain);
    off.disabled.set(true);
    let ratio = NumberFieldState::new(0.5, Some(0.0), Some(1.0), 0.1, plain);
    let big = NumberFieldState::new(1234.0, None, None, 1.0, plain);
    let balance = NumberFieldState::new(1500.0, None, None, 1.0, NumberFormat::currency("EUR", true));
    let signed = NumberFieldState::new(
        5.0,
        None,
        None,
        1.0,
        NumberFormat {
            sign: SignDisplay::ExceptZero,
            min_fraction: 1,
            ..plain
        },
    );
    let percent = NumberFieldState::new(0.05, None, None, 0.01, NumberFormat::PERCENT);
    Box::new(move || {
        page(
            "Number Field",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo("NumberFieldDemo.vue", field("Age", &age)))
                .child(demo("NumberFieldDisabledDemo.vue", field("Age", &off)))
                .child(demo("NumberFieldCurrencyDemo.vue", field("Balance", &balance)))
                .child(demo("NumberFieldDecimalDemo.vue", field("Number", &signed)))
                .child(demo("NumberFieldPercentageDemo.vue", field("Percent", &percent)))
                .child(section(
                    "Bounded decimals, grouped thousands",
                    div()
                        .flex()
                        .gap(6.0)
                        .child(field("Ratio", &ratio).w_px(160.0))
                        .child(field("Amount", &big).w_px(160.0)),
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
