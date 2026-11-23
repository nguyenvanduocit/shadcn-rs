//! Sidebar: shadcn's `sidebar-07` (team switcher, collapsible main nav,
//! projects, user; collapses to icons) beside a fixed sidebar showing the
//! parts sidebar-07 does not use.

use gui::prelude::*;
use gui::{Div, Element, Icon, Model, ScenarioError, Signal, TextField, div, icon, text};
use shadcn::focus::StateLook;
use shadcn::{
    ButtonSize, ButtonVariant, Orientation, SidebarCollapsible, SidebarMenuButtonProps, SidebarMenuButtonSize,
    SidebarMenuButtonVariant, SidebarMenuSubButtonSize, SidebarSide, SidebarVariant, avatar, avatar_fallback,
    breadcrumb, breadcrumb_item, breadcrumb_link, breadcrumb_list, breadcrumb_page, breadcrumb_separator, button,
    separator, sidebar, sidebar_content, sidebar_footer, sidebar_group, sidebar_group_action, sidebar_group_content,
    sidebar_group_label, sidebar_header, sidebar_input, sidebar_inset, sidebar_menu, sidebar_menu_action,
    sidebar_menu_badge, sidebar_menu_button, sidebar_menu_item, sidebar_menu_skeleton, sidebar_menu_sub,
    sidebar_menu_sub_button, sidebar_menu_sub_item, sidebar_provider, sidebar_rail, sidebar_separator, sidebar_trigger,
    theme,
};

use crate::{Render, Shot, page};

/// sidebar-07's main nav.
const NAV: [(&str, Icon, &[&str]); 4] = [
    ("Playground", Icon::SquareTerminal, &["History", "Starred", "Settings"]),
    ("Models", Icon::Bot, &["Genesis", "Explorer", "Quantum"]),
    (
        "Documentation",
        Icon::BookOpen,
        &["Introduction", "Get Started", "Tutorials", "Changelog"],
    ),
    ("Settings", Icon::Settings2, &["General", "Team", "Billing", "Limits"]),
];

/// sidebar-07's projects.
const PROJECTS: [(&str, Icon); 3] = [
    ("Design Engineering", Icon::Frame),
    ("Sales & Marketing", Icon::PieChart),
    ("Travel", Icon::Map),
];

/// Whether sidebar-07 is open: the script presses ⌘B through it.
#[derive(Clone, Copy)]
struct SidebarOpen(Signal<bool>);

/// The page's state.
#[derive(Clone)]
struct Demo {
    /// sidebar-07's `v-model:open`: the page's inset follows it too.
    open: Signal<bool>,
    variant: Signal<SidebarVariant>,
    collapsible: Signal<SidebarCollapsible>,
    side: Signal<SidebarSide>,
    open_groups: Signal<Vec<&'static str>>,
    active: Signal<&'static str>,
    search: TextField,
}

impl Demo {
    /// sidebar-07 collapsed to its icons.
    fn icon_collapsed(&self) -> bool {
        self.collapsible.get() == SidebarCollapsible::Icon && !self.open.get()
    }
}

pub fn render() -> Render {
    let demo = Demo {
        open: Signal::labeled("sidebar open", true),
        variant: Signal::labeled("variant", SidebarVariant::Sidebar),
        collapsible: Signal::labeled("collapsible", SidebarCollapsible::Icon),
        side: Signal::labeled("side", SidebarSide::Left),
        open_groups: Signal::labeled("open groups", vec!["Playground"]),
        active: Signal::labeled("active", "History"),
        search: TextField::new(""),
    };
    // The script reaches the sidebar through this to press ⌘B.
    gui::provide(SidebarOpen(demo.open));
    Box::new(move || {
        let t = theme();
        let frame = |w: Div| {
            w.height(pct(100.0))
                .border(1.0)
                .border_color(t.border)
                .radius(10.0)
                .overflow(Overflow::Hidden)
        };
        page(
            "Sidebar",
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .gap(16.0)
                .flex(Flex::grow(1.0))
                .min_height(0.0)
                .child(controls(&demo))
                .child(
                    div()
                        .display(Display::Flex)
                        .gap(16.0)
                        .height(472.0)
                        .flex_shrink(0.0)
                        .child(frame(div().flex(Flex::grow(1.0)).min_width(0.0)).child(sidebar_07(&demo)))
                        .child(frame(div().width(258.0).flex_shrink(0.0)).child(parts(&demo))),
                )
                .child(crate::demo("SidebarDemo.vue", sidebar_demo())),
        )
    })
}

