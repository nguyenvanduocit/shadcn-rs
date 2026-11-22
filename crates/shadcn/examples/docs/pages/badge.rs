use gui::prelude::*;
use gui::{Color, Icon, div, icon};
use shadcn::{BadgeVariant, badge, theme};

use crate::{Render, demo, page};

pub fn render() -> Render {
    Box::new(|| {
        let t = theme();
        page(
            "Badge",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "BadgeDemo.vue",
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .child(
                            div()
                                .display(Display::Flex)
                                .width(pct(100.0))
                                .flex_wrap(true)
                                .gap(8.0)
                                .child(badge(BadgeVariant::Default).child("Badge"))
                                .child(badge(BadgeVariant::Secondary).child("Secondary"))
                                .child(badge(BadgeVariant::Destructive).child("Destructive"))
                                .child(badge(BadgeVariant::Outline).child("Outline")),
                        )
                        .child(
                            div()
                                .display(Display::Flex)
                                .width(pct(100.0))
                                .flex_wrap(true)
                                .gap(8.0)
                                // bg-blue-500 text-white dark:bg-blue-600.
                                .child(
                                    badge(BadgeVariant::Secondary)
                                        .background(if t.dark {
                                            Color::oklch(0.546, 0.245, 262.881)
                                        } else {
                                            Color::oklch(0.623, 0.214, 259.815)
                                        })
                                        .text_color(Color::WHITE)
                                        .child(icon(Icon::BadgeCheck).size(12.0))
                                        .child("Verified"),
                                )
                                .children(
                                    [
                                        ("8", BadgeVariant::Default),
                                        ("99", BadgeVariant::Destructive),
                                        ("20+", BadgeVariant::Outline),
                                    ]
                                    .map(|(n, v)| {
                                        badge(v)
                                            .height(20.0)
                                            .min_width(20.0)
                                            .radius(9999.0)
                                            .padding_x(4.0)
                                            .font_family(FontFamily::GEIST_MONO)
                                            .child(n)
                                    }),
                                ),
                        ),
                ))
                .children(
                    [
                        ("BadgeSecondary.vue", "Secondary", BadgeVariant::Secondary),
                        ("BadgeDestructive.vue", "Destructive", BadgeVariant::Destructive),
                        ("BadgeOutline.vue", "Outline", BadgeVariant::Outline),
                    ]
                    .map(|(file, label, v)| demo(file, badge(v).child(label))),
                ),
        )
    })
}

pub fn script(s: &mut crate::Shot) -> Result<(), gui::ScenarioError> {
    crate::still(s)
}
