//! The docs: ui.shadcn.com/docs as a window. A sidebar of Getting Started and
//! every component A–Z; a header with the breadcrumb, a search that opens a
//! command palette (⌘K / Ctrl+K) and the light/dark switch; the page; and
//! "On this page" beside it.
//!
//! A component's page is its gallery page rendered with [`Host::Docs`]: each
//! demo is lifted out of it ([`lift`]) into a Preview | Code block whose code
//! is cut from the page's own source ([`source`]), so the code shown is the
//! code that runs.

mod catalog;
mod component;
mod export;
mod fidelity;
mod intro;
mod lift;
mod prose;
mod source;
mod theming;

use std::path::Path;
use std::rc::Rc;

use gui::layout::{Content, FrameNode};
use gui::prelude::*;
use gui::{
    Div, Element, ElementKey, Icon, KeyEvent, Modifiers, NamedKey, RunOptions, Scenario, ScenarioError, ScrollHandle,
    Signal, div, icon, key_of, text,
};
use shadcn::{
    ButtonSize, ButtonVariant, CommandItem, CommandSection, Orientation, SidebarCollapsible, SidebarMenuButtonProps,
    SidebarMenuButtonSize, breadcrumb, breadcrumb_item, breadcrumb_link, breadcrumb_list, breadcrumb_page,
    breadcrumb_separator, button, command, command_input, command_list, dialog, dialog_content, dialog_description,
    dialog_header, dialog_title, kbd, kbd_group, separator, sidebar, sidebar_content, sidebar_group,
    sidebar_group_content, sidebar_group_label, sidebar_header, sidebar_inset, sidebar_menu, sidebar_menu_button,
    sidebar_menu_item, sidebar_provider, sidebar_trigger, theme,
};

use self::catalog::{Catalog, Route};
use self::component::DemoStates;
use self::fidelity::Verdicts;
use crate::{Host, Render};
use shadcn::focus::StateLook;

pub use self::export::export;

/// The page column's widest: a 768px demo in its preview's padding and border.
const CONTENT_WIDTH: f32 = 820.0;
/// How far below the top of the page a part counts as the one being read.
const READING_LINE: f32 = 96.0;

/// One part of a page: what "On this page" lists (when titled) and scrolls to.
pub struct Part {
    key: ElementKey,
    toc: Option<String>,
    body: Element,
}

impl Part {
    fn new(id: &str, toc: Option<&str>, body: impl IntoElement) -> Self {
        Self::keyed(key_of(id), toc.map(str::to_owned), body)
    }

    fn keyed(key: ElementKey, toc: Option<String>, body: impl IntoElement) -> Self {
        Self {
            key,
            toc,
            body: body.into_element(),
        }
    }
}

/// Where the reader is, and the page's scroll.
#[derive(Clone)]
pub struct Nav {
    route: Signal<Route>,
    scroll: ScrollHandle,
}

impl Nav {
    /// Open `route` at its top.
    pub fn go(&self, route: Route) {
        self.scroll.scroll_to((0.0, 0.0));
        if self.route.get() != route {
            self.route.set(route);
        }
    }

    /// Scroll the part keyed `key` to the top.
    fn reveal(&self, key: ElementKey) {
        let Some(laid) = self.scroll.layout() else {
            return;
        };
        if let Some(part) = laid.children.iter().find(|c| c.key == Some(key)) {
            self.scroll
                .scroll_to((0.0, (part.bounds.y - laid.padding.top).max(0.0)));
        }
    }

    /// The part being read: the last of `listed` whose top has passed the
    /// reading line, or the last of them once the page is scrolled to its end.
    fn reading(&self, listed: &[ElementKey]) -> Option<ElementKey> {
        let laid = self.scroll.layout()?;
        let y = self.scroll.offset().1;
        let mut parts = laid
            .children
            .iter()
            .filter(|c| c.key.is_some_and(|k| listed.contains(&k)));
        if laid.max.1 > 0.0 && y >= laid.max.1 - 1.0 {
            return parts.next_back().and_then(|c| c.key);
        }
        parts
            .rfind(|c| c.bounds.y - laid.padding.top <= y + READING_LINE)
            .and_then(|c| c.key)
    }
}