/// `SidebarDemo.vue`: one group of five links, and a trigger in the inset's
/// header. Its provider is `min-h-svh`, the reference's 900px viewport.
fn sidebar_demo() -> Element {
    let items = [
        ("Home", Icon::House),
        ("Inbox", Icon::Inbox),
        ("Calendar", Icon::Calendar),
        ("Search", Icon::Search),
        ("Settings", Icon::Settings),
    ];
    let menu = sidebar_menu().children(items.map(|(title, glyph)| {
        sidebar_menu_item().child(
            sidebar_menu_button(SidebarMenuButtonProps::default())
                .child(icon(glyph))
                .child(
                    text(title)
                        .truncate(true)
                        .nowrap(true)
                        .overflow_x(Overflow::Hidden)
                        .min_width(0.0),
                ),
        )
    }));
    sidebar_provider()
        .height(900.0)
        .child(
            sidebar().child(
                sidebar_content().child(
                    sidebar_group()
                        .child(sidebar_group_label().child("Application"))
                        .child(sidebar_group_content().child(menu)),
                ),
            ),
        )
        .child(
            sidebar_inset().child(
                div()
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .justify_content(Justify::Between)
                    .padding_x(16.0)
                    .height(48.0)
                    .child(sidebar_trigger()),
            ),
        )
        .into_element()
}

/// Buttons choosing the variant, collapse mode and side.
fn controls(demo: &Demo) -> Div {
    fn choice<T: Copy + PartialEq + 'static>(signal: &Signal<T>, options: &[(&'static str, T)]) -> Div {
        let current = signal.get();
        div()
            .display(Display::Flex)
            .gap(4.0)
            .children(options.iter().map(|&(label, value)| {
                let signal = *signal;
                let variant = if value == current {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                };
                button(variant, ButtonSize::Sm)
                    .child(label)
                    .on_click(move |_| signal.set(value))
            }))
    }
    let t = theme();
    let caption = |s: &'static str| text(s).text_size(14.0).line_height(20.0).text_color(t.muted_foreground);
    div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(12.0)
        .child(caption("Variant"))
        .child(choice(
            &demo.variant,
            &[
                ("sidebar", SidebarVariant::Sidebar),
                ("floating", SidebarVariant::Floating),
                ("inset", SidebarVariant::Inset),
            ],
        ))
        .child(caption("Collapsible"))
        .child(choice(
            &demo.collapsible,
            &[
                ("icon", SidebarCollapsible::Icon),
                ("offcanvas", SidebarCollapsible::Offcanvas),
                ("none", SidebarCollapsible::None),
            ],
        ))
        .child(caption("Side"))
        .child(choice(
            &demo.side,
            &[("left", SidebarSide::Left), ("right", SidebarSide::Right)],
        ))
}

fn sidebar_07(demo: &Demo) -> Element {
    let side = sidebar()
        .child(sidebar_header().child(team_switcher()))
        .child(sidebar_content().child(nav_main(demo)).child(nav_projects(demo)))
        .child(sidebar_footer().child(nav_user()))
        .child(sidebar_rail());
    let provider = sidebar_provider()
        .model(Model::bind(demo.open))
        .variant(demo.variant.get())
        .collapsible(demo.collapsible.get())
        .side(demo.side.get());
    match demo.side.get() {
        SidebarSide::Left => provider.child(side).child(inset(demo)),
        SidebarSide::Right => provider.child(inset(demo)).child(side),
    }
    .into_element()
}

/// The square logo or avatar at the start of a large menu button.
fn tile(label: impl IntoElement) -> Div {
    div()
        .size(32.0)
        .flex_shrink(0.0)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .radius(10.0)
        .overflow(Overflow::Hidden)
        .child(label)
}

