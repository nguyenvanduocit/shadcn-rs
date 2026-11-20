//! Sidebar: shadcn's `sidebar-07` (team switcher, collapsible main nav,
//! projects, user; collapses to icons) beside a fixed sidebar showing the
//! parts sidebar-07 does not use.

use gui::prelude::*;
use gui::{Div, Icon, ScenarioError, Signal, TextField, div, icon, text};
use shadcn::{
    ButtonSize, ButtonVariant, Orientation, SidebarCollapsible, SidebarMenuButtonProps, SidebarMenuButtonSize,
    SidebarMenuButtonVariant, SidebarMenuSubButtonSize, SidebarSide, SidebarState, SidebarVariant, avatar,
    avatar_fallback, breadcrumb, breadcrumb_item, breadcrumb_link, breadcrumb_list, breadcrumb_page,
    breadcrumb_separator, button, separator, sidebar, sidebar_content, sidebar_footer, sidebar_group,
    sidebar_group_action, sidebar_group_content, sidebar_group_label, sidebar_header, sidebar_input, sidebar_inset,
    sidebar_menu, sidebar_menu_action, sidebar_menu_badge, sidebar_menu_button, sidebar_menu_item,
    sidebar_menu_skeleton, sidebar_menu_sub, sidebar_menu_sub_button, sidebar_menu_sub_item, sidebar_provider,
    sidebar_rail, sidebar_separator, sidebar_trigger, theme,
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

/// The page's state.
#[derive(Clone)]
struct Demo {
    sb: SidebarState,
    variant: Signal<SidebarVariant>,
    collapsible: Signal<SidebarCollapsible>,
    side: Signal<SidebarSide>,
    open_groups: Signal<Vec<&'static str>>,
    active: Signal<&'static str>,
    parts: SidebarState,
    search: TextField,
    basic: SidebarState,
}

pub fn render() -> Render {
    let demo = Demo {
        sb: SidebarState::new(true),
        variant: Signal::labeled("variant", SidebarVariant::Sidebar),
        collapsible: Signal::labeled("collapsible", SidebarCollapsible::Icon),
        side: Signal::labeled("side", SidebarSide::Left),
        open_groups: Signal::labeled("open groups", vec!["Playground"]),
        active: Signal::labeled("active", "History"),
        parts: SidebarState::new(true).collapsible(SidebarCollapsible::None),
        search: TextField::new(""),
        basic: SidebarState::new(true),
    };
    // The script reaches the sidebar through this to press ⌘B.
    gui::provide(demo.sb.clone());
    Box::new(move || {
        let sb = demo
            .sb
            .clone()
            .variant(demo.variant.get())
            .collapsible(demo.collapsible.get())
            .side(demo.side.get());
        let t = theme();
        let frame = |w: Div| {
            w.h_full()
                .border()
                .border_color(t.border)
                .rounded_lg()
                .overflow_hidden()
        };
        page(
            "Sidebar",
            div()
                .flex_col()
                .gap(4.0)
                .flex_1()
                .min_h(0.0)
                .child(controls(&demo))
                .child(
                    div()
                        .flex()
                        .gap(4.0)
                        .h_px(472.0)
                        .shrink_0()
                        .child(frame(div().flex_1().min_w(0.0)).child(sidebar_07(&sb, &demo)))
                        .child(frame(div().w_px(258.0).shrink_0()).child(parts(&demo))),
                )
                .child(crate::demo("SidebarDemo.vue", sidebar_demo(&demo.basic))),
        )
    })
}

/// `SidebarDemo.vue`: one group of five links, and a trigger in the inset's
/// header. Its provider is `min-h-svh`, the reference's 900px viewport.
fn sidebar_demo(sb: &SidebarState) -> Div {
    let items = [
        ("Home", Icon::House),
        ("Inbox", Icon::Inbox),
        ("Calendar", Icon::Calendar),
        ("Search", Icon::Search),
        ("Settings", Icon::Settings),
    ];
    let menu = sidebar_menu().children(items.map(|(title, glyph)| {
        sidebar_menu_item(sb, title).child(
            sidebar_menu_button(sb, title, SidebarMenuButtonProps::default())
                .child(icon(glyph))
                .child(text(title).truncate()),
        )
    }));
    sidebar_provider(sb)
        .h_px(900.0)
        .child(sidebar(
            sb,
            [sidebar_content(sb).child(
                sidebar_group()
                    .child(sidebar_group_label(sb).child("Application"))
                    .child(sidebar_group_content().child(menu)),
            )],
        ))
        .child(
            sidebar_inset(sb).child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(4.0)
                    .h(12.0)
                    .child(sidebar_trigger(sb)),
            ),
        )
}