/// The search palette: a command list over every page, in a dialog. Each
/// opening mounts a fresh command, its search empty and focused.
#[derive(Clone)]
struct Search {
    dialog: Signal<bool>,
}

impl Search {
    fn open(&self) {
        self.dialog.set(true);
    }

    fn toggle(&self) {
        if self.dialog.get_untracked() {
            self.dialog.set(false);
        } else {
            self.open();
        }
    }
}

/// ⌘K on macOS, Ctrl+K elsewhere.
fn is_search_key(e: &KeyEvent) -> bool {
    e.modifiers.primary() && matches!(&e.key, gui::Key::Character(c) if c.eq_ignore_ascii_case("k"))
}

/// The docs app and what it keeps between frames.
struct Docs {
    catalog: Rc<Catalog>,
    nav: Nav,
    search: Search,
    verdicts: Signal<Rc<Verdicts>>,
}

impl Docs {
    fn new(start: Route) -> Self {
        let catalog = Rc::new(Catalog::load());
        Self {
            catalog,
            nav: Nav {
                route: Signal::labeled("docs route", start),
                scroll: ScrollHandle::new(),
            },
            search: Search {
                dialog: Signal::labeled("search open", false),
            },
            verdicts: Signal::labeled("verdicts", Rc::new(Verdicts::read())),
        }
    }

    fn render(&self) -> Element {
        let route = self.nav.route.get();
        let body = match route {
            Route::Page(i) => page_view(Rc::clone(&self.catalog), self.nav.clone(), self.verdicts, i),
            _ => {
                let parts = self.parts(route);
                body(&self.nav, parts).into_element()
            }
        };
        let search = self.search.clone();
        let t = theme();
        div()
            .named("docs")
            .size(pct(100.0))
            .background(t.background)
            .text_color(t.foreground)
            .on_key_down(move |e| {
                let hit = is_search_key(e);
                if hit {
                    search.toggle();
                }
                hit
            })
            .child(
                sidebar_provider()
                    .collapsible(SidebarCollapsible::Offcanvas)
                    .child(self.side(route))
                    .child(sidebar_inset().min_height(0.0).child(self.header(route)).child(body)),
            )
            .child(self.palette())
            .into_element()
    }

    /// The page's parts, then the links to the pages either side of it.
    fn parts(&self, route: Route) -> Vec<Part> {
        let verdicts = self.verdicts.get();
        let mut parts = match route {
            Route::Introduction => intro::parts(&self.catalog, &verdicts, &self.nav),
            Route::Theming => theming::parts(&source::module_doc(include_str!("../../../src/theme.rs"))),
            Route::Fidelity => {
                let signal = self.verdicts;
                fidelity::parts(&self.catalog, &verdicts, &self.nav, move || {
                    signal.set(Rc::new(Verdicts::read()));
                })
            }
            Route::Page(_) => Vec::new(),
        };
        parts.push(Part::new("pager", None, pager(&self.catalog, &self.nav, route)));
        parts
    }