/// Two lines (name over detail) beside a tile, then the up-down chevrons.
fn identity(tile: impl IntoElement, name: &str, detail: &str) -> Div {
    sidebar_menu().child(
        sidebar_menu_item().child(
            sidebar_menu_button(SidebarMenuButtonProps {
                size: SidebarMenuButtonSize::Lg,
                ..SidebarMenuButtonProps::default()
            })
            .child(tile)
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .flex(Flex::grow(1.0))
                    .min_width(0.0)
                    .text_align(TextAlign::Left)
                    .text_size(14.0)
                    .line_height(20.0)
                    .line_height(LineHeight::Relative(1.25))
                    .child(
                        text(name)
                            .truncate(true)
                            .nowrap(true)
                            .overflow_x(Overflow::Hidden)
                            .min_width(0.0)
                            .font_weight(500),
                    )
                    .child(
                        text(detail)
                            .truncate(true)
                            .nowrap(true)
                            .overflow_x(Overflow::Hidden)
                            .min_width(0.0)
                            .text_size(12.0)
                            .line_height(16.0),
                    ),
            )
            .child(icon(Icon::ChevronsUpDown).margin_left(Length::Auto)),
        ),
    )
}

fn team_switcher() -> Div {
    let t = theme();
    let logo = tile(icon(Icon::GalleryVerticalEnd))
        .background(t.sidebar_primary)
        .text_color(t.sidebar_primary_foreground);
    identity(logo, "Acme Inc", "Enterprise")
}

fn nav_user() -> Div {
    let initials = avatar_fallback("CN").radius(10.0);
    identity(avatar().radius(10.0).child(initials), "shadcn", "m@example.com")
}

fn nav_main(demo: &Demo) -> Div {
    let open = demo.open_groups.get();
    let current = demo.active.get();
    sidebar_group()
        .child(sidebar_group_label().child("Platform"))
        .child(sidebar_menu().children(NAV.iter().map(|&(title, glyph, subs)| {
            let is_open = open.contains(&title);
            let groups = demo.open_groups;
            let button = sidebar_menu_button(SidebarMenuButtonProps {
                tooltip: Some(title.to_owned()),
                ..SidebarMenuButtonProps::default()
            })
            .on_click(move |_| {
                groups.update(|g| {
                    if g.contains(&title) {
                        g.retain(|o| *o != title);
                    } else {
                        g.push(title);
                    }
                });
            })
            .child(icon(glyph))
            .child(
                text(title)
                    .truncate(true)
                    .nowrap(true)
                    .overflow_x(Overflow::Hidden)
                    .min_width(0.0),
            )
            .child(
                icon(Icon::ChevronRight)
                    .margin_left(Length::Auto)
                    .when(is_open, |i| i.rotate(90.0)),
            );
            sidebar_menu_item().child(button).when(is_open, |d| {
                d.child(sidebar_menu_sub().children(subs.iter().map(|&sub| {
                    let active = demo.active;
                    sidebar_menu_sub_item().child(
                        sidebar_menu_sub_button(SidebarMenuSubButtonSize::Md, sub == current)
                            .on_click(move |_| active.set(sub))
                            .child(
                                text(sub)
                                    .truncate(true)
                                    .nowrap(true)
                                    .overflow_x(Overflow::Hidden)
                                    .min_width(0.0),
                            ),
                    )
                })))
            })
        })))
}

fn nav_projects(demo: &Demo) -> Div {
    let t = theme();
    let current = demo.active.get();
    let items = PROJECTS.iter().map(|&(name, glyph)| {
        let active = demo.active;
        sidebar_menu_item()
            .child(
                sidebar_menu_button(SidebarMenuButtonProps {
                    is_active: name == current,
                    ..SidebarMenuButtonProps::default()
                })
                .padding_right(32.0)
                .on_click(move |_| active.set(name))
                .child(icon(glyph))
                .child(
                    text(name)
                        .truncate(true)
                        .nowrap(true)
                        .overflow_x(Overflow::Hidden)
                        .min_width(0.0),
                ),
            )
            .child(sidebar_menu_action(SidebarMenuButtonSize::Default, true).child(icon(Icon::Ellipsis)))
    });
    let more = sidebar_menu_item().child(
        sidebar_menu_button(SidebarMenuButtonProps::default())
            .when(true, |b| {
                // Over the button's rest: its hover and press text stay over it.
                let i = b.node_interaction();
                let b = b.text_color(t.sidebar_foreground.opacity(0.7));
                match i {
                    Some(i) => b.text_color_while(move || i.hovered() || i.pressed(), t.sidebar_accent_foreground),
                    None => b,
                }
            })
            .child(icon(Icon::Ellipsis))
            .child(
                text("More")
                    .truncate(true)
                    .nowrap(true)
                    .overflow_x(Overflow::Hidden)
                    .min_width(0.0),
            ),
    );
    sidebar_group()
        .when(demo.icon_collapsed(), |e| e.display(Display::None))
        .child(sidebar_group_label().child("Projects"))
        .child(sidebar_menu().children(items).child(more))
}

