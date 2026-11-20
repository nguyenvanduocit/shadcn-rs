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
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "BadgeDemo.vue",
                    div()
                        .flex_col()
                        .items_center()
                        .gap(2.0)
                        .child(
                            div()
                                .flex()
                                .w_full()
                                .flex_wrap()
                                .gap(2.0)
                                .child(badge(BadgeVariant::Default).child("Badge"))
                                .child(badge(BadgeVariant::Secondary).child("Secondary"))
                                .child(badge(BadgeVariant::Destructive).child("Destructive"))
                                .child(badge(BadgeVariant::Outline).child("Outline")),
                        )
                        .child(
                            div()
                                .flex()
                                .w_full()
                                .flex_wrap()
                                .gap(2.0)
                                // bg-blue-500 text-white dark:bg-blue-600.
                                .child(
                                    badge(BadgeVariant::Secondary)
                                        .bg(if t.dark {
                                            Color::oklch(0.546, 0.245, 262.881)
                                        } else {
                                            Color::oklch(0.623, 0.214, 259.815)
                                        })
                                        .text_color(Color::WHITE)
                                        .child(icon(Icon::BadgeCheck).size(3.0))
                                        .child("Verified"),
                                )
                                .children(
                                    [
                                        ("8", BadgeVariant::Default),
                                        ("99", BadgeVariant::Destructive),
                                        ("20+", BadgeVariant::Outline),
                                    ]
                                    .map(|(n, v)| {
                                        badge(v).h(5.0).min_w(5.0).rounded_full().px(1.0).font_mono().child(n)
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