    fn side(&self, current: Route) -> impl IntoElement {
        let t = theme();
        let item = |route: Route| {
            let title = self.catalog.title(route);
            let nav = self.nav.clone();
            sidebar_menu_item().child(
                sidebar_menu_button(SidebarMenuButtonProps {
                    is_active: route == current,
                    ..SidebarMenuButtonProps::default()
                })
                .height(30.0)
                .on_click(move |_| nav.go(route))
                .child(
                    text(title)
                        .truncate(true)
                        .nowrap(true)
                        .overflow_x(Overflow::Hidden)
                        .min_width(0.0),
                ),
            )
        };
        let group = |label: &str, routes: &[Route]| {
            sidebar_group()
                .padding_y(4.0)
                .child(sidebar_group_label().child(label.to_owned()))
                .child(sidebar_group_content().child(sidebar_menu().gap(2.0).children(routes.iter().map(|&r| item(r)))))
        };
        let home = self.nav.clone();
        let brand = sidebar_menu().child(
            sidebar_menu_item().child(
                sidebar_menu_button(SidebarMenuButtonProps {
                    size: SidebarMenuButtonSize::Lg,
                    ..SidebarMenuButtonProps::default()
                })
                .on_click(move |_| home.go(Route::Introduction))
                .child(
                    div()
                        .size(32.0)
                        .flex_shrink(0.0)
                        .display(Display::Flex)
                        .align_items(Align::Center)
                        .justify_content(Justify::Center)
                        .radius(10.0)
                        .background(t.sidebar_primary)
                        .text_color(t.sidebar_primary_foreground)
                        .child(icon(Icon::Layers)),
                )
                .child(
                    div()
                        .display(Display::Flex)
                        .flex_direction(FlexDirection::Column)
                        .min_width(0.0)
                        .line_height(LineHeight::Relative(1.25))
                        .child(text("shadcn-rs").font_weight(600))
                        .child(
                            text(format!("Docs · {} components", self.catalog.components.len()))
                                .text_size(12.0)
                                .line_height(16.0)
                                .text_color(t.muted_foreground),
                        ),
                ),
            ),
        );
        sidebar().child(sidebar_header().child(brand)).child(
            sidebar_content()
                .gap(0.0)
                .padding_bottom(16.0)
                .child(group("Getting Started", &self.catalog.getting_started))
                .child(group("Components", &self.catalog.components)),
        )
    }