fn inset(demo: &Demo) -> impl IntoElement {
    let t = theme();
    let placeholder = || div().background(t.muted.opacity(0.5)).radius(14.0);
    let crumbs = breadcrumb().child(
        breadcrumb_list()
            .child(breadcrumb_item().child(breadcrumb_link("Building Your Application")))
            .child(breadcrumb_separator())
            .child(breadcrumb_item().child(breadcrumb_page("Data Fetching"))),
    );
    sidebar_inset()
        .child(
            div()
                .display(Display::Flex)
                .height((if demo.icon_collapsed() { 12.0 } else { 16.0 }) * 4.0)
                .flex_shrink(0.0)
                .align_items(Align::Center)
                .gap(8.0)
                .child(
                    div()
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .gap(8.0)
                        .padding_x(16.0)
                        .child(sidebar_trigger().margin_left(-4.0))
                        .child(separator(Orientation::Vertical).margin_right(8.0).height(16.0))
                        .child(crumbs),
                ),
        )
        .child(
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .flex(Flex::grow(1.0))
                .gap(16.0)
                .padding(16.0)
                .padding_top(0.0)
                .child(
                    div()
                        .display(Display::Grid)
                        .grid_columns(Tracks::repeat(3, Track::Fr(1.0)))
                        .gap(16.0)
                        .children((0..3).map(|_| placeholder().aspect_ratio(16.0 / 9.0))),
                )
                .child(placeholder().flex(Flex::grow(1.0))),
        )
}

/// A fixed (`collapsible: none`) sidebar with an input, a group action,
/// badges, the outline variant, the three sizes, a small sub menu and skeletons.
fn parts(demo: &Demo) -> Element {
    let props = |variant, size| SidebarMenuButtonProps {
        variant,
        size,
        ..SidebarMenuButtonProps::default()
    };
    let inbox = sidebar_menu()
        .child(
            sidebar_menu_item()
                .child(
                    sidebar_menu_button(SidebarMenuButtonProps {
                        is_active: true,
                        ..props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Default)
                    })
                    .child(icon(Icon::Inbox))
                    .child(
                        text("Inbox")
                            .truncate(true)
                            .nowrap(true)
                            .overflow_x(Overflow::Hidden)
                            .min_width(0.0),
                    ),
                )
                .child(sidebar_menu_badge(SidebarMenuButtonSize::Default).child("24")),
        )
        .child(
            sidebar_menu_item()
                .child(
                    sidebar_menu_button(props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Sm))
                        .child(icon(Icon::Pencil))
                        .child(
                            text("Drafts")
                                .truncate(true)
                                .nowrap(true)
                                .overflow_x(Overflow::Hidden)
                                .min_width(0.0),
                        ),
                )
                .child(sidebar_menu_badge(SidebarMenuButtonSize::Sm).child("3"))
                .child(
                    sidebar_menu_sub()
                        .child(
                            sidebar_menu_sub_item().child(
                                sidebar_menu_sub_button(SidebarMenuSubButtonSize::Sm, true).child(
                                    text("Replies")
                                        .truncate(true)
                                        .nowrap(true)
                                        .overflow_x(Overflow::Hidden)
                                        .min_width(0.0),
                                ),
                            ),
                        )
                        .child(
                            sidebar_menu_sub_item().child(
                                sidebar_menu_sub_button(SidebarMenuSubButtonSize::Sm, false).child(
                                    text("Notes")
                                        .truncate(true)
                                        .nowrap(true)
                                        .overflow_x(Overflow::Hidden)
                                        .min_width(0.0),
                                ),
                            ),
                        ),
                ),
        )
        .child(
            sidebar_menu_item().child(
                sidebar_menu_button(props(SidebarMenuButtonVariant::Outline, SidebarMenuButtonSize::Default))
                    .child(icon(Icon::Star))
                    .child(
                        text("Starred")
                            .truncate(true)
                            .nowrap(true)
                            .overflow_x(Overflow::Hidden)
                            .min_width(0.0),
                    ),
            ),
        )
        .child(
            sidebar_menu_item().child(
                sidebar_menu_button(props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Lg))
                    .child(icon(Icon::Download))
                    .child(
                        text("Archive (large)")
                            .truncate(true)
                            .nowrap(true)
                            .overflow_x(Overflow::Hidden)
                            .min_width(0.0),
                    ),
            ),
        );
    sidebar_provider()
        .collapsible(SidebarCollapsible::None)
        .child(
            sidebar()
                .child(sidebar_header().child(sidebar_input(&demo.search).placeholder("Search the docs...")))
                .child(sidebar_separator())
                .child(
                    sidebar_content()
                        .child(
                            sidebar_group()
                                .child(sidebar_group_label().child("Mail"))
                                .child(sidebar_group_action().child(icon(Icon::Plus)))
                                .child(sidebar_group_content().child(inbox)),
                        )
                        .child(sidebar_group().child(sidebar_group_label().child("Loading")).child(
                            sidebar_menu().children(
                                ["one", "two"].map(|id| sidebar_menu_item().child(sidebar_menu_skeleton(id, true))),
                            ),
                        )),
                ),
        )
        .into_element()
}

