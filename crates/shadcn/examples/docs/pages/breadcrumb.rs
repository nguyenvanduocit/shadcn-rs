use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, Side, SideAlign, Signal, div, icon, text};
use shadcn::{
    MenuItem, MenuState, breadcrumb, breadcrumb_ellipsis, breadcrumb_item, breadcrumb_link, breadcrumb_list,
    breadcrumb_page, breadcrumb_separator, breadcrumb_separator_of, dropdown_menu_content, dropdown_menu_item,
    dropdown_menu_trigger,
};

use crate::{Render, Shot, demo, page};

pub fn render() -> Render {
    let visited = Signal::labeled("visited", String::from("nothing yet"));
    let components = MenuState::new();
    let collapsed = MenuState::new();
    Box::new(move || {
        let link = |label: &'static str| {
            let visited = visited.clone();
            breadcrumb_link(label).on_click(move |_| visited.set(label.to_owned()))
        };
        // A menu of pages the trail skips; picking one visits it.
        let menu = |m: &MenuState, trigger: Div, labels: &[&'static str]| {
            let content = dropdown_menu_content(m).children(labels.iter().map(|&label| {
                let visited = visited.clone();
                dropdown_menu_item(m, label, MenuItem::default(), move || visited.set(label.to_owned())).child(label)
            }));
            dropdown_menu_trigger(m, trigger, Side::Bottom, SideAlign::Start, content)
        };
        // The demos' `DropdownMenuTrigger class="flex items-center gap-1"`.
        let trigger = || div().flex().items_center().gap(1.0).cursor_pointer().focusable();
        page(
            "Breadcrumb",
            div()
                .flex_col()
                .gap(8.0)
                .child(demo(
                    "BreadcrumbDemo.vue",
                    breadcrumb().child(
                        breadcrumb_list()
                            .child(breadcrumb_item().child(link("Home")))
                            .child(breadcrumb_separator())
                            .child(breadcrumb_item().child(trigger().child(breadcrumb_ellipsis().size(4.0))))
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
                                    &components,
                                    // `[&_svg:not([class*='size-'])]:size-3.5`.
                                    trigger()
                                        .child(text("Components"))
                                        .child(icon(Icon::ChevronDown).size(3.5)),
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
                                &collapsed,
                                trigger().child(breadcrumb_ellipsis().size(4.0)),
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
                .child(text(format!("Clicked: {}", visited.get())).text_sm()),
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
        .find(|n| n.bounds.width > 20.0)
        .map(|n| (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0))
        .ok_or_else(|| ScenarioError::TextNotFound("components menu".into()))?;
    s.click(x, y);
    s.frame("components menu open")?;
    s.click_text("Themes")?;
    s.frame("picked themes")?;
    Ok(())
}