    fn header(&self, route: Route) -> Div {
        let t = theme();
        let crumbs = {
            let nav = self.nav.clone();
            breadcrumb().child(
                breadcrumb_list()
                    .child(
                        breadcrumb_item().child(breadcrumb_link("Docs").on_click(move |_| nav.go(Route::Introduction))),
                    )
                    .child(breadcrumb_separator())
                    .child(breadcrumb_item().child(text(self.catalog.group(route))))
                    .child(breadcrumb_separator())
                    .child(breadcrumb_item().child(breadcrumb_page(self.catalog.title(route)))),
            )
        };
        let search = self.search.clone();
        let find = button(ButtonVariant::Outline, ButtonSize::Sm);
        let i = find.interaction();
        // The outline button's hover (its fill and text) over the fixed look.
        let find = find
            .named("docs-search")
            .width(256.0)
            .justify_content(Justify::Start)
            .font_weight(400)
            .text_color(t.muted_foreground)
            .text_color_while(move || i.hovered(), t.accent_foreground)
            .when(!t.dark, |b| {
                b.background(t.muted.opacity(0.5))
                    .background_while(move || i.hovered(), t.accent)
            })
            .on_click(move |_| search.open())
            .child(icon(Icon::Search))
            .child(text("Search documentation..."))
            .child(
                kbd_group()
                    .margin_left(Length::Auto)
                    .child(kbd().child(if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }))
                    .child(kbd().child("K")),
            );
        let themes = shadcn::use_theme();
        let toggle = button(ButtonVariant::Ghost, ButtonSize::Icon)
            .named("theme-toggle")
            .size(32.0)
            .on_click(move |_| themes.update(|t| t.scheme = t.scheme.toggled()))
            .child(icon(if t.dark { Icon::Sun } else { Icon::Moon }));
        div()
            .named("docs-header")
            .display(Display::Flex)
            .align_items(Align::Center)
            .gap(8.0)
            .height(56.0)
            .flex_shrink(0.0)
            .padding_x(16.0)
            .border_bottom(1.0)
            .border_color(t.border)
            .child(sidebar_trigger().margin_left(-4.0))
            .child(separator(Orientation::Vertical).height(16.0).margin_right(8.0))
            .child(crumbs)
            .child(
                div()
                    .margin_left(Length::Auto)
                    .display(Display::Flex)
                    .align_items(Align::Center)
                    .gap(8.0)
                    .child(find)
                    .child(toggle),
            )
    }

    /// The page's column, and "On this page" beside it.
    /// The command palette over every page.
    fn palette(&self) -> Element {
        let titled = |routes: &[Route]| -> Vec<CommandItem> {
            routes
                .iter()
                .map(|&r| CommandItem::new(self.catalog.title(r)))
                .collect()
        };
        let sections = std::sync::Arc::new(vec![
            CommandSection::group(Some("Getting Started"), titled(&self.catalog.getting_started)),
            CommandSection::Separator,
            CommandSection::group(Some("Components"), titled(&self.catalog.components)),
            CommandSection::empty("No results found."),
        ]);
        let routes: Vec<(String, Route)> = self
            .catalog
            .getting_started
            .iter()
            .chain(&self.catalog.components)
            .map(|&r| (self.catalog.title(r), r))
            .collect();
        let glyphs: Vec<(String, Icon)> = routes
            .iter()
            .map(|(title, route)| {
                let glyph = match route {
                    Route::Page(_) if self.catalog.group(*route) == "Components" => Icon::Layers,
                    _ => Icon::FileText,
                };
                (title.clone(), glyph)
            })
            .collect();
        let (nav, open) = (self.nav.clone(), self.search.dialog);
        let on_select = move |item: CommandItem| {
            if let Some(&(_, route)) = routes.iter().find(|(t, _)| *t == item.value) {
                nav.go(route);
                open.set(false);
            }
        };
        let row = move |item: &CommandItem, row: Div| {
            let glyph = glyphs
                .iter()
                .find(|(t, _)| *t == item.value)
                .map_or(Icon::FileText, |&(_, g)| g);
            row.child(icon(glyph).text_color(theme().muted_foreground))
                .child(text(item.value.clone()))
        };
        dialog()
            .open(self.search.dialog)
            .child(
                dialog_content()
                    .map_root(|d| d.named("docs-palette"))
                    .padding(0.0)
                    .gap(0.0)
                    .max_width(560.0)
                    .overflow(Overflow::Hidden)
                    // `CommandDialog`'s `DialogHeader class="sr-only"`: read, not shown.
                    .child(
                        primitives::visually_hidden::visually_hidden(
                            dialog_header(),
                            primitives::visually_hidden::Hidden::Visually,
                        )
                        .child(dialog_title("Search documentation"))
                        .child(dialog_description("Search for a page to open.")),
                    )
                    .child(
                        command()
                            .child(command_input("Search documentation..."))
                            .child(command_list(sections, row).on_select(on_select).max_h_px(360.0)),
                    ),
            )
            .into_element()
    }
}

/// Open the docs in a window with the debug tools `debug` turns on;
/// `setup` runs first on the UI thread, where the app's state lives
/// (ADR-0015), to provide what the docs read (the theme).
pub fn run(debug: gui::DebugTools, setup: impl FnOnce() + Send + 'static) -> ! {
    let options = RunOptions {
        debug,
        ..RunOptions::default()
    };
    gui::run(
        "shadcn-rs docs",
        move || {
            setup();
            gui::provide(Host::Docs);
            let docs = Docs::new(Route::Introduction);
            move || docs.render()
        },
        options,
    )
}

/// The docs as a page's render function, opened at the introduction: what
/// `--inspect docs` drives headless.
pub fn app() -> Render {
    gui::provide(Host::Docs);
    let docs = Docs::new(Route::Introduction);
    Box::new(move || docs.render())
}

/// The platform's shortcut modifier held alone.
fn primary() -> Modifiers {
    Modifiers {
        command: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Modifiers::default()
    }
}