pub fn script(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default")?;
    let (x, y) = s.locate("Documentation")?;
    s.hover(x, y);
    s.wheel(200.0);
    s.frame("scrolled to projects")?;
    let (x, y) = s.locate("Design Engineering")?;
    s.hover(x, y);
    s.frame("hover project")?;
    s.click_text("Design Engineering")?;
    s.frame("project active")?;
    s.click_text("Models")?;
    s.frame("models expanded")?;
    s.click_text("Genesis")?;
    s.frame("genesis active")?;
    let (x, y) = centre(s, None, "sidebar-trigger")?;
    s.click(x, y);
    s.frame("collapsed to icons")?;
    let (x, y) = centre(s, Some("Models"), "sidebar-menu-button")?;
    s.hover(x, y);
    s.frame("collapsed tooltip")?;
    // Scenario presses only named keys, so ⌘B runs the provider's action directly.
    let open = gui::use_context::<SidebarOpen>().ok_or_else(|| ScenarioError::Unhandled("no sidebar state".into()))?;
    s.act("⌘B", || open.0.update(|o| *o = !*o));
    s.frame("expanded by shortcut")?;
    // The rail's right half lies under the inset, which is drawn after it.
    let (x, y) = centre(s, None, "sidebar-rail")?;
    s.hover(x - 4.0, y);
    s.frame("rail hover")?;
    s.click_text("floating")?;
    s.frame("floating")?;
    s.click_text("inset")?;
    s.frame("inset")?;
    let (x, y) = centre(s, None, "sidebar-trigger")?;
    s.click(x, y);
    s.frame("inset collapsed to icons")?;
    s.click_text("sidebar")?;
    s.click_text("offcanvas")?;
    s.frame("offcanvas collapsed")?;
    s.click_text("right")?;
    let (x, y) = centre(s, None, "sidebar-trigger")?;
    s.click(x, y);
    s.frame("right expanded")?;
    Ok(())
}

/// The centre of the first node named `name`: the one holding the text
/// `inside` when given, else the first in the frame.
fn centre(s: &mut Shot, inside: Option<&str>, name: &str) -> Result<(f32, f32), ScenarioError> {
    s.settle()?;
    let nodes = &s.last().nodes;
    let start = match inside {
        Some(label) => nodes
            .iter()
            .position(|n| gui::inspect::find_text(std::slice::from_ref(n), label).is_some()),
        None => nodes.iter().position(|n| n.name == Some(name)),
    };
    let found = start
        .and_then(|i| {
            std::iter::successors(Some(i), |&n| nodes.get(n).and_then(|node| node.parent))
                .find(|&n| nodes.get(n).is_some_and(|node| node.name == Some(name)))
        })
        .and_then(|i| nodes.get(i))
        .ok_or_else(|| ScenarioError::TextNotFound(name.to_owned()))?;
    let b = found.bounds;
    Ok((b.x + b.width / 2.0, b.y + b.height / 2.0))
}
