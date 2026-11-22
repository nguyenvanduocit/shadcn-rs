use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, div, icon, text};
use shadcn::{ButtonSize, ButtonVariant, button, collapsible, collapsible_content, collapsible_trigger_as, theme};

use crate::{Render, Shot, demo, page};

fn repository(name: &str) -> gui::Div {
    let t = theme();
    div()
        .rounded_md()
        .border()
        .border_color(t.border)
        .px(4.0)
        .py(2.0)
        .font_mono()
        .text_sm()
        .child(name.to_owned())
}

pub fn render() -> Render {
    Box::new(move || {
        page(
            "Collapsible",
            demo(
                "CollapsibleDemo.vue",
                collapsible()
                    .flex_col()
                    .w_px(350.0)
                    .gap(2.0)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(4.0)
                            .px(4.0)
                            .child(text("@peduarte starred 3 repositories").text_sm().font_semibold())
                            .child(collapsible_trigger_as(
                                button(ButtonVariant::Ghost, ButtonSize::Icon)
                                    .size(8.0)
                                    .child(icon(Icon::ChevronsUpDown)),
                            )),
                    )
                    .child(repository("@radix-ui/primitives"))
                    .child(
                        collapsible_content()
                            .flex_col()
                            .gap(2.0)
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