/// Buttons choosing the variant, collapse mode and side.
fn controls(demo: &Demo) -> Div {
    fn choice<T: Copy + PartialEq + 'static>(signal: &Signal<T>, options: &[(&'static str, T)]) -> Div {
        let current = signal.get();
        div().flex().gap(1.0).children(options.iter().map(|&(label, value)| {
            let signal = signal.clone();
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
    let caption = |s: &'static str| text(s).text_sm().text_color(t.muted_foreground);
    div()
        .flex()
        .items_center()
        .gap(3.0)
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

fn sidebar_07(sb: &SidebarState, demo: &Demo) -> Div {
    let side = sidebar(
        sb,
        [
            sidebar_header().child(team_switcher(sb)),
            sidebar_content(sb)
                .child(nav_main(sb, demo))
                .child(nav_projects(sb, demo)),
            sidebar_footer().child(nav_user(sb)),
            sidebar_rail(sb),
        ],
    );
    let provider = sidebar_provider(sb);
    match demo.side.get() {
        SidebarSide::Left => provider.child(side).child(inset(sb)),
        SidebarSide::Right => provider.child(inset(sb)).child(side),
    }
}

/// The square logo or avatar at the start of a large menu button.
fn tile(label: impl IntoElement) -> Div {
    div()
        .size(8.0)
        .shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_lg()
        .overflow_hidden()
        .child(label)
}

/// Two lines (name over detail) beside a tile, then the up-down chevrons.
fn identity(sb: &SidebarState, id: &str, tile: Div, name: &str, detail: &str) -> Div {
    sidebar_menu().child(
        sidebar_menu_item(sb, id).child(
            sidebar_menu_button(
                sb,
                id,
                SidebarMenuButtonProps {
                    size: SidebarMenuButtonSize::Lg,
                    ..SidebarMenuButtonProps::default()
                },
            )
            .child(tile)
            .child(
                div()
                    .flex_col()
                    .flex_1()
                    .min_w(0.0)
                    .text_left()
                    .text_sm()
                    .leading_tight()
                    .child(text(name).truncate().font_medium())
                    .child(text(detail).truncate().text_xs()),
            )
            .child(icon(Icon::ChevronsUpDown).ml_auto()),
        ),
    )
}

fn team_switcher(sb: &SidebarState) -> Div {
    let t = theme();
    let logo = tile(icon(Icon::GalleryVerticalEnd))
        .bg(t.sidebar_primary)
        .text_color(t.sidebar_primary_foreground);
    identity(sb, "team", logo, "Acme Inc", "Enterprise")
}

fn nav_user(sb: &SidebarState) -> Div {
    let initials = avatar_fallback(None, "CN").map(|f| f.rounded_lg());
    identity(
        sb,
        "user",
        avatar().rounded_lg().child(initials),
        "shadcn",
        "m@example.com",
    )
}

fn nav_main(sb: &SidebarState, demo: &Demo) -> Div {
    let open = demo.open_groups.get();
    let current = demo.active.get();
    sidebar_group()
        .child(sidebar_group_label(sb).child("Platform"))
        .child(sidebar_menu().children(NAV.iter().map(|&(title, glyph, subs)| {
            let is_open = open.contains(&title);
            let groups = demo.open_groups.clone();
            let button = sidebar_menu_button(
                sb,
                title,
                SidebarMenuButtonProps {
                    tooltip: Some(title.to_owned()),
                    ..SidebarMenuButtonProps::default()
                },
            )
            .on_click(move |_| {
                groups.update(|mut g| {
                    if g.contains(&title) {
                        g.retain(|o| *o != title);
                    } else {
                        g.push(title);
                    }
                    g
                });
            })
            .child(icon(glyph))
            .child(text(title).truncate())
            .child(icon(Icon::ChevronRight).ml_auto().when(is_open, |i| i.rotate(90.0)));
            sidebar_menu_item(sb, title).child(button).when(is_open, |d| {
                d.child(sidebar_menu_sub(sb).children(subs.iter().map(|&sub| {
                    let active = demo.active.clone();
                    sidebar_menu_sub_item().child(
                        sidebar_menu_sub_button(SidebarMenuSubButtonSize::Md, sub == current)
                            .on_click(move |_| active.set(sub))
                            .child(text(sub).truncate()),
                    )
                })))
            })
        })))
}

fn nav_projects(sb: &SidebarState, demo: &Demo) -> Div {
    let t = theme();
    let current = demo.active.get();
    let items = PROJECTS.iter().map(|&(name, glyph)| {
        let active = demo.active.clone();
        sidebar_menu_item(sb, name)
            .child(
                sidebar_menu_button(
                    sb,
                    name,
                    SidebarMenuButtonProps {
                        is_active: name == current,
                        ..SidebarMenuButtonProps::default()
                    },
                )
                .pr(8.0)
                .on_click(move |_| active.set(name))
                .child(icon(glyph))
                .child(text(name).truncate()),
            )
            .child(sidebar_menu_action(sb, name, SidebarMenuButtonSize::Default, true).child(icon(Icon::Ellipsis)))
    });
    let more = sidebar_menu_item(sb, "More").child(
        sidebar_menu_button(sb, "More", SidebarMenuButtonProps::default())
            .text_color(t.sidebar_foreground.opacity(0.7))
            .child(icon(Icon::Ellipsis))
            .child(text("More").truncate()),
    );
    sidebar_group()
        .when(sb.icon_collapsed(), Styled::hidden)
        .child(sidebar_group_label(sb).child("Projects"))
        .child(sidebar_menu().children(items).child(more))
}

fn inset(sb: &SidebarState) -> Div {
    let t = theme();
    let placeholder = || div().bg(t.muted.opacity(0.5)).rounded_xl();
    let crumbs = breadcrumb().child(
        breadcrumb_list()
            .child(breadcrumb_item().child(breadcrumb_link("Building Your Application")))
            .child(breadcrumb_separator())
            .child(breadcrumb_item().child(breadcrumb_page("Data Fetching"))),
    );
    sidebar_inset(sb)
        .child(
            div()
                .flex()
                .h(if sb.icon_collapsed() { 12.0 } else { 16.0 })
                .shrink_0()
                .items_center()
                .gap(2.0)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(2.0)
                        .px(4.0)
                        .child(sidebar_trigger(sb).ml(-1.0))
                        .child(separator(Orientation::Vertical).mr(2.0).h(4.0))
                        .child(crumbs),
                ),
        )
        .child(
            div()
                .flex_col()
                .flex_1()
                .gap(4.0)
                .p(4.0)
                .pt(0.0)
                .child(
                    div()
                        .grid()
                        .grid_cols(3)
                        .gap(4.0)
                        .children((0..3).map(|_| placeholder().aspect(16.0 / 9.0))),
                )
                .child(placeholder().flex_1()),
        )
}

/// A fixed (`collapsible: none`) sidebar with an input, a group action,
/// badges, the outline variant, the three sizes, a small sub menu and skeletons.
fn parts(demo: &Demo) -> Div {
    let sb = &demo.parts;
    let props = |variant, size| SidebarMenuButtonProps {
        variant,
        size,
        ..SidebarMenuButtonProps::default()
    };
    let inbox = sidebar_menu()
        .child(
            sidebar_menu_item(sb, "Inbox")
                .child(
                    sidebar_menu_button(
                        sb,
                        "Inbox",
                        SidebarMenuButtonProps {
                            is_active: true,
                            ..props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Default)
                        },
                    )
                    .child(icon(Icon::Inbox))
                    .child(text("Inbox").truncate()),
                )
                .child(sidebar_menu_badge(sb, "Inbox", SidebarMenuButtonSize::Default).child("24")),
        )
        .child(
            sidebar_menu_item(sb, "Drafts")
                .child(
                    sidebar_menu_button(
                        sb,
                        "Drafts",
                        props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Sm),
                    )
                    .child(icon(Icon::Pencil))
                    .child(text("Drafts").truncate()),
                )
                .child(sidebar_menu_badge(sb, "Drafts", SidebarMenuButtonSize::Sm).child("3"))
                .child(
                    sidebar_menu_sub(sb)
                        .child(
                            sidebar_menu_sub_item().child(
                                sidebar_menu_sub_button(SidebarMenuSubButtonSize::Sm, true)
                                    .child(text("Replies").truncate()),
                            ),
                        )
                        .child(
                            sidebar_menu_sub_item().child(
                                sidebar_menu_sub_button(SidebarMenuSubButtonSize::Sm, false)
                                    .child(text("Notes").truncate()),
                            ),
                        ),
                ),
        )
        .child(
            sidebar_menu_item(sb, "Starred").child(
                sidebar_menu_button(
                    sb,
                    "Starred",
                    props(SidebarMenuButtonVariant::Outline, SidebarMenuButtonSize::Default),
                )
                .child(icon(Icon::Star))
                .child(text("Starred").truncate()),
            ),
        )
        .child(
            sidebar_menu_item(sb, "Archive").child(
                sidebar_menu_button(
                    sb,
                    "Archive",
                    props(SidebarMenuButtonVariant::Default, SidebarMenuButtonSize::Lg),
                )
                .child(icon(Icon::Download))
                .child(text("Archive (large)").truncate()),
            ),
        );
    sidebar_provider(sb).child(sidebar(
        sb,
        [
            sidebar_header().child(sidebar_input(&demo.search).placeholder("Search the docs...")),
            sidebar_separator(),
            sidebar_content(sb)
                .child(
                    sidebar_group()
                        .child(sidebar_group_label(sb).child("Mail"))
                        .child(sidebar_group_action(sb).child(icon(Icon::Plus)))
                        .child(sidebar_group_content().child(inbox)),
                )
                .child(
                    sidebar_group()
                        .child(sidebar_group_label(sb).child("Loading"))
                        .child(sidebar_menu().children(
                            ["one", "two"].map(|id| sidebar_menu_item(sb, id).child(sidebar_menu_skeleton(id, true))),
                        )),
                ),
        ],
    ))
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
    let sb = gui::use_context::<SidebarState>().ok_or_else(|| ScenarioError::Unhandled("no sidebar state".into()))?;
    s.act("⌘B", || sb.toggle());
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
