use gui::prelude::*;
use gui::{Icon, ScenarioError, TextField, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, InputGroupAlign, InputGroupButtonSize, Orientation, SelectSize, SelectState, button,
    button_group, button_group_of_groups, button_group_separator, input, input_group, input_group_addon,
    input_group_button, input_group_input, select_content, select_item, select_trigger_showing, theme,
};

use crate::{Render, Shot, demo, page};

const H: Orientation = Orientation::Horizontal;

/// ButtonGroupWithSelectDemo's currencies: the symbol (its value) and name.
const CURRENCIES: [(&str, &str); 3] = [("$", "US Dollar"), ("€", "Euro"), ("£", "British Pound")];

fn outline(label: &str, size: ButtonSize) -> gui::Div {
    button(ButtonVariant::Outline, size).child(label.to_owned())
}

fn outline_icon(i: Icon, size: ButtonSize) -> gui::Div {
    button(ButtonVariant::Outline, size).child(icon(i))
}

pub fn render() -> Render {
    let search = TextField::new("");
    let message = TextField::new("");
    let amount = TextField::new("");
    let currency = SelectState::new(Some("$"));
    Box::new(move || {
        let t = theme();
        let currencies = select_content(&currency)
            .min_w(24.0)
            .children(CURRENCIES.map(|(symbol, name)| {
                select_item(&currency, symbol, symbol, false).child(text(name).text_color(t.muted_foreground))
            }));
        page(
            "Button Group",
            div()
                .flex_col()
                .gap(8.0)
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
                        .flex_col()
                        .items_start()
                        .gap(8.0)
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
                            input(&search).placeholder("Search...").into_element(),
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
                    button_group_of_groups(
                        H,
                        [
                            button_group(H, [outline_icon(Icon::Plus, ButtonSize::Icon)]),
                            button_group(
                                H,
                                [
                                    input_group(input_group_input(&message).placeholder("Send a message...")).addon(
                                        input_group_addon(InputGroupAlign::InlineEnd).button(
                                            input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                                                .child(icon(Icon::AudioLines)),
                                        ),
                                    ),
                                ],
                            )
                            .flex_1(),
                        ],
                    )
                    .radius_var(9999.0 * 16.0),
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
                                    select_trigger_showing(
                                        &currency,
                                        text(currency.value.get().unwrap_or_default()),
                                        SelectSize::Default,
                                        currencies,
                                    )
                                    .font_mono()
                                    .w(14.0)
                                    .into_element(),
                                    input(&amount).placeholder("10.00").into_element(),
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
