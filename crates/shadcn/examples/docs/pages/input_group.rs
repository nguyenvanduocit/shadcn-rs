use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, TextField, div, icon, text};
use shadcn::{
    ButtonVariant, InputGroupAlign, InputGroupButtonSize, Orientation, button_group, button_group_text, input_group,
    input_group_addon, input_group_button, input_group_input, input_group_text, input_group_textarea, label, separator,
    spinner, spinner_of, theme,
};

use crate::{Render, Shot, demo, page};

const START: InputGroupAlign = InputGroupAlign::InlineStart;
const END: InputGroupAlign = InputGroupAlign::InlineEnd;
const ABOVE: InputGroupAlign = InputGroupAlign::BlockStart;
const BELOW: InputGroupAlign = InputGroupAlign::BlockEnd;

pub fn render() -> Render {
    // One text field per control on the page, made once.
    let f: [TextField; 29] = std::array::from_fn(|_| TextField::new(""));
    let copied = Signal::labeled("copied", false);
    let favorite = Signal::labeled("favorite", false);
    Box::new(move || {
        let t = theme();
        let ghost_icon =
            |i: Icon| input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs).child(icon(i));
        let copy = copied;
        let fav = favorite;
        page(
            "Input Group",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("InputGroupDemo.vue", overview(&f)))
                .child(demo(
                    "InputGroupWithIcon.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(24.0)
                        .child(
                            input_group(input_group_input(&f[4]).placeholder("Search..."))
                                .addon(input_group_addon(START).child(icon(Icon::Search))),
                        )
                        .child(
                            input_group(input_group_input(&f[5]).placeholder("Enter your email"))
                                .addon(input_group_addon(START).child(icon(Icon::Mail))),
                        )
                        .child(
                            input_group(input_group_input(&f[6]).placeholder("Card number"))
                                .addon(input_group_addon(START).child(icon(Icon::CreditCard)))
                                .addon(input_group_addon(END).child(icon(Icon::Check))),
                        )
                        .child(
                            input_group(input_group_input(&f[7]).placeholder("Card number"))
                                .addon(input_group_addon(END).child(icon(Icon::Star)).child(icon(Icon::Info))),
                        ),
                ))
                .child(demo(
                    "InputGroupWithText.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(24.0)
                        .child(
                            input_group(input_group_input(&f[8]).placeholder("0.00"))
                                .addon(input_group_addon(START).child(input_group_text().child("$")))
                                .addon(input_group_addon(END).child(input_group_text().child("USD"))),
                        )
                        .child(
                            input_group(input_group_input(&f[10]).placeholder("example.com").padding_left(2.0))
                                .addon(input_group_addon(START).child(input_group_text().child("https://")))
                                .addon(input_group_addon(END).child(input_group_text().child(".com"))),
                        )
                        .child(
                            input_group(input_group_input(&f[11]).placeholder("Enter your username"))
                                .addon(input_group_addon(END).child(input_group_text().child("@company.com"))),
                        )
                        .child(
                            input_group(input_group_textarea(&f[12]).placeholder("Enter your message")).addon(
                                input_group_addon(BELOW).child(
                                    input_group_text()
                                        .text_size(12.0)
                                        .line_height(16.0)
                                        .text_color(t.muted_foreground)
                                        .child("120 characters left"),
                                ),
                            ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithButton.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(24.0)
                        .child(
                            input_group(input_group_input(&f[9]).placeholder("https://x.com/shadcn")).addon(
                                input_group_addon(END).button(
                                    input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                                        .on_click(move |_| copy.set(true))
                                        .child(icon(if copied.get() { Icon::Copy } else { Icon::Check })),
                                ),
                            ),
                        )
                        .child(
                            // `--radius: 9999px`: the group (`rounded-md`) and its
                            // first button (`calc(var(--radius) - 5px)`) follow it.
                            input_group(input_group_input(&f[13]))
                                .radius(9999.0 * 0.8)
                                .addon(
                                    input_group_addon(START).button(
                                        input_group_button(ButtonVariant::Secondary, InputGroupButtonSize::IconXs)
                                            .radius(9999.0 - 5.0)
                                            .child(icon(Icon::Info)),
                                    ),
                                )
                                .addon(
                                    input_group_addon(START)
                                        .text_color(t.muted_foreground)
                                        .padding_left(6.0)
                                        .child("https://"),
                                )
                                .addon(
                                    input_group_addon(END).button(
                                        input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                                            .radius(9999.0)
                                            .on_click(move |_| fav.update(|v| *v = !*v))
                                            .child(icon(Icon::Star)),
                                    ),
                                ),
                        )
                        .child(
                            input_group(input_group_input(&f[14]).placeholder("Type to search...")).addon(
                                input_group_addon(END).button(
                                    input_group_button(ButtonVariant::Secondary, InputGroupButtonSize::Xs)
                                        .child("Search"),
                                ),
                            ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithLabel.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(16.0)
                        .child(
                            input_group(input_group_input(&f[15]).placeholder("shadcn"))
                                .addon(input_group_addon(START).child(label("@"))),
                        )
                        .child(
                            input_group(input_group_input(&f[16]).placeholder("shadcn@vercel.com")).addon(
                                input_group_addon(ABOVE)
                                    .child(label("Email").text_color(t.foreground))
                                    .button(ghost_icon(Icon::Info).margin_left(Length::Auto).radius(9999.0)),
                            ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithTooltip.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(16.0)
                        .child(
                            input_group(input_group_input(&f[17]).placeholder("Enter password"))
                                .addon(input_group_addon(END).button(ghost_icon(Icon::Info))),
                        )
                        .child(
                            input_group(input_group_input(&f[18]).placeholder("Your email address"))
                                .addon(input_group_addon(END).button(ghost_icon(Icon::CircleHelp))),
                        )
                        .child(
                            input_group(input_group_input(&f[19]).placeholder("Enter API key"))
                                .addon(input_group_addon(START).button(ghost_icon(Icon::Info))),
                        ),
                ))
                .child(demo(
                    "InputGroupWithDropdown.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(16.0)
                        .child(
                            input_group(input_group_input(&f[20]).placeholder("Enter file name"))
                                .addon(input_group_addon(END).button(ghost_icon(Icon::Ellipsis))),
                        )
                        .child(
                            // `--radius: 1rem`: the group (`rounded-md`) and its
                            // button (`calc(var(--radius) - 5px)`) follow it.
                            input_group(input_group_input(&f[21]).placeholder("Enter search query"))
                                .radius(16.0 * 0.8)
                                .addon(
                                    input_group_addon(END).button(
                                        input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::Xs)
                                            .radius(16.0 - 5.0)
                                            // `!pr-1.5`: important, so it beats `has-[>svg]:px-2`
                                            // too (the `xs` size pads the same with an icon).
                                            .padding_right(6.0)
                                            .text_size(12.0)
                                            .line_height(16.0)
                                            .child("Search In...")
                                            .child(icon(Icon::ChevronDown).size(12.0)),
                                    ),
                                ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithSpinner.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(16.0)
                        .child(
                            input_group(input_group_input(&f[23]).placeholder("Searching...").disabled(true))
                                .addon(input_group_addon(END).child(spinner())),
                        )
                        .child(
                            input_group(input_group_input(&f[24]).placeholder("Processing...").disabled(true))
                                .addon(input_group_addon(START).child(spinner())),
                        )
                        .child(
                            input_group(
                                input_group_input(&f[25])
                                    .placeholder("Saving changes...")
                                    .disabled(true),
                            )
                            .addon(
                                input_group_addon(END)
                                    .child(input_group_text().child("Saving..."))
                                    .child(spinner()),
                            ),
                        )
                        .child(
                            // `<LoaderIcon class="animate-spin" />`: one linear turn per second.
                            input_group(
                                input_group_input(&f[26])
                                    .placeholder("Refreshing data...")
                                    .disabled(true),
                            )
                            .addon(input_group_addon(START).child(spinner_of(Icon::Loader)))
                            .addon(
                                input_group_addon(END).child(
                                    input_group_text()
                                        .text_color(t.muted_foreground)
                                        .child("Please wait..."),
                                ),
                            ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithTextarea.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(448.0)
                        .gap(16.0)
                        .child(
                            input_group(
                                input_group_textarea(&f[22])
                                    .placeholder("console.log('Hello, world!');")
                                    .min_height(200.0),
                            )
                            .addon(
                                input_group_addon(BELOW)
                                    .border_top(1.0)
                                    .border_color(t.border)
                                    .child(input_group_text().child("Line 1, Column 1"))
                                    .button(
                                        input_group_button(ButtonVariant::Default, InputGroupButtonSize::Sm)
                                            .margin_left(Length::Auto)
                                            .child("Run")
                                            .child(icon(Icon::CornerDownLeft)),
                                    ),
                            )
                            .addon(
                                input_group_addon(ABOVE)
                                    .border_bottom(1.0)
                                    .border_color(t.border)
                                    .child(
                                        input_group_text()
                                            .font_family(FontFamily::GEIST_MONO)
                                            .font_weight(500)
                                            .child(icon(Icon::Braces))
                                            .child(text("script.js")),
                                    )
                                    .button(ghost_icon(Icon::RefreshCw).margin_left(Length::Auto))
                                    .button(ghost_icon(Icon::Copy)),
                            ),
                        ),
                ))
                .child(demo(
                    "InputGroupWithButtonGroup.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .child(button_group(
                            Orientation::Horizontal,
                            [
                                // `ButtonGroupText as-child` on a `Label`: the label's `leading-none`.
                                button_group_text()
                                    .line_height(LineHeight::Relative(1.0))
                                    .child("https://")
                                    .into_element(),
                                input_group(input_group_input(&f[27]))
                                    .addon(input_group_addon(END).child(icon(Icon::Link2)))
                                    .into_element(),
                                button_group_text().child(".com").into_element(),
                            ],
                        )),
                ))
                .child(demo(
                    "InputGroupWithCustomInput.vue",
                    // The demo's own `<textarea data-slot="input-group-control">`:
                    // `min-h-16 px-3 py-2.5 md:text-sm`, grown to its text.
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .gap(24.0)
                        .child(
                            input_group(
                                input_group_textarea(&f[28])
                                    .placeholder("Autoresize textarea...")
                                    .padding_y(10.0),
                            )
                            .addon(
                                input_group_addon(BELOW).button(
                                    input_group_button(ButtonVariant::Default, InputGroupButtonSize::Sm)
                                        .margin_left(Length::Auto)
                                        .child("Submit"),
                                ),
                            ),
                        ),
                )),
        )
    })
}

/// `InputGroupDemo.vue`, in a function of its own to keep the page's frame small.
fn overview(f: &[TextField; 29]) -> gui::Div {
    let t = theme();
    div()
        .display(Display::Grid)
        .width(pct(100.0))
        .max_width(384.0)
        .gap(24.0)
        .child(
            input_group(input_group_input(&f[0]).placeholder("Search..."))
                .addon(input_group_addon(START).child(icon(Icon::Search)))
                .addon(input_group_addon(END).child("12 results")),
        )
        .child(
            input_group(input_group_input(&f[1]).placeholder("example.com").padding_left(4.0))
                .addon(input_group_addon(START).child(input_group_text().child("https://")))
                .addon(
                    input_group_addon(END).button(
                        input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::IconXs)
                            .radius(9999.0)
                            .child(icon(Icon::Info)),
                    ),
                ),
        )
        .child(
            input_group(input_group_textarea(&f[2]).placeholder("Ask, Search or Chat...")).addon(
                input_group_addon(BELOW)
                    .button(
                        input_group_button(ButtonVariant::Outline, InputGroupButtonSize::IconXs)
                            .radius(9999.0)
                            .child(icon(Icon::Plus)),
                    )
                    .button(input_group_button(ButtonVariant::Ghost, InputGroupButtonSize::Xs).child("Auto"))
                    .child(input_group_text().margin_left(Length::Auto).child("52% used"))
                    .child(separator(Orientation::Vertical).height(16.0))
                    .button(
                        input_group_button(ButtonVariant::Default, InputGroupButtonSize::IconXs)
                            .radius(9999.0)
                            .disabled(true)
                            .child(icon(Icon::ArrowUp)),
                    ),
            ),
        )
        .child(
            input_group(input_group_input(&f[3]).placeholder("@shadcn")).addon(
                input_group_addon(END).child(
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .justify_content(Justify::Center)
                        .radius(9999.0)
                        .background(t.primary)
                        .text_color(t.primary_foreground)
                        .size(16.0)
                        .child(icon(Icon::Check).size(12.0)),
                ),
            ),
        )
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Search...")?;
    s.click(x, y);
    s.type_text("shadcn");
    s.frame("focused ring on the group")?;
    let (x, y) = s.locate("12 results")?;
    s.click(x, y);
    s.press(NamedKey::Backspace, Modifiers::default());
    s.frame("addon click focuses the input")?;
    Ok(())
}
