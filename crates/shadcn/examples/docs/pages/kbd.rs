use gui::prelude::*;
use gui::{Icon, ScenarioError, TextField, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, InputGroupAlign, Orientation, button, button_group, input_group, input_group_addon,
    input_group_input, kbd, kbd_group, kbd_on_tooltip, theme, tooltip, tooltip_content, tooltip_trigger,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let search = TextField::new("");
    Box::new(move || {
        let t = theme();
        page(
            "Kbd",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "KbdDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(kbd_group().children(["⌘", "⇧", "⌥", "⌃"].map(|k| kbd().child(k))))
                        .child(
                            kbd_group()
                                .child(kbd().child("Ctrl"))
                                .child(text("+"))
                                .child(kbd().child("B")),
                        ),
                ))
                .child(demo(
                    "KbdGroupDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(
                            // A `<p>` flowing text around an inline group: a space either side.
                            div()
                                .display(Display::Flex)
                                .align_items(Align::Center)
                                .text_size(14.0)
                                .line_height(20.0)
                                .text_color(t.muted_foreground)
                                .child(text("Use "))
                                .child(
                                    kbd_group()
                                        .child(kbd().child("Ctrl + B"))
                                        .child(kbd().child("Ctrl + K")),
                                )
                                .child(text(" to open the command palette")),
                        ),
                ))
                .child(demo(
                    "KbdWithButton.vue",
                    div()
                        .display(Display::Flex)
                        .flex_wrap(true)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .padding_right(8.0)
                                .child("Accept")
                                .child(kbd().child("⏎")),
                        )
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .padding_right(8.0)
                                .child("Cancel")
                                .child(kbd().child("Esc")),
                        ),
                ))
                .child(demo(
                    "KbdWithInputGroup.vue",
                    div()
                        .display(Display::Flex)
                        .width(pct(100.0))
                        .max_width(320.0)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(24.0)
                        .child(
                            input_group(input_group_input(&search).placeholder("Search..."))
                                .addon(input_group_addon(InputGroupAlign::InlineStart).child(icon(Icon::Search)))
                                .addon(
                                    input_group_addon(InputGroupAlign::InlineEnd)
                                        .kbd(kbd().child("⌘"))
                                        .kbd(kbd().child("K")),
                                ),
                        ),
                ))
                .child(demo(
                    "KbdWithTooltip.vue",
                    // `TooltipTrigger as-child`: each button is the group's
                    // item and its tooltip's anchor; each tooltip's anchor
                    // holds one, so the buttons are joined here.
                    div()
                        .display(Display::Flex)
                        .flex_wrap(true)
                        .gap(16.0)
                        .child(button_group(
                            Orientation::Horizontal,
                            [
                                tooltip()
                                    .child(tooltip_trigger(
                                        button(ButtonVariant::Outline, ButtonSize::Sm)
                                            .radius_right(0.0)
                                            .child("Save"),
                                    ))
                                    .child(
                                        tooltip_content().child(
                                            div()
                                                .display(Display::Flex)
                                                .align_items(Align::Center)
                                                .gap(8.0)
                                                .child("Save Changes")
                                                .child(kbd_on_tooltip().child("S")),
                                        ),
                                    ),
                                tooltip()
                                    .child(tooltip_trigger(
                                        button(ButtonVariant::Outline, ButtonSize::Sm)
                                            .radius_left(0.0)
                                            .border_left(0.0)
                                            .child("Print"),
                                    ))
                                    .child(
                                        tooltip_content().child(
                                            div()
                                                .display(Display::Flex)
                                                .align_items(Align::Center)
                                                .gap(8.0)
                                                .child("Print Document")
                                                .child(
                                                    kbd_group()
                                                        .child(kbd_on_tooltip().child("Ctrl"))
                                                        .child(kbd_on_tooltip().child("P")),
                                                ),
                                        ),
                                    ),
                            ],
                        )),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Print")?;
    s.hover(x, y);
    s.settle()?;
    s.frame("print tooltip with its keys")?;
    Ok(())
}
