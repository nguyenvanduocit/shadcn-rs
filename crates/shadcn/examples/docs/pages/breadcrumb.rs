use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, SideAlign, Signal, div, icon, text};
use shadcn::{
    MenuItem, breadcrumb, breadcrumb_ellipsis, breadcrumb_item, breadcrumb_link, breadcrumb_list, breadcrumb_page,
    breadcrumb_separator, breadcrumb_separator_of, dropdown_menu, dropdown_menu_content, dropdown_menu_item,
    dropdown_menu_trigger,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let visited = Signal::labeled("visited", String::from("nothing yet"));
    Box::new(move || {
        let link = |label: &'static str| breadcrumb_link(label).on_click(move |_| visited.set(label.to_owned()));
        // A menu of pages the trail skips; picking one visits it.
        let menu = |trigger: Div, labels: &[&'static str]| {
            let content = dropdown_menu_content()
                .align(SideAlign::Start)
                .children(labels.iter().map(|&label| {
                    dropdown_menu_item(label, MenuItem::default())
                        .on_select(move |_| visited.set(label.to_owned()))
                        .child(label)
                }));
            dropdown_menu().child(dropdown_menu_trigger(trigger)).child(content)
        };
        // The demos' `DropdownMenuTrigger class="flex items-center gap-1"`.
        let trigger = || {
            div()
                .display(Display::Flex)
                .align_items(Align::Center)
                .gap(4.0)
                .cursor(Cursor::Pointer)
                .focusable()
        };
        page(
            "Breadcrumb",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(32.0)
                .child(demo(
                    "BreadcrumbDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(trigger().child(breadcrumb_ellipsis().size(16.0))))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(link("Components")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(breadcrumb_page("Breadcrumb"))),
                    ),
                ))
                .child(demo(
                    "BreadcrumbCollapsedDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(breadcrumb_ellipsis()))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(link("Components")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(breadcrumb_page("Breadcrumb"))),
                    ),
                ))
                .child(demo(
                    "BreadcrumbCustomSeparatorDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator_of(Icon::Slash))
                            .child(breadcrumb_item().child(link("Components")))
                            .child(breadcrumb_separator_of(Icon::Slash))
                            .child(breadcrumb_item().child(breadcrumb_page("Breadcrumb"))),
                    ),
                ))
                .child(demo(
                    "BreadcrumbDropdownDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator_of(Icon::Slash))
                            .child(
                                breadcrumb_item().child(menu(
                                    // `[&_svg:not([class*='size-'])]:size-3.5`.
                                    trigger()
                                        .child(text("Components"))
                                        .child(icon(Icon::ChevronDown).size(14.0)),
                                    &["Documentation", "Themes", "GitHub"],
                                )),
                            )
                            .child(breadcrumb_separator_of(Icon::Slash))
                            .child(breadcrumb_item().child(breadcrumb_page("Breadcrumb"))),
                    ),
                ))
                .child(demo(
                    "BreadcrumbLinkDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(link("Components")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(breadcrumb_page("Breadcrumb"))),
                    ),
                ))
                .child(demo(
                    "BreadcrumbResponsiveDemo.vue",
                    // The desktop branch (`min-width: 768px`): the middle pages
                    // in a menu, the last two at full width (`md:max-w-none`).
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(menu(
                                trigger().child(breadcrumb_ellipsis().size(16.0)),
                                &["Documentation", "Building Your Application"],
                            )))
                            .child(breadcrumb_separator())
                            // The demo puts this separator inside the item, so its gap is the item's.
                            .child(
                                breadcrumb_item()
                                    .child(link("Data Fetching"))
                                    .child(breadcrumb_separator()),
                            )
                            .child(breadcrumb_item().child(breadcrumb_page("Caching and Revalidating"))),
                    ),
                ))
                .child(
                    text(format!("Clicked: {}", visited.get()))
                        .text_size(14.0)
                        .line_height(20.0),
                ),
        )
    })
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Components")?;
    s.hover(x, y);
    s.frame("hover components")?;
    s.click_text("Home")?;
    s.frame("clicked home")?;
    let (x, y) = s
        .last()
        .nodes
        .iter()
        .filter(|n| n.name == Some("dropdown-menu-trigger"))
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .find(|b| b.width > 20.0)
        .map(|b| (b.x + b.width / 2.0, b.y + b.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("components menu".into()))?;
    s.click(x, y);
    s.frame("components menu open")?;
    s.click_text("Themes")?;
    s.frame("picked themes")?;
    Ok(())
}
