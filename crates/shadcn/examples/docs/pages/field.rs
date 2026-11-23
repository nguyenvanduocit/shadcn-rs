use gui::prelude::*;
use gui::{ScenarioError, Signal, div, span};
use shadcn::{
    ButtonSize, ButtonVariant, FieldLegendVariant, FieldOrientation, FieldStack, Orientation, button, checkbox, field,
    field_checkbox_group, field_choices, field_content, field_description, field_group, field_label, field_label_card,
    field_legend, field_separator, field_set, field_title, input, radio_group, radio_group_item, select,
    select_content, select_item, select_value, slider, switch, textarea,
};

use crate::{Render, Shot, demo, page};

const V: FieldOrientation = FieldOrientation::Vertical;
const H: FieldOrientation = FieldOrientation::Horizontal;
const R: FieldOrientation = FieldOrientation::Responsive;

/// A label that flips its checkbox, as `<label for>` does.
fn label_for(name: &str, checked: &Signal<bool>) -> gui::Div {
    let flip = *checked;
    field_label(name.to_owned())
        .font_weight(400)
        .on_click(move |_| flip.update(|c| *c = !*c))
}

/// A checkbox beside its label, in a horizontal field.
fn check(name: &str, checked: &Signal<bool>) -> FieldStack {
    field(H, false)
        .child(checkbox().model(*checked))
        .child(label_for(name, checked))
}

/// A select of `values` (each its own label) with nothing chosen.
fn pick(placeholder: &str, values: &[&str]) -> impl IntoElement {
    let content = values
        .iter()
        .fold(select_content(), |c, v| c.child(select_item(*v, *v)));
    select().child(select_value(placeholder)).child(content)
}