/// Click the centre of the first node in the last frame that `pick` takes.
fn click_node(s: &mut Scenario, what: &str, pick: impl Fn(&[FrameNode], usize) -> bool) -> Result<(), ScenarioError> {
    let nodes = &s.last().nodes;
    let b = (0..nodes.len())
        .find(|&i| pick(nodes, i))
        .and_then(|i| nodes.get(i))
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .ok_or_else(|| ScenarioError::TextNotFound(what.to_owned()))?;
    s.click(b.x + b.width / 2.0, b.y + b.height / 2.0);
    Ok(())
}

/// Whether node `i` is, or is inside, the node named `name`.
fn within(nodes: &[FrameNode], i: usize, name: &str) -> bool {
    std::iter::successors(Some(i), |&n| nodes.get(n).and_then(|n| n.parent))
        .any(|n| nodes.get(n).is_some_and(|n| n.name == Some(name)))
}

/// Whether node `i` shows exactly `label`.
fn shows(nodes: &[FrameNode], i: usize, label: &str) -> bool {
    nodes
        .get(i)
        .is_some_and(|n| matches!(&n.content, Content::Text { content, .. } if &**content == label))
}

/// Walk the docs headless at 1440×900, 2x, a frame per step into `dir`: the
/// Introduction, a component (Button), its Code tab, "On this page", the
/// palette (⌘K, "cal", Enter), dark mode, Theming, Fidelity and Tab focus.
/// Prints how many demos' code was found in their page's source.
pub fn trace(dir: &Path) -> Result<(), ScenarioError> {
    gui::provide(Host::Docs);
    coverage();
    let docs = Docs::new(Route::Introduction);
    let (nav, catalog) = (docs.nav.clone(), Rc::clone(&docs.catalog));
    let mut s = Scenario::new(move || docs.render(), dir, 1440, 900, 2.0)?;
    s.frame("introduction")?;
    click_node(&mut s, "Button in the sidebar", |n, i| {
        shows(n, i, "Button") && within(n, i, "sidebar")
    })?;
    s.frame("button")?;
    s.click_text("Code")?;
    s.frame("button code tab")?;
    click_node(&mut s, "Outline in On this page", |n, i| {
        shows(n, i, "Outline") && within(n, i, "docs-toc")
    })?;
    s.frame("toc outline")?;
    s.shortcut('k', primary());
    s.frame("palette open")?;
    s.settle()?;
    s.type_text("cal");
    s.frame("palette cal")?;
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.frame("palette arrow down")?;
    s.press(NamedKey::ArrowUp, Modifiers::default());
    s.press(NamedKey::Enter, Modifiers::default());
    s.frame("calendar")?;
    click_node(&mut s, "theme-toggle", |n, i| {
        n.get(i).is_some_and(|n| n.name == Some("theme-toggle"))
    })?;
    s.frame("calendar dark")?;
    click_node(&mut s, "Theming in the sidebar", |n, i| {
        shows(n, i, "Theming") && within(n, i, "sidebar")
    })?;
    s.frame("theming dark")?;
    click_node(&mut s, "Fidelity in the sidebar", |n, i| {
        shows(n, i, "Fidelity") && within(n, i, "sidebar")
    })?;
    s.frame("fidelity dark")?;
    s.shortcut('k', primary());
    s.settle()?;
    s.type_text("zzz");
    s.frame("palette no results")?;
    s.press(NamedKey::Escape, Modifiers::default());
    s.frame("palette escaped")?;
    for _ in 0..3 {
        s.press(NamedKey::Tab, Modifiers::default());
    }
    s.frame("tab focus")?;
    click_node(&mut s, "theme-toggle", |n, i| {
        n.get(i).is_some_and(|n| n.name == Some("theme-toggle"))
    })?;
    sweep(&mut s, &nav, &catalog)?;
    println!("docs trace: {}", dir.display());
    Ok(())
}

