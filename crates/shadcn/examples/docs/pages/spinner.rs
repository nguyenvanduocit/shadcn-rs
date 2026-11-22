use gui::prelude::*;
use gui::{Color, Icon, ScenarioError, TextField, div, icon};
use shadcn::{
    BadgeVariant, ButtonSize, ButtonVariant, EmptyMediaVariant, InputGroupAlign, InputGroupButtonSize,
    ItemMediaVariant, ItemSize, ItemVariant, badge, button, empty, empty_content, empty_description, empty_header,
    empty_media, empty_title, input_group, input_group_addon, input_group_button, input_group_input,
    input_group_textarea, item, item_actions, item_content, item_description, item_footer, item_media, item_title,
    progress, spinner, spinner_of,
};

use crate::{Render, Shot, demo, page};

/// Tailwind's `red-500`, `green-500`, `blue-500`, `yellow-500`, `purple-500`.
const COLORS: [(f32, f32, f32); 5] = [
    (0.637, 0.237, 25.331),
    (0.723, 0.219, 149.579),
    (0.623, 0.214, 259.815),
    (0.795, 0.184, 86.047),
    (0.627, 0.265, 303.9),
];

pub fn render() -> Render {
    let message = TextField::new("");
    let long_message = TextField::new("");
    Box::new(move || {
        page(
            "Spinner",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo(
                    "SpinnerBasic.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .justify_content(Justify::Center)
                        .gap(32.0)
                        .child(spinner()),
                ))
                .child(demo(
                    "SpinnerDemo.vue",
                    button(ButtonVariant::Default, ButtonSize::Default)
                        .disabled(true)
                        .child(spinner())
                        .child("Processing Payment"),
                ))
                .child(demo(
                    "SpinnerSizeDemo.vue",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(24.0)
                        .child(spinner().size(12.0))
                        .child(spinner().size(16.0))
                        .child(spinner().size(24.0))
                        .child(spinner().size(32.0)),
                ))
                .child(demo(
                    "SpinnerColorDemo.vue",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(24.0)
                        .children(COLORS.map(|(l, c, h)| spinner().size(24.0).text_color(Color::oklch(l, c, h)))),
                ))
                .child(demo(
                    "SpinnerBadgeDemo.vue",
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .child(badge(BadgeVariant::Default).child(spinner()).child("Syncing"))
                        .child(badge(BadgeVariant::Secondary).child(spinner()).child("Updating"))
                        .child(badge(BadgeVariant::Outline).child(spinner()).child("Processing")),
                ))
                .child(demo(
                    "SpinnerButtonDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(16.0)
                        .children(
                            [
                                (ButtonVariant::Default, "Loading..."),
                                (ButtonVariant::Outline, "Please wait"),
                                (ButtonVariant::Secondary, "Processing"),
                            ]
                            .map(|(variant, label)| {
                                button(variant, ButtonSize::Sm)
                                    .disabled(true)
                                    .child(spinner())
                                    .child(label)
                            }),
                        ),
                ))
                .child(demo(
                    "SpinnerEmptyDemo.vue",
                    empty()
                        .width(pct(100.0))
                        .child(
                            empty_header()
                                // The spinner keeps its own `size-4` on the tile.
                                .child(empty_media(EmptyMediaVariant::Icon).child(spinner()))
                                .child(empty_title("Processing your request"))
                                .child(empty_description(
                                    "Please wait while we process your request. Do not refresh the page.",
                                )),
                        )
                        .child(empty_content().child(button(ButtonVariant::Outline, ButtonSize::Sm).child("Cancel"))),
                ))
                .child(demo("SpinnerCustomDemo.vue", spinner_of(Icon::Loader)))
                .child(demo(
                    "SpinnerInputGroupDemo.vue",
                    div()
                        .display(Display::Flex)
                        .width(pct(100.0))
                        .max_width(448.0)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        .child(
                            input_group(
                                input_group_input(&message)
                                    .placeholder("Send a message...")
                                    .disabled(true),
                            )
                            .addon(input_group_addon(InputGroupAlign::InlineEnd).child(spinner())),
                        )
                        .child(
                            input_group(
                                input_group_textarea(&long_message)
                                    .placeholder("Send a message...")
                                    .disabled(true),
                            )
                            .addon(
                                input_group_addon(InputGroupAlign::BlockEnd)
                                    .child(spinner())
                                    .child("Validating...")
                                    .button(
                                        input_group_button(ButtonVariant::Default, InputGroupButtonSize::Xs)
                                            .margin_left(Length::Auto)
                                            .child(icon(Icon::ArrowUp)),
                                    ),
                            ),
                        ),
                ))
                .child(demo(
                    "SpinnerItemDemo.vue",
                    div()
                        .display(Display::Flex)
                        .width(pct(100.0))
                        .max_width(448.0)
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .gap(16.0)
                        // `--radius: 1rem`: the item (`rounded-md`), its media
                        // (`rounded-sm`) and its button (`rounded-md`) follow it.
                        .child(
                            item(ItemVariant::Outline, ItemSize::Default)
                                .radius(16.0 * 0.8)
                                .child(item_media(ItemMediaVariant::Icon).radius(16.0 * 0.6).child(spinner()))
                                .child(
                                    item_content()
                                        .child(item_title("Downloading..."))
                                        .child(item_description("129 MB / 1000 MB")),
                                )
                                .child(
                                    item_actions().child(
                                        button(ButtonVariant::Outline, ButtonSize::Sm)
                                            .radius(16.0 * 0.8)
                                            .child("Cancel"),
                                    ),
                                )
                                .child(item_footer().child(progress(75.0))),
                        ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("at 0s")?;
    s.at(0.125);
    s.frame("at 0.125s, turned 45 degrees")?;
    Ok(())
}