pub fn render() -> Render {
    // One checkbox state per checkbox on the page, made once.
    let b: [Signal<bool>; 10] = std::array::from_fn(|i| Signal::new(matches!(i, 0 | 1 | 6)));
    let compute = Signal::labeled("compute", Some("kubernetes".to_owned()));
    let price = Signal::labeled("price", vec![200.0_f64, 800.0]);
    Box::new(move || {
        let months = ["01", "02", "03", "04", "05", "06", "07", "08", "09", "10", "11", "12"];
        let years = ["2024", "2025", "2026", "2027", "2028", "2029"];
        let field_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_group()
                .child(
                    field_set()
                        .child(field_legend("Payment Method", None))
                        .child(field_description("All transactions are secure and encrypted"))
                        .child(
                            field_group()
                                .child(
                                    field(V, false)
                                        .child(field_label("Name on Card"))
                                        .child(input().placeholder("Evil Rabbit")),
                                )
                                .child(
                                    field(V, false)
                                        .child(field_label("Card Number"))
                                        .child(input().placeholder("1234 5678 9012 3456"))
                                        .child(field_description("Enter your 16-digit card number")),
                                )
                                .child(
                                    div()
                                        .display(Display::Grid)
                                        .grid_columns(Tracks::repeat(3, Track::Fr(1.0)))
                                        .gap(16.0)
                                        .child(field(V, false).child(field_label("Month")).child(pick("MM", &months)))
                                        .child(field(V, false).child(field_label("Year")).child(pick("YYYY", &years)))
                                        .child(
                                            field(V, false)
                                                .child(field_label("CVV"))
                                                .child(input().placeholder("123")),
                                        ),
                                ),
                        ),
                )
                .child(field_separator(None))
                .child(
                    field_set()
                        .child(field_legend("Billing Address", None))
                        .child(field_description(
                            "The billing address associated with your payment method",
                        ))
                        .child(field_group().child(check("Same as shipping address", &b[0]))),
                )
                .child(
                    field_set().child(
                        field_group().child(
                            field(V, false)
                                .child(field_label("Comments"))
                                .child(textarea().placeholder("Add any additional comments")),
                        ),
                    ),
                )
                .child(
                    field(H, false)
                        .child(button(ButtonVariant::Default, ButtonSize::Default).child("Submit"))
                        .child(button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel")),
                ),
        );

        let input_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_set().child(
                field_group()
                    .child(
                        field(V, false)
                            .child(field_label("Username"))
                            .child(input().placeholder("Max Leiter"))
                            .child(field_description("Choose a unique username for your account.")),
                    )
                    .child(
                        field(V, false)
                            .child(field_label("Password"))
                            .child(field_description("Must be at least 8 characters long."))
                            .child(input().placeholder("********")),
                    ),
            ),
        );

        let fieldset_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_set()
                .child(field_legend("Address Information", None))
                .child(field_description("We need your address to deliver your order."))
                .child(
                    field_group()
                        .child(
                            field(V, false)
                                .child(field_label("Street Address"))
                                .child(input().placeholder("123 Main St")),
                        )
                        .child(
                            div()
                                .display(Display::Grid)
                                .grid_columns(Tracks::repeat(2, Track::Fr(1.0)))
                                .gap(16.0)
                                .child(
                                    field(V, false)
                                        .child(field_label("City"))
                                        .child(input().placeholder("New York")),
                                )
                                .child(
                                    field(V, false)
                                        .child(field_label("Postal Code"))
                                        .child(input().placeholder("90502")),
                                ),
                        ),
                ),
        );

        let group_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_group()
                .child(
                    field_set()
                        .child(field_label("Responses"))
                        .child(field_description(
                            "Get notified when ChatGPT responds to requests that take time, like research or image generation.",
                        ))
                        .child(
                            field_checkbox_group().child(
                                field(H, false)
                                    .child(checkbox().model(b[1]).disabled(true))
                                    .child(field_label("Push notifications").font_weight(400)),
                            ),
                        ),
                )
                .child(field_separator(None))
                .child(
                    field_set()
                        .child(field_label("Tasks"))
                        .child(
                            field_description("Get notified when tasks you've created have updates. ")
                                .link("Manage tasks"),
                        )
                        .child(
                            field_checkbox_group()
                                .child(check("Push notifications", &b[2]))
                                .child(check("Email notifications", &b[3])),
                        ),
                ),
        );

        let chosen = compute.get();
        let card = |value: &'static str, title: &str, description: &str| {
            let choose = compute;
            field_label_card(
                field(H, false)
                    .child(
                        field_content()
                            .child(field_title(title))
                            .child(field_description(description)),
                    )
                    .child(radio_group_item(value)),
                chosen.as_deref() == Some(value),
            )
            .on_click(move |_| choose.set(Some(value.to_owned())))
        };
        let choice_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_group().child(
                field_set()
                    .child(field_label("Compute Environment"))
                    .child(field_description("Select the compute environment for your cluster."))
                    .child(field_choices(
                        radio_group()
                            .model(compute)
                            .child(card(
                                "kubernetes",
                                "Kubernetes",
                                "Run GPU workloads on a K8s configured cluster.",
                            ))
                            .child(card(
                                "vm",
                                "Virtual Machine",
                                "Access a VM configured cluster to run GPU workloads.",
                            )),
                    )),
            ),
        );

        let checkbox_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_group()
                .child(
                    field_set()
                        .child(field_legend("Show these items on the desktop", Some(FieldLegendVariant::Label)))
                        .child(field_description("Select the items you want to show on the desktop."))
                        .child(
                            field_group()
                                .gap(12.0)
                                .child(check("Hard disks", &b[4]))
                                .child(check("External disks", &b[5]))
                                .child(check("CDs, DVDs, and iPods", &b[8]))
                                .child(check("Connected servers", &b[9])),
                        ),
                )
                .child(field_separator(None))
                .child(
                    field(H, false).child(checkbox().model(b[6])).child(
                        field_content()
                            .child(field_label("Sync Desktop & Documents folders"))
                            .child(field_description(
                                "Your Desktop & Documents folders are being synced with iCloud Drive. You can access them from other devices.",
                            )),
                    ),
                ),
        );

        let radio = |value: &str, name: &str| {
            field(H, false)
                .child(radio_group_item(value))
                .child(field_label(name.to_owned()).font_weight(400))
        };
        let radio_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_set()
                .child(field_label("Subscription Plan"))
                .child(field_description(
                    "Yearly and lifetime plans offer significant savings.",
                ))
                .child(field_choices(
                    radio_group()
                        .child(radio("monthly", "Monthly ($9.99/month)"))
                        .child(radio("yearly", "Yearly ($99.99/year)"))
                        .child(radio("lifetime", "Lifetime ($299.99)")),
                )),
        );

        let switch_demo = div().width(pct(100.0)).max_width(448.0).child(
            field(H, false)
                .child(field_content().child(field_label("Multi-factor authentication")).child(field_description(
                    "Enable multi-factor authentication. If you do not have a two-factor device, you can use a one-time code sent to your email.",
                )))
                .child(switch()),
        );

        let textarea_demo = div().width(pct(100.0)).max_width(448.0).child(
            field_set().child(
                field_group().child(
                    field(V, false)
                        .child(field_label("Feedback"))
                        .child(textarea().placeholder("Your feedback helps us improve..."))
                        .child(field_description("Share your thoughts about our service.")),
                ),
            ),
        );

        let responsive_demo = div().width(pct(100.0)).max_width(896.0).child(
            field_set()
                .child(field_legend("Profile", None))
                .child(field_description("Fill in your profile information."))
                .child(field_separator(None))
                .child(
                    field_group()
                        .child(
                            field(R, false)
                                .child(
                                    field_content()
                                        .child(field_label("Name"))
                                        .child(field_description("Provide your full name for identification")),
                                )
                                .child(input().placeholder("Evil Rabbit")),
                        )
                        .child(field_separator(None))
                        .child(
                            field(R, false)
                                .child(field_content().child(field_label("Message")).child(field_description(
                                    "You can write your message here. Keep it short, preferably under 100 characters.",
                                )))
                                .child(
                                    textarea()
                                        .placeholder("Hello, world!")
                                        .min_height(100.0)
                                        .min_width(300.0),
                                ),
                        )
                        .child(field_separator(None))
                        .child(
                            field(R, false)
                                .child(button(ButtonVariant::Default, ButtonSize::Default).child("Submit"))
                                .child(button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel")),
                        ),
                ),
        );

        let range = price.get();
        let at = |i: usize| range.get(i).copied().unwrap_or_default().to_string();
        let slider_demo = div().width(pct(100.0)).max_width(448.0).child(
            field(V, false)
                .child(field_title("Price Range"))
                .child(
                    field_description("Set your budget range ($ ")
                        .span(span(at(0)).font_weight(500).tabular_nums(true))
                        .span(" - ")
                        .span(span(at(1)).font_weight(500).tabular_nums(true))
                        .span(")."),
                )
                .child(
                    slider(Orientation::Horizontal)
                        .model(price)
                        .max(1000.0)
                        .step(10.0)
                        .margin_top(8.0)
                        .width(pct(100.0)),
                ),
        );

        let departments = [
            "Engineering",
            "Design",
            "Marketing",
            "Sales",
            "Customer Support",
            "Human Resources",
            "Finance",
            "Operations",
        ];
        let select_demo = div().width(pct(100.0)).max_width(448.0).child(
            field(V, false)
                .child(field_label("Department"))
                .child(pick("Choose department", &departments))
                .child(field_description("Select your department or area of work.")),
        );

        page(
            "Field",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("FieldDemo.vue", field_demo))
                .child(demo("FieldInputDemo.vue", input_demo))
                .child(demo("FieldFieldsetDemo.vue", fieldset_demo))
                .child(demo("FieldGroupDemo.vue", group_demo))
                .child(demo("FieldChoiceCardDemo.vue", choice_demo))
                .child(demo("FieldCheckboxDemo.vue", checkbox_demo))
                .child(demo("FieldRadioDemo.vue", radio_demo))
                .child(demo("FieldSwitchDemo.vue", switch_demo))
                .child(demo("FieldTextareaDemo.vue", textarea_demo))
                .child(demo("FieldSliderDemo.vue", slider_demo))
                .child(demo("FieldSelectDemo.vue", select_demo))
                .child(demo("FieldResponsiveDemo.vue", responsive_demo)),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Evil Rabbit")?;
    s.click(x, y);
    s.type_text("Nguyen Van A");
    s.click_text("Same as shipping address")?;
    s.frame("filled")?;
    Ok(())
}
