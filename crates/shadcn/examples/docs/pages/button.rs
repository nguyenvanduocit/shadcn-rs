use gui::prelude::*;
use gui::{Icon, ScenarioError, Signal, div, icon, text};
use shadcn::{ButtonSize, ButtonVariant, button, spinner};

use crate::{Render, Shot, demo, page, section};

pub fn render() -> Render {
    let clicks = Signal::labeled("clicks", 0_u32);
    Box::new(move || {
        let variants = [
            ("Default", ButtonVariant::Default),
            ("Secondary", ButtonVariant::Secondary),
            ("Destructive", ButtonVariant::Destructive),
            ("Outline", ButtonVariant::Outline),
            ("Ghost", ButtonVariant::Ghost),
            ("Link", ButtonVariant::Link),
        ];
        let sizes = [
            ("Xs", ButtonSize::Xs),
            ("Small", ButtonSize::Sm),
            ("Default", ButtonSize::Default),
            ("Large", ButtonSize::Lg),
        ];
        let c = clicks.clone();
        page(
            "Button",
            div()
                .flex_col()
                .gap(8.0)
                .child(section(
                    "Variants",
                    div().flex().flex_wrap().gap(2.0).children(variants.map(|(name, v)| {
                        let c = c.clone();
                        button(v, ButtonSize::Default)
                            .child(name)
                            .on_click(move |_| c.update(|n| n.saturating_add(1)))
                    })),
                ))
                .child(section(
                    "Sizes",
                    div()
                        .flex()
                        .items_center()
                        .gap(2.0)
                        .children(sizes.map(|(name, s)| button(ButtonVariant::Outline, s).child(name))),
                ))
                .child(section(
                    "With icons",
                    div()
                        .flex()
                        .items_center()
                        .gap(2.0)
                        .child(
                            button(ButtonVariant::Default, ButtonSize::Default)
                                .child(icon(Icon::Mail))
                                .child("Login with Email"),
                        )
                        .child(button(ButtonVariant::Outline, ButtonSize::Icon).child(icon(Icon::ChevronRight)))
                        .child(button(ButtonVariant::Secondary, ButtonSize::IconSm).child(icon(Icon::Plus)))
                        .child(
                            button(ButtonVariant::Outline, ButtonSize::Sm)
                                .disabled(true)
                                .child(spinner())
                                .child("Please wait"),
                        ),
                ))
                .child(text(format!("Clicked {} times", clicks.get())).text_sm())
                .child(section("shadcn-vue demos", demos())),
        )
    })
}

/// shadcn-vue's button demos, one for one, for the verdict.
fn demos() -> gui::Div {
    div()
        .flex_col()
        .gap(4.0)
        .child(demo(
            "ButtonDemo.vue",
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(2.0)
                .child(button(ButtonVariant::Outline, ButtonSize::Default).child("Button"))
                .child(button(ButtonVariant::Outline, ButtonSize::Icon).child(icon(Icon::ArrowUp))),
        ))
        .children(
            [
                ("ButtonDefault.vue", "Default", ButtonVariant::Default),
                ("ButtonSecondary.vue", "Secondary", ButtonVariant::Secondary),
                ("ButtonDestructive.vue", "Destructive", ButtonVariant::Destructive),
                ("ButtonOutline.vue", "Outline", ButtonVariant::Outline),
                ("ButtonGhost.vue", "Ghost", ButtonVariant::Ghost),
                ("ButtonLink.vue", "Link", ButtonVariant::Link),
            ]
            .map(|(file, label, v)| demo(file, button(v, ButtonSize::Default).child(label))),
        )
        .child(demo(
            "ButtonWithIcon.vue",
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .child(icon(Icon::GitBranch))
                .child("New Branch"),
        ))
        .child(demo(
            "ButtonIcon.vue",
            button(ButtonVariant::Outline, ButtonSize::Icon).child(icon(Icon::CircleFadingArrowUp)),
        ))
        .child(demo(
            "ButtonLoading.vue",
            button(ButtonVariant::Outline, ButtonSize::Sm)
                .disabled(true)
                .child(spinner())
                .child("Submit"),
        ))
        .child(demo(
            "ButtonRounded.vue",
            div().flex_col().gap(8.0).child(
                button(ButtonVariant::Outline, ButtonSize::Icon)
                    .rounded_full()
                    .child(icon(Icon::ArrowUp)),
            ),
        ))
        .child(demo(
            "ButtonSize.vue",
            // `flex-col sm:flex-row`: a row at the gallery's width.
            div().flex().items_start().gap(8.0).children(
                [
                    ("Small", ButtonSize::Sm, ButtonSize::IconSm),
                    ("Default", ButtonSize::Default, ButtonSize::Icon),
                    ("Large", ButtonSize::Lg, ButtonSize::IconLg),
                ]
                .map(|(label, size, icon_size)| {
                    div()
                        .flex()
                        .items_start()
                        .gap(2.0)
                        .child(button(ButtonVariant::Outline, size).child(label))
                        .child(button(ButtonVariant::Outline, icon_size).child(icon(Icon::ArrowUpRight)))
                }),
            ),
        ))
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Outline")?;
    s.hover(x, y);
    s.frame("hover outline")?;
    s.click_text("Secondary")?;
    s.frame("clicked")?;
    s.press(gui::NamedKey::Tab, gui::Modifiers::default());
    s.frame("tab focus")?;
    Ok(())
}
