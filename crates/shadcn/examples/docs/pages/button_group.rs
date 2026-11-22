use gui::prelude::*;
use gui::{Icon, ScenarioError, Signal, TextField, div, icon, text};
use shadcn::{
    Button, ButtonSize, ButtonVariant, InputGroupAlign, InputGroupButtonSize, Orientation, button, button_group,
    button_group_of_groups, button_group_separator, input, input_group, input_group_addon, input_group_button,
    input_group_input, select, select_content, select_item, theme,
};

use crate::{Render, Shot, demo, page};

const H: Orientation = Orientation::Horizontal;

/// ButtonGroupWithSelectDemo's currencies: the symbol (its value) and name.
const CURRENCIES: [(&str, &str); 3] = [("$", "US Dollar"), ("€", "Euro"), ("£", "British Pound")];

fn outline(label: &str, size: ButtonSize) -> Button {
    button(ButtonVariant::Outline, size).child(label.to_owned())
}

/// `9999rem`, the radius of the input group demo.
const PILL: f32 = 9999.0 * 16.0;

fn outline_icon(i: Icon, size: ButtonSize) -> Button {
    button(ButtonVariant::Outline, size).child(icon(i))
}

pub fn render() -> Render {
    let search = TextField::new("");
    let message = TextField::new("");
    let amount = TextField::new("");
    let currency = Signal::labeled("currency", Some("$".to_owned()));
    Box::new(move || {
        let t = theme();
        let currencies =
            select_content()
                .min_width(96.0)
                .children(CURRENCIES.map(|(symbol, name)| {
                    select_item(symbol, symbol).child(text(name).text_color(t.muted_foreground))
                }));
        page(
            "Button Group",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "ButtonGroupDemo.vue",
                    button_group_of_groups(
                        H,
                        [
                            button_group(H, [outline_icon(Icon::ArrowLeft, ButtonSize::Icon)]),
                            button_group(
                                H,
                                [
                                    outline("Archive", ButtonSize::Default),
                                    outline("Report", ButtonSize::Default),
                                ],
                            ),
                            button_group(
                                H,
                                [
                                    outline("Snooze", ButtonSize::Default),
                                    outline_icon(Icon::Ellipsis, ButtonSize::Icon),
                                ],
                            ),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupSizeDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Start)
                        .gap(32.0)
                        .child(button_group(
                            H,
                            [
                                outline("Small", ButtonSize::Sm),
                                outline("Button", ButtonSize::Sm),
                                outline("Group", ButtonSize::Sm),
                                outline_icon(Icon::Plus, ButtonSize::IconSm),
                            ],
                        ))
                        .child(button_group(
                            H,
                            [
                                outline("Default", ButtonSize::Default),
                                outline("Button", ButtonSize::Default),
                                outline("Group", ButtonSize::Default),
                                outline_icon(Icon::Plus, ButtonSize::Default),
                            ],
                        ))
                        .child(button_group(
                            H,
                            [
                                outline("Large", ButtonSize::Lg),
                                outline("Button", ButtonSize::Lg),
                                outline("Group", ButtonSize::Lg),
                                outline_icon(Icon::Plus, ButtonSize::IconLg),
                            ],
                        )),
                ))
                .child(demo(
                    "ButtonGroupNestedDemo.vue",
                    button_group_of_groups(
                        H,
                        [
                            button_group(H, ["1", "2", "3", "4", "5"].map(|n| outline(n, ButtonSize::Sm))),
                            button_group(
                                H,
                                [
                                    outline_icon(Icon::ArrowLeft, ButtonSize::IconSm),
                                    outline_icon(Icon::ArrowRight, ButtonSize::IconSm),
                                ],
                            ),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupOrientationDemo.vue",
                    button_group(
                        Orientation::Vertical,
                        [
                            outline_icon(Icon::Plus, ButtonSize::Icon),
                            outline_icon(Icon::Minus, ButtonSize::Icon),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupSeparatorDemo.vue",
                    button_group(
                        H,
                        [
                            button(ButtonVariant::Secondary, ButtonSize::Sm)
                                .child("Copy")
                                .into_element(),
                            button_group_separator(Orientation::Vertical).into_element(),
                            button(ButtonVariant::Secondary, ButtonSize::Sm)
                                .child("Paste")
                                .into_element(),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupSplitDemo.vue",
                    button_group(
                        H,
                        [
                            button(ButtonVariant::Secondary, ButtonSize::Default)
                                .child("Button")
                                .into_element(),
                            button_group_separator(Orientation::Vertical).into_element(),
                            button(ButtonVariant::Secondary, ButtonSize::Icon)
                                .child(icon(Icon::Plus))
                                .into_element(),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupWithInputDemo.vue",
                    button_group(
                        H,
                        [
                            input().field(search).placeholder("Search...").into_element(),
                            outline_icon(Icon::Search, ButtonSize::Default).into_element(),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupWithPopoverDemo.vue",
                    button_group(
                        H,
                        [
                            button(ButtonVariant::Outline, ButtonSize::Default)
                                .child(icon(Icon::Bot))
                                .child("Copilot"),
                            outline_icon(Icon::ChevronDown, ButtonSize::Icon),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupWithDropdownMenuDemo.vue",
                    button_group(
                        H,
                        [
                            outline("Follow", ButtonSize::Default),
                            outline_icon(Icon::ChevronDown, ButtonSize::Icon),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupInputGroupDemo.vue",
                    // `--radius: 9999rem` on the group: the button and the
                    // input group (`rounded-md`) and the input group's button
                    // (`rounded-[calc(var(--radius)-5px)]`) follow it.
                    button_group_of_groups(
                        H,
                        [
                            button_group(H, [outline_icon(Icon::Plus, ButtonSize::Icon).radius(PILL * 0.8)]),
                            button_group(
                                H,
                                [
                                    input_group(input_group_input(&message).placeholder("Send a message..."))
                                        .radius(PILL * 0.8)
                                        .addon(
                                            input_group_addon(InputGroupAlign::InlineEnd).button(
                                                input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                                                    .radius(PILL - 5.0)
                                                    .child(icon(Icon::AudioLines)),
                                            ),
                                        ),
                                ],
                            )
                            .flex(Flex::grow(1.0)),
                        ],
                    ),
                ))
                .child(demo(
                    "ButtonGroupWithSelectDemo.vue",
                    button_group_of_groups(
                        H,
                        [
                            button_group(
                                H,
                                [
                                    // The trigger's slot is the value itself, not a `SelectValue`.
                                    select()
                                        .model(currency)
                                        .child(text(currency.get().unwrap_or_default()))
                                        .child(currencies)
                                        .font_family(FontFamily::GEIST_MONO)
                                        .width(56.0)
                                        .into_element(),
                                    input().field(amount).placeholder("10.00").into_element(),
                                ],
                            ),
                            button_group(H, [outline_icon(Icon::ArrowRight, ButtonSize::Icon)]),
                        ],
                    ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Report")?;
    s.hover(x, y);
    s.frame("hover report")?;
    let (x, y) = s.locate("Search...")?;
    s.click(x, y);
    s.type_text("shadcn");
    s.frame("input focused")?;
    Ok(())
}