/// Open every component page and print each demo (or playground) whose
/// content reaches past its box unclipped; frame the pages in [`SWEEP_FRAMES`].
fn sweep(s: &mut Scenario, nav: &Nav, catalog: &Catalog) -> Result<(), ScenarioError> {
    const SWEEP_FRAMES: &[&str] = &[
        "sidebar",
        "typography",
        "chart",
        "table",
        "form_formisch",
        "badge",
        "resizable",
    ];
    let (mut overflowing, mut scrolled) = (Vec::new(), Vec::new());
    for route in catalog.order() {
        let Some(entry) = catalog.entry(route) else {
            continue;
        };
        s.act(format!("open {}", entry.page.name), || nav.go(route));
        s.settle()?;
        s.settle()?;
        let nodes = &s.last().nodes;
        for (i, n) in nodes.iter().enumerate() {
            let preview = std::iter::successors(n.parent, |&p| nodes.get(p).and_then(|p| p.parent)).find(|&p| {
                nodes.get(p).is_some_and(|p| {
                    matches!(p.name, Some("docs-preview" | component::PLAYGROUND)) && p.layer == n.layer
                })
            });
            let outside = gui::inspect::OUTSIDE;
            let Some(box_) = preview.and_then(|p| nodes.get(p)).map(|p| p.bounds(outside)) else {
                continue;
            };
            let b = n.bounds(outside);
            let past = b.x < box_.x - 1.0 || b.right() > box_.right() + 1.0;
            let demo = std::iter::successors(Some(i), |&p| nodes.get(p).and_then(|p| p.parent))
                .find_map(|p| nodes.get(p).and_then(|p| p.name).filter(|name| name.ends_with(".vue")))
                .or(Some("playground"));
            if past {
                let line = format!("{}:{}", entry.page.name, demo.unwrap_or("?"));
                let list = if n.clip(outside).is_none() {
                    &mut overflowing
                } else {
                    &mut scrolled
                };
                if !list.contains(&line) {
                    list.push(line);
                }
            }
        }
        if SWEEP_FRAMES.contains(&entry.page.name) {
            s.frame(&format!("page {}", entry.page.name))?;
        }
    }
    println!("reaching past their box: {} {overflowing:?}", overflowing.len());
    println!(
        "wider than their box, scrolled sideways: {} {scrolled:?}",
        scrolled.len()
    );
    Ok(())
}

/// Print, over every page, how many demos' code the Code tab finds: as the
/// `demo("<File>.vue", …)` call, through a list mapped through `demo`, or not.
fn coverage() {
    let catalog = Catalog::load();
    let (mut exact, mut listed, mut missing) = (0_u32, 0_u32, Vec::new());
    for entry in &catalog.entries {
        let lifted = lift::lift((entry.page.render)()());
        for (file, _) in lifted.demos {
            match source::demo_code(&entry.source, file) {
                Some(c) if c.exact => exact = exact.saturating_add(1),
                Some(_) => listed = listed.saturating_add(1),
                None => missing.push(format!("{}:{file}", entry.page.name)),
            }
        }
    }
    let total = exact
        .saturating_add(listed)
        .saturating_add(u32::try_from(missing.len()).unwrap_or(u32::MAX));
    println!(
        "Code tab over {total} demos: {exact} cut at their demo() call, {listed} through a list mapped by demo(), \
         {} not in their page's file {missing:?}",
        missing.len(),
    );
}

/// A gallery page (ADR-0013): an instance whose setup makes the page's state
/// and its demos' once, unmounted (and its state dropped) when the reader
/// leaves the page. Only it renders when what it shows changes.
fn page_view(catalog: Rc<Catalog>, nav: Nav, verdicts: Signal<Rc<Verdicts>>, i: usize) -> Element {
    gui::component(i, move |_| {
        let render = catalog.entries.get(i).map(|e| (e.page.render)());
        // Each demo's code, cut once here from the demos the page shows.
        let demos = match (catalog.entries.get(i), render.as_ref()) {
            (Some(entry), Some(render)) => component::demo_states(entry, &gui::untracked(|| lift::lift(render()))),
            _ => DemoStates::new(),
        };
        move || {
            let (Some(entry), Some(render)) = (catalog.entries.get(i), render.as_ref()) else {
                return gui::Element::Empty;
            };
            let lifted = lift::lift(render());
            let mut parts = component::parts(entry, lifted, &demos, &verdicts.get());
            parts.push(Part::new("pager", None, pager(&catalog, &nav, Route::Page(i))));
            body(&nav, parts).into_element()
        }
    })
    .key(gui::ElementKey::try_from(i).unwrap_or(0))
    .into_element()
}

