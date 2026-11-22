use gui::prelude::*;
use gui::{Icon, ScenarioError, Track, div, text};
use shadcn::{AlertVariant, alert, alert_description, alert_title};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    Box::new(|| {
        let destructive = AlertVariant::Destructive;
        page(
            "Alert",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "AlertDemo.vue",
                    div()
                        .display(Display::Grid)
                        .width(pct(100.0))
                        .max_width(576.0)
                        .align_items(Align::Start)
                        .gap(16.0)
                        .child(
                            alert(AlertVariant::Default, Some(Icon::CircleCheck))
                                .child(alert_title("Success! Your changes have been saved"))
                                .child(
                                    alert_description(AlertVariant::Default)
                                        .child("This is an alert with icon, title and description."),
                                ),
                        )
                        .child(
                            alert(AlertVariant::Default, Some(Icon::Popcorn))
                                .child(alert_title("This Alert has a title and an icon. No description.")),
                        )
                        .child(
                            alert(destructive, Some(Icon::CircleAlert))
                                .child(alert_title("Unable to process your payment."))
                                .child(
                                    alert_description(destructive)
                                        .child(
                                            text("Please verify your billing information and try again.")
                                                .line_height(LineHeight::Relative(1.625)),
                                        )
                                        .child(
                                            div()
                                                .margin_top(8.0)
                                                .display(Display::Flex)
                                                .flex_direction(FlexDirection::Column)
                                                .gap(4.0)
                                                .children(
                                                    [
                                                        "Check your card details",
                                                        "Ensure sufficient funds",
                                                        "Verify billing address",
                                                    ]
                                                    .map(|item| {
                                                        // `list-inside list-disc`: Chrome sets the text 19.5px
                                                        // after the marker's left edge (measured on its render).
                                                        div()
                                                            .display(Display::Grid)
                                                            .grid_columns(vec![Track::Px(19.5), Track::Fr(1.0)])
                                                            .child("•")
                                                            .child(item)
                                                    }),
                                                ),
                                        ),
                                ),
                        ),
                ))
                .child(demo(
                    "AlertDestructive.vue",
                    alert(destructive, Some(Icon::CircleAlert))
                        .child(alert_title("Error"))
                        .child(alert_description(destructive).child("Your session has expired. Please log in again.")),
                ))
                .child(section(
                    "Without an icon",
                    div().width(576.0).child(
                        alert(AlertVariant::Default, None)
                            .child(alert_title("Heads up!"))
                            .child(
                                alert_description(AlertVariant::Default)
                                    .child("An alert with no icon keeps its text in the second column."),
                            ),
                    ),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    crate::still(s)
}
