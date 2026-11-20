use gui::prelude::*;
use gui::{Div, Icon, Modifiers, NamedKey, ScenarioError, Signal, div, icon, text};
use shadcn::{
    NavigationMenuState, navigation_menu, navigation_menu_content, navigation_menu_item, navigation_menu_link,
    navigation_menu_list, navigation_menu_trigger, navigation_menu_trigger_style, theme,
};

use crate::{Render, Shot, demo, motion_frames, page};

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
    let nav = NavigationMenuState::new();
    let visited = Signal::labeled("visited", String::from("nowhere yet"));
    Box::new(move || {
        let t = theme();
        let go = |to: &'static str| {
            let visited = visited.clone();
            move || visited.set(to.to_owned())
        };
        // `NavigationMenuItem.vue`, the demo's list item.
        let list_item = |title: &'static str, body: &'static str| {
            navigation_menu_link(false, go(title))
                .child(text(title).text_sm().leading_none().font_medium())
                .child(
                    text(body)
                        .text_color(t.muted_foreground)
                        .line_clamp(2)
                        .text_sm()
                        .leading_snug(),
                )
        };
        let titled = |title: &'static str, body: &'static str| {
            navigation_menu_link(false, go(title))
                .child(text(title).font_medium())
                .child(text(body).text_color(t.muted_foreground))
        };
        let plain = |label: &'static str| navigation_menu_link(false, go(label)).child(label);
        let with_icon = |glyph: Icon, label: &'static str| {
            navigation_menu_link(false, go(label))
                .flex_row()
                .items_center()
                .gap(2.0)
                .child(icon(glyph).text_color(t.muted_foreground))
                .child(label)
        };
        let stack = |width: f32, links: Vec<Div>| div().grid().w_px(width).gap(4.0).child(div().children(links));

        // `bg-linear-to-b from-muted/50 to-muted`: gui paints no gradients, so the end colour fills it.
        let hero = navigation_menu_link(false, go("shadcn/ui"))
            .row_span(3)
            .h_full()
            .w_full()
            .justify_end()
            .rounded_md()
            .bg(t.muted)
            .p(6.0)
            .child(text("shadcn/ui").mt(4.0).mb(2.0).text_lg().font_medium())
            .child(
                text("Beautifully designed components built with Tailwind CSS.")
                    .text_color(t.muted_foreground)
                    .text_sm()
                    .leading_tight(),
            );
        let home = navigation_menu_content().child(
            div()
                .grid()
                .gap(2.0)
                .w_px(400.0)
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
                .grid()
                .w_px(500.0)
                .gap(2.0)
                .grid_cols(2)
                .children(COMPONENTS.map(|(title, body)| list_item(title, body))),
        );
        let list = navigation_menu_content().child(stack(
            300.0,
            vec![
                titled("Components", "Browse all components in the library."),
                titled("Documentation", "Learn how to use the library."),
                titled("Blog", "Read our latest blog posts."),
            ],
        ));
        let simple = navigation_menu_content().child(stack(
            200.0,
            vec![plain("Components"), plain("Documentation"), plain("Blocks")],
        ));
        let icons = navigation_menu_content().child(stack(
            200.0,
            vec![
                with_icon(Icon::CircleHelp, "Backlog"),
                with_icon(Icon::Circle, "To Do"),
                with_icon(Icon::CircleCheck, "Done"),
            ],
        ));
        let docs = navigation_menu_trigger_style(false).on_click({
            let go = go("Docs");
            move |_| go()
        });

        let menu = navigation_menu().child(
            navigation_menu_list()
                .child(navigation_menu_item().child(navigation_menu_trigger(&nav, "home", "Home", home)))
                .child(navigation_menu_item().child(navigation_menu_trigger(
                    &nav,
                    "components",
                    "Components",
                    components,
                )))
                .child(navigation_menu_item().child(docs.child("Docs")))
                .child(navigation_menu_item().child(navigation_menu_trigger(&nav, "list", "List", list)))
                .child(navigation_menu_item().child(navigation_menu_trigger(&nav, "simple", "Simple", simple)))
                .child(navigation_menu_item().child(navigation_menu_trigger(&nav, "icon", "With Icon", icons))),
        );
        page(
            "Navigation Menu",
            div()
                .flex_col()
                .gap(6.0)
                .child(demo("NavigationMenuDemo.vue", menu))
                .child(
                    text(format!("Visited: {}.", visited.get()))
                        .text_sm()
                        .text_color(t.muted_foreground),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    motion_frames(s, "opening", &[0, 50, 100, 200], |s| s.click_text("Home"))?;
    motion_frames(s, "closing", &[0, 50, 100, 200], |s| {
        s.press(NamedKey::Escape, Modifiers::default());
        Ok(())
    })?;
    s.hover(900.0, 600.0);
    let (x, y) = s.locate("Docs")?;
    s.hover(x, y);
    s.frame("hover docs")?;
    s.click_text("Home")?;
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
    s.frame("with icon")?;
    s.click(900.0, 600.0);
    s.frame("click outside closes")?;
    Ok(())
}
