use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, div, icon, text};
use shadcn::{ButtonSize, ButtonVariant, button, collapsible, collapsible_content, collapsible_trigger_as, theme};

use crate::{Render, Shot, demo, page};

fn repository(name: &str) -> gui::Div {
    let t = theme();
    div()
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .padding_x(16.0)
        .padding_y(8.0)
        .font_family(FontFamily::GEIST_MONO)
        .text_size(14.0)
        .line_height(20.0)
        .child(name.to_owned())
}

pub fn render() -> Render {
    Box::new(move || {
        page(
            "Collapsible",
            demo(
                "CollapsibleDemo.vue",
                collapsible()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .width(350.0)
                    .gap(8.0)
                    .child(
                        div()
                            .display(Display::Flex)
                            .align_items(Align::Center)
                            .justify_content(Justify::Between)
                            .gap(16.0)
                            .padding_x(16.0)
                            .child(
                                text("@peduarte starred 3 repositories")
                                    .text_size(14.0)
                                    .line_height(20.0)
                                    .font_weight(600),
                            )
                            .child(collapsible_trigger_as(
                                button(ButtonVariant::Ghost, ButtonSize::Icon)
                                    .size(32.0)
                                    .child(icon(Icon::ChevronsUpDown)),
                            )),
                    )
                    .child(repository("@radix-ui/primitives"))
                    .child(
                        collapsible_content()
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::Column)
                            .gap(8.0)
                            .child(repository("@radix-ui/colors"))
                            .child(repository("@stitches/react")),
                    ),
            ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("closed")?;
    // The toggle is the icon button at the right end of the heading's row.
    let (_, y) = s.locate("@peduarte starred 3 repositories")?;
    s.click(32.0 + 350.0 - 16.0 - 16.0, y);
    s.frame("clicked open")?;
    s.press(NamedKey::Tab, Modifiers::default());
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("tab and enter close")?;
    Ok(())
}