/// Previous and next, as ui.shadcn.com ends each page.
fn pager(catalog: &Catalog, nav: &Nav, route: Route) -> Div {
    let (prev, next) = catalog.neighbours(route);
    let link = |to: Route, glyph: Icon, before: bool| {
        let nav = nav.clone();
        let title = catalog.title(to);
        button(ButtonVariant::Secondary, ButtonSize::Sm)
            .on_click(move |_| nav.go(to))
            .when(before, |b| b.child(icon(glyph)))
            .child(title)
            .when(!before, |b| b.child(icon(glyph)))
    };
    div()
        .display(Display::Flex)
        .align_items(Align::Center)
        .padding_top(24.0)
        .when_some(prev, |d, r| d.child(link(r, Icon::ArrowLeft, true)))
        .when_some(next, |d, r| {
            d.child(div().margin_left(Length::Auto).child(link(r, Icon::ArrowRight, false)))
        })
}

/// The page's column and its "On this page" list.
fn body(nav: &Nav, parts: Vec<Part>) -> Div {
    let t = theme();
    let listed: Vec<(ElementKey, String)> = parts
        .iter()
        .filter_map(|p| p.toc.clone().map(|label| (p.key, label)))
        .collect();
    let keys: Vec<ElementKey> = listed.iter().map(|(k, _)| *k).collect();
    let reading = nav.reading(&keys);
    let column = div()
        .named("docs-main")
        .flex(Flex::grow(1.0))
        .min_width(0.0)
        .height(pct(100.0))
        .overflow_y(Overflow::Scroll)
        .overflow_x(Overflow::Hidden)
        .track_scroll(&nav.scroll)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .align_items(Align::Center)
        .gap(56.0)
        .padding_x(40.0)
        .padding_top(32.0)
        .padding_bottom(64.0)
        .children(parts.into_iter().map(|p| {
            div()
                .key(p.key)
                .width(pct(100.0))
                .max_width(CONTENT_WIDTH)
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .child(p.body)
        }));
    let toc = (!listed.is_empty()).then(|| {
        div()
            .named("docs-toc")
            .width(224.0)
            .flex_shrink(0.0)
            .height(pct(100.0))
            .min_height(0.0)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden)
            .scrollbar_hidden(true)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(8.0)
            .padding_top(32.0)
            .padding_right(24.0)
            .padding_bottom(32.0)
            .text_size(14.0)
            .line_height(20.0)
            .child(text("On this page").font_weight(500).margin_bottom(4.0))
            .children(listed.into_iter().map(|(key, label)| {
                let nav = nav.clone();
                let on = reading == Some(key);
                text(label)
                    .text_color(if on { t.foreground } else { t.muted_foreground })
                    .when(on, |l| l.font_weight(500))
                    .cursor(Cursor::Pointer)
                    .focusable()
                    .with_interaction(|mut d, i| {
                        let (ring, shape) = (d.style_mut().ring, d.style_mut().shape);
                        let shown = Ring::new(2.0, t.ring);
                        d.text_color_while(move || i.hovered(), t.foreground)
                            .ring(move || if i.focus_visible() { Some(shown) } else { ring })
                            .shape(move || {
                                if i.focus_visible() {
                                    Shape::Rounded(Corners::all(6.0))
                                } else {
                                    shape
                                }
                            })
                    })
                    .on_click(move |_| nav.reveal(key))
            }))
    });
    div()
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .min_height(0.0)
        .child(column)
        .child(toc)
}
