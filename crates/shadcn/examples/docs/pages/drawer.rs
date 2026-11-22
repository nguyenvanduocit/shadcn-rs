use gui::prelude::*;
use gui::{Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, DrawerContentProps, DrawerDirection, Part, button, drawer, drawer_close, drawer_content,
    drawer_description, drawer_footer, drawer_header, drawer_title, drawer_trigger, theme,
};

use crate::{Render, Shot, demo, motion_frames, page};

const LOREM: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Sed do eiusmod tempor incididunt ut \
                     labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris \
                     nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit \
                     esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt \
                     in culpa qui officia deserunt mollit anim id est laborum.";

/// `DrawerSides`: each edge with the direction that dismisses it.
const SIDES: [(&str, DrawerDirection); 4] = [
    ("Top", DrawerDirection::Up),
    ("Right", DrawerDirection::Right),
    ("Bottom", DrawerDirection::Down),
    ("Left", DrawerDirection::Left),
];

/// What `DrawerSides` and `DrawerScrollableContent` put in the drawer: a
/// header, ten paragraphs scrolling between it and the footer, and the footer.
fn scrolling(content: Part<DrawerContentProps>) -> Part<DrawerContentProps> {
    content
        .child(
            drawer_header()
                .child(drawer_title("Move Goal"))
                .child(drawer_description("Set your daily activity goal.")),
        )
        .child(
            div()
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .padding_x(16.0)
                .children((0..10).map(|_| text(LOREM).margin_bottom(16.0).line_height(LineHeight::Relative(1.5)))),
        )
        .child(
            drawer_footer()
                .child(button(ButtonVariant::Default, ButtonSize::Default).child("Submit"))
                .child(drawer_close(
                    button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel"),
                )),
        )
}

pub fn render() -> Render {
    let goal = Signal::labeled("goal", 350_i32);
    Box::new(move || {
        let t = theme();
        let step = |delta: i32, glyph: Icon, disabled: bool| {
            button(ButtonVariant::Outline, ButtonSize::Icon)
                .height(32.0)
                .width(32.0)
                .flex_shrink(0.0)
                .radius(9999.0)
                .disabled(disabled)
                .on_click(move |_| goal.update(|g| *g = g.saturating_add(delta)))
                .child(icon(glyph))
        };
        let g = goal.get();
        let goal_demo = drawer()
            .child(drawer_trigger(
                button(ButtonVariant::Outline, ButtonSize::Default).child("Open Drawer"),
            ))
            .child(
                drawer_content(DrawerDirection::Down).child(
                    div()
                        .margin_x(Length::Auto)
                        .width(pct(100.0))
                        .max_width(384.0)
                        .child(
                            drawer_header()
                                .child(drawer_title("Move Goal"))
                                .child(drawer_description("Set your daily activity goal.")),
                        )
                        .child(
                            div()
                                .padding(16.0)
                                .padding_bottom(0.0)
                                .child(
                                    div()
                                        .display(Display::Flex)
                                        .align_items(Align::Center)
                                        .justify_content(Justify::Center)
                                        .gap(8.0)
                                        .child(step(-10, Icon::Minus, g <= 200))
                                        .child(
                                            div()
                                                .flex(Flex::grow(1.0))
                                                .text_align(TextAlign::Center)
                                                .child(
                                                    text(g.to_string())
                                                        .text_size(72.0)
                                                        .line_height(72.0)
                                                        .font_weight(700)
                                                        .tracking(-0.025),
                                                )
                                                .child(
                                                    text("CALORIES/DAY")
                                                        .text_size(11.2)
                                                        .line_height(16.0)
                                                        .text_color(t.muted_foreground),
                                                ),
                                        )
                                        .child(step(10, Icon::Plus, g >= 400)),
                                )
                                .child(div().margin_top(12.0).height(120.0)),
                        )
                        .child(
                            drawer_footer()
                                .child(button(ButtonVariant::Default, ButtonSize::Default).child("Submit"))
                                .child(drawer_close(
                                    button(ButtonVariant::Outline, ButtonSize::Default).child("Cancel"),
                                )),
                        ),
                ),
            );
        let sides_demo = div()
            .display(Display::Flex)
            .flex_wrap(true)
            .gap(8.0)
            .children(SIDES.iter().map(|(name, dir)| {
                let vertical = matches!(dir, DrawerDirection::Up | DrawerDirection::Down);
                drawer()
                    .child(drawer_trigger(
                        button(ButtonVariant::Outline, ButtonSize::Default).child(*name),
                    ))
                    // `max-h-[50vh]` of the 640px gallery window.
                    .child(scrolling(drawer_content(*dir).when(vertical, |c| c.max_height(320.0))))
            }));
        page(
            "Drawer",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo("DrawerDemo.vue", goal_demo))
                .child(demo("DrawerSides.vue", sides_demo))
                .child(demo(
                    "DrawerScrollableContent.vue",
                    drawer()
                        .child(drawer_trigger(
                            button(ButtonVariant::Outline, ButtonSize::Default).child("Scrollable Content"),
                        ))
                        .child(scrolling(drawer_content(DrawerDirection::Right))),
                )),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &[0, 125, 250, 500], |s| s.click_text("Open Drawer"))?;
    motion_frames(s, "closing", &[0, 125, 250, 500], |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.click_text("Open Drawer")?;
    s.settle()?;
    s.frame("open")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closed")?;
    s.click_text("Open Drawer")?;
    s.settle()?;
    let (x, y) = s.locate("Move Goal")?;
    s.drag((x, y), (x, y + 20.0));
    s.frame("short swipe stays")?;
    s.drag((x, y), (x, y + 80.0));
    s.frame("long swipe dismissed")?;
    for side in ["Top", "Right", "Left"] {
        s.click_text(side)?;
        s.frame(&format!("{side} open"))?;
        s.click_text("Cancel")?;
        s.settle()?;
    }
    s.click_text("Bottom")?;
    s.settle()?;
    s.click(480.0, 40.0);
    s.frame("overlay press closed")?;
    s.click_text("Scrollable Content")?;
    s.settle()?;
    s.frame("scrollable content open")?;
    let (x, y) = s.locate("Move Goal")?;
    s.hover(x, y + 200.0);
    s.wheel(400.0);
    s.frame("scrolled the paragraphs")?;
    Ok(())
}
