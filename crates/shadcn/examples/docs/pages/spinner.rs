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
                .flex_col()
                .gap(6.0)
                .child(demo(
                    "SpinnerBasic.vue",
                    div()
                        .flex_col()
                        .items_center()
                        .justify_center()
                        .gap(8.0)
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
                        .flex()
                        .items_center()
                        .gap(6.0)
                        .child(spinner().size(3.0))
                        .child(spinner().size(4.0))
                        .child(spinner().size(6.0))
                        .child(spinner().size(8.0)),
                ))
                .child(demo(
                    "SpinnerColorDemo.vue",
                    div()
                        .flex()
                        .items_center()
                        .gap(6.0)
                        .children(COLORS.map(|(l, c, h)| spinner().size(6.0).text_color(Color::oklch(l, c, h)))),
                ))
                .child(demo(
                    "SpinnerBadgeDemo.vue",
                    div()
                        .flex()
                        .items_center()
                        .gap(4.0)
                        .radius_var(19.2)
                        .child(badge(BadgeVariant::Default).child(spinner()).child("Syncing"))
                        .child(badge(BadgeVariant::Secondary).child(spinner()).child("Updating"))
                        .child(badge(BadgeVariant::Outline).child(spinner()).child("Processing")),
                ))
                .child(demo(
                    "SpinnerButtonDemo.vue",
                    div().flex_col().items_center().gap(4.0).children(
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
                        .w_full()
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
                        .flex()
                        .w_full()
                        .max_w_px(448.0)
                        .flex_col()
                        .gap(4.0)
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
                                            .ml_auto()
                                            .child(icon(Icon::ArrowUp)),
                                    ),
                            ),
                        ),
                ))
                .child(demo(
                    "SpinnerItemDemo.vue",
                    div()
                        .flex()
                        .w_full()
                        .max_w_px(448.0)
                        .flex_col()
                        .gap(4.0)
                        .radius_var(16.0)
                        .child(
                            item(ItemVariant::Outline, ItemSize::Default)
                                .child(item_media(ItemMediaVariant::Icon).child(spinner()))
                                .child(
                                    item_content()
                                        .child(item_title("Downloading..."))
                                        .child(item_description("129 MB / 1000 MB")),
                                )
                                .child(
                                    item_actions()
                                        .child(button(ButtonVariant::Outline, ButtonSize::Sm).child("Cancel")),
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
