use gui::prelude::*;
use gui::{Element, Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text, text_bound};
use shadcn::focus::StateLook;
use shadcn::{
    navigation_menu, navigation_menu_content, navigation_menu_item, navigation_menu_link, navigation_menu_link_as,
    navigation_menu_list, navigation_menu_trigger, navigation_menu_trigger_style, theme,
};

use crate::{Render, Shot, demo, motion_frames, page};

/// How long the pointer moves on a trigger before its panel opens.
const OPEN_DELAY: std::time::Duration = primitives::navigation_menu::OPEN_DELAY;

const COMPONENTS: [(&str, &str); 6] = [
    (
        "Alert Dialog",
        "A modal dialog that interrupts the user with important content and expects a response.",
    ),
    (
        "Hover Card",
        "For sighted users to preview content available behind a link.",
    ),
    (
        "Progress",
        "Displays an indicator showing the completion progress of a task, typically displayed as a progress bar.",
    ),
    ("Scroll-area", "Visually or semantically separates content."),
    (
        "Tabs",
        "A set of layered sections of content—known as tab panels—that are displayed one at a time.",
    ),
    (
        "Tooltip",
        "A popup that displays information related to an element when the element receives keyboard focus or the mouse hovers over it.",
    ),
];

pub fn render() -> Render {
    let visited = Signal::labeled("visited", String::from("nowhere yet"));
    Box::new(move || {
        let t = theme();
        let go = |to: &'static str| move |()| visited.set(to.to_owned());
        // `NavigationMenuItem.vue`, the demo's list item.
        let list_item = |title: &'static str, body: &'static str| {
            navigation_menu_link(false, go(title))
                .child(
                    text(title)
                        .text_size(14.0)
                        .line_height(20.0)
                        .line_height(LineHeight::Relative(1.0))
                        .font_weight(500),
                )
                .child(
                    text(body)
                        .text_color(t.muted_foreground)
                        .line_clamp(2)
                        .text_size(14.0)
                        .line_height(20.0)
                        .line_height(LineHeight::Relative(1.375)),
                )
        };
        let titled = |title: &'static str, body: &'static str| {
            navigation_menu_link(false, go(title))
                .child(text(title).font_weight(500))
                .child(text(body).text_color(t.muted_foreground))
        };
        let plain = |label: &'static str| navigation_menu_link(false, go(label)).child(label);
        let with_icon = |glyph: Icon, label: &'static str| {
            navigation_menu_link(false, go(label))
                .display(Display::Flex)
                .flex_direction(FlexDirection::Row)
                .align_items(Align::Center)
                .gap(8.0)
                .child(icon(glyph).text_color(t.muted_foreground))
                .child(label)
        };
        let stack = |width: f32, links: Vec<Element>| {
            div()
                .display(Display::Grid)
                .width(width)
                .gap(16.0)
                .child(div().children(links))
        };

        // `bg-linear-to-b from-muted/50 to-muted`: gui paints no gradients, so the end colour fills it.
        let hero = navigation_menu_link(false, go("shadcn/ui"))
            .grid_row_span(3)
            .height(pct(100.0))
            .width(pct(100.0))
            .justify_content(Justify::End)
            .radius(8.0)
            .map_root(|d| {
                d.with_interaction(|d, i| {
                    // Over the link's rest: its hover and focus fill stay over it.
                    d.background(t.muted)
                        .background_while(move || i.hovered() || i.focused(), t.accent)
                })
            })
            .padding(24.0)
            .child(
                text("shadcn/ui")
                    .margin_top(16.0)
                    .margin_bottom(8.0)
                    .text_size(18.0)
                    .line_height(28.0)
                    .font_weight(500),
            )
            .child(
                text("Beautifully designed components built with Tailwind CSS.")
                    .text_color(t.muted_foreground)
                    .text_size(14.0)
                    .line_height(20.0)
                    .line_height(LineHeight::Relative(1.25)),
            );
        let home = navigation_menu_content().child(
            div()
                .display(Display::Grid)
                .gap(8.0)
                .width(400.0)
                .child(hero)
                .child(list_item(
                    "Introduction",
                    "Re-usable components built using Radix UI and Tailwind CSS.",
                ))
                .child(list_item(
                    "Installation",
                    "How to install dependencies and structure your app.",
                ))
                .child(list_item("Typography", "Styles for headings, paragraphs, lists...etc")),
        );
        let components = navigation_menu_content().child(
            div()
                .display(Display::Grid)
                .width(500.0)
                .gap(8.0)
                .grid_columns(Tracks::repeat(2, Track::Fr(1.0)))
                .children(COMPONENTS.map(|(title, body)| list_item(title, body))),
        );
        let list = navigation_menu_content().child(stack(
            300.0,
            vec![
                titled("Components", "Browse all components in the library.").into_element(),
                titled("Documentation", "Learn how to use the library.").into_element(),
                titled("Blog", "Read our latest blog posts.").into_element(),
            ],
        ));
        let simple = navigation_menu_content().child(stack(
            200.0,
            vec![
                plain("Components").into_element(),
                plain("Documentation").into_element(),
                plain("Blocks").into_element(),
            ],
        ));
        let icons = navigation_menu_content().child(stack(
            200.0,
            vec![
                with_icon(Icon::CircleHelp, "Backlog").into_element(),
                with_icon(Icon::Circle, "To Do").into_element(),
                with_icon(Icon::CircleCheck, "Done").into_element(),
            ],
        ));
        let docs = navigation_menu_link_as(navigation_menu_trigger_style(false), false, go("Docs"));

        let entry = |value: &str, label: &str, content| {
            navigation_menu_item(value)
                .child(navigation_menu_trigger(label))
                .child(content)
        };
        let menu = navigation_menu().viewport(false).child(
            navigation_menu_list()
                .child(entry("home", "Home", home))
                .child(entry("components", "Components", components))
                .child(navigation_menu_item("docs").child(docs.child("Docs")))
                .child(entry("list", "List", list))
                .child(entry("simple", "Simple", simple))
                .child(entry("icon", "With Icon", icons)),
        );
        page(
            "Navigation Menu",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(24.0)
                .child(demo("NavigationMenuDemo.vue", menu))
                .child(
                    text_bound(move || format!("Visited: {}.", visited.get()))
                        .text_size(14.0)
                        .line_height(20.0)
                        .text_color(t.muted_foreground),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    // The pointer moving on a trigger opens it after 200 ms, and a click
    // within 300 ms of that move is ignored (reka's): it opens by the hover.
    motion_frames(s, "opening", &[0, 50, 100, 200], |s| {
        let (x, y) = s.locate("Home")?;
        s.hover(x, y);
        s.wait(OPEN_DELAY);
        Ok(())
    })?;
    motion_frames(s, "closing", &[0, 50, 100, 200], |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.hover(900.0, 600.0);
    let (x, y) = s.locate("Docs")?;
    s.hover(x, y);
    s.frame("hover docs")?;
    s.click_text("Home")?;
    s.rest(OPEN_DELAY.saturating_mul(2))?;
    s.frame("home open")?;
    let (x, y) = s.locate("Installation")?;
    s.hover(x, y);
    s.frame("hover installation")?;
    let (x, y) = s.locate("Components")?;
    s.hover(x, y);
    s.frame("hover switches to components")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("escape closes")?;
    s.click_text("With Icon")?;
    s.rest(OPEN_DELAY.saturating_mul(2))?;
    s.frame("with icon")?;
    s.click(900.0, 600.0);
    s.frame("click outside closes")?;
    Ok(())
}
