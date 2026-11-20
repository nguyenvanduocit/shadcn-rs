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
    Signal, div, icon, key_of, text, use_context,
};
use shadcn::{
    ButtonSize, ButtonVariant, CommandState, DialogState, Orientation, SidebarCollapsible, SidebarMenuButtonProps,
    SidebarMenuButtonSize, SidebarState, Theme, breadcrumb, breadcrumb_item, breadcrumb_link, breadcrumb_list,
    breadcrumb_page, breadcrumb_separator, button, command, command_empty, command_group, command_input, command_item,
    command_list, command_separator, dialog_content, kbd, kbd_group, separator, sidebar, sidebar_content,
    sidebar_group, sidebar_group_content, sidebar_group_label, sidebar_header, sidebar_inset, sidebar_menu,
    sidebar_menu_button, sidebar_menu_item, sidebar_provider, sidebar_trigger, theme,
};

use self::catalog::{Catalog, Route};
use self::component::DemoStates;
use self::fidelity::Verdicts;
use crate::{Host, Render, pages};

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

/// The search palette: a command list over every page, in a dialog.
#[derive(Clone)]
struct Search {
    dialog: DialogState,
    cmd: CommandState,
}

impl Search {
    fn open(&self) {
        self.cmd.search.set_text("");
        self.dialog.open();
        self.cmd.search.focus.focus();
    }

    fn toggle(&self) {
        if self.dialog.is_open() {
            self.dialog.close();
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
    /// Each gallery page's render, built the first time it is opened.
    renders: Vec<Option<Render>>,
    demos: DemoStates,
    sidebar: SidebarState,
    search: Search,
    verdicts: Signal<Rc<Verdicts>>,
}

impl Docs {
    fn new(start: Route) -> Self {
        let catalog = Rc::new(Catalog::load());
        Self {
            renders: pages::ALL.iter().map(|_| None).collect(),
            catalog,
            nav: Nav {
                route: Signal::labeled("docs route", start),
                scroll: ScrollHandle::new(),
            },
            demos: DemoStates::new(),
            sidebar: SidebarState::new(true).collapsible(SidebarCollapsible::Offcanvas),
            search: Search {
                dialog: DialogState::new(),
                cmd: CommandState::new(),
            },
            verdicts: Signal::labeled("verdicts", Rc::new(Verdicts::read())),
        }
    }

    fn render(&mut self) -> Element {
        let route = self.nav.route.get();
        let parts = self.parts(route);
        let search = self.search.clone();
        let t = theme();
        div()
            .named("docs")
            .size_full()
            .bg(t.background)
            .text_color(t.foreground)
            .on_key_down(move |e| {
                let hit = is_search_key(e);
                if hit {
                    search.toggle();
                }
                hit
            })
            .child(
                sidebar_provider(&self.sidebar).child(self.side(route)).child(
                    sidebar_inset(&self.sidebar)
                        .min_h(0.0)
                        .child(self.header(route))
                        .child(self.body(parts)),
                ),
            )
            .child(self.palette())
            .into_element()
    }

    /// The page's parts, then the links to the pages either side of it.
    fn parts(&mut self, route: Route) -> Vec<Part> {
        let verdicts = self.verdicts.get();
        let mut parts = match route {
            Route::Introduction => intro::parts(&self.catalog, &verdicts, &self.nav),
            Route::Theming => theming::parts(&source::module_doc(include_str!("../../../src/theme.rs"))),
            Route::Fidelity => {
                let signal = self.verdicts.clone();
                fidelity::parts(&self.catalog, &verdicts, &self.nav, move || {
                    signal.set(Rc::new(Verdicts::read()));
                })
            }
            Route::Page(i) => self.component(i, &verdicts),
        };
        parts.push(Part::new("pager", None, self.pager(route)));
        parts
    }

    fn component(&mut self, i: usize, verdicts: &Verdicts) -> Vec<Part> {
        let (Some(entry), Some(slot)) = (self.catalog.entries.get(i), self.renders.get_mut(i)) else {
            return Vec::new();
        };
        let render = slot.get_or_insert_with(|| (entry.page.render)());
        let lifted = lift::lift(render());
        component::parts(entry, lifted, &mut self.demos, verdicts)
    }

    /// Previous and next, as ui.shadcn.com ends each page.
    fn pager(&self, route: Route) -> Div {
        let (prev, next) = self.catalog.neighbours(route);
        let link = |to: Route, glyph: Icon, before: bool| {
            let nav = self.nav.clone();
            let title = self.catalog.title(to);
            button(ButtonVariant::Secondary, ButtonSize::Sm)
                .on_click(move |_| nav.go(to))
                .when(before, |b| b.child(icon(glyph)))
                .child(title)
                .when(!before, |b| b.child(icon(glyph)))
        };
        div()
            .flex()
            .items_center()
            .pt(6.0)
            .when_some(prev, |d, r| d.child(link(r, Icon::ArrowLeft, true)))
            .when_some(next, |d, r| {
                d.child(div().ml_auto().child(link(r, Icon::ArrowRight, false)))
            })
    }

    fn side(&self, current: Route) -> Div {
        let sb = &self.sidebar;
        let t = theme();
        let item = |route: Route| {
            let title = self.catalog.title(route);
            let nav = self.nav.clone();
            sidebar_menu_item(sb, &title).child(
                sidebar_menu_button(
                    sb,
                    &title,
                    SidebarMenuButtonProps {
                        is_active: route == current,
                        ..SidebarMenuButtonProps::default()
                    },
                )
                .h(7.5)
                .on_click(move |_| nav.go(route))
                .child(text(title.clone()).truncate()),
            )
        };
        let group = |label: &str, routes: &[Route]| {
            sidebar_group()
                .py(1.0)
                .child(sidebar_group_label(sb).child(label.to_owned()))
                .child(sidebar_group_content().child(sidebar_menu().gap(0.5).children(routes.iter().map(|&r| item(r)))))
        };
        let home = self.nav.clone();
        let brand = sidebar_menu().child(
            sidebar_menu_item(sb, "brand").child(
                sidebar_menu_button(
                    sb,
                    "brand",
                    SidebarMenuButtonProps {
                        size: SidebarMenuButtonSize::Lg,
                        ..SidebarMenuButtonProps::default()
                    },
                )
                .on_click(move |_| home.go(Route::Introduction))
                .child(
                    div()
                        .size(8.0)
                        .shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .bg(t.sidebar_primary)
                        .text_color(t.sidebar_primary_foreground)
                        .child(icon(Icon::Layers)),
                )
                .child(
                    div()
                        .flex_col()
                        .min_w(0.0)
                        .leading_tight()
                        .child(text("shadcn-rs").font_semibold())
                        .child(
                            text(format!("Docs · {} components", self.catalog.components.len()))
                                .text_xs()
                                .text_color(t.muted_foreground),
                        ),
                ),
            ),
        );
        sidebar(
            sb,
            [
                sidebar_header().child(brand).into_element(),
                sidebar_content(sb)
                    .gap(0.0)
                    .pb(4.0)
                    .child(group("Getting Started", &self.catalog.getting_started))
                    .child(group("Components", &self.catalog.components))
                    .into_element(),
            ],
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
        let find = button(ButtonVariant::Outline, ButtonSize::Sm)
            .named("docs-search")
            .w_px(256.0)
            .justify_start()
            .font_normal()
            .text_color(t.muted_foreground)
            .when(!t.dark, |b| b.bg(t.muted.opacity(0.5)))
            .on_click(move |_| search.open())
            .child(icon(Icon::Search))
            .child(text("Search documentation..."))
            .child(
                kbd_group()
                    .ml_auto()
                    .child(kbd().child(if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" }))
                    .child(kbd().child("K")),
            );
        let themes = use_context::<Signal<Theme>>();
        let toggle = button(ButtonVariant::Ghost, ButtonSize::Icon)
            .named("theme-toggle")
            .size(8.0)
            .on_click(move |_| {
                if let Some(s) = &themes {
                    s.update(|t| if t.dark { Theme::light() } else { Theme::dark() });
                }
            })
            .child(icon(if t.dark { Icon::Sun } else { Icon::Moon }));
        div()
            .named("docs-header")
            .flex()
            .items_center()
            .gap(2.0)
            .h(14.0)
            .shrink_0()
            .px(4.0)
            .border_b()
            .border_color(t.border)
            .child(sidebar_trigger(&self.sidebar).ml(-1.0))
            .child(separator(Orientation::Vertical).h(4.0).mr(2.0))
            .child(crumbs)
            .child(div().ml_auto().flex().items_center().gap(2.0).child(find).child(toggle))
    }

    /// The page's column, and "On this page" beside it.
    fn body(&self, parts: Vec<Part>) -> Div {
        let t = theme();
        let listed: Vec<(ElementKey, String)> = parts
            .iter()
            .filter_map(|p| p.toc.clone().map(|label| (p.key, label)))
            .collect();
        let keys: Vec<ElementKey> = listed.iter().map(|(k, _)| *k).collect();
        let reading = self.nav.reading(&keys);
        let column = div()
            .named("docs-main")
            .flex_1()
            .min_w(0.0)
            .h_full()
            .overflow_y_scroll()
            .track_scroll(&self.nav.scroll)
            .flex_col()
            .items_center()
            .gap(14.0)
            .px(10.0)
            .pt(8.0)
            .pb(16.0)
            .children(parts.into_iter().map(|p| {
                div()
                    .key(p.key)
                    .w_full()
                    .max_w_px(CONTENT_WIDTH)
                    .flex_col()
                    .child(p.body)
            }));
        let toc = (!listed.is_empty()).then(|| {
            div()
                .named("docs-toc")
                .w(56.0)
                .shrink_0()
                .h_full()
                .min_h(0.0)
                .overflow_y_scroll()
                .scrollbar_none()
                .flex_col()
                .gap(2.0)
                .pt(8.0)
                .pr(6.0)
                .pb(8.0)
                .text_sm()
                .child(text("On this page").font_medium().mb(1.0))
                .children(listed.into_iter().map(|(key, label)| {
                    let nav = self.nav.clone();
                    let on = reading == Some(key);
                    text(label)
                        .text_color(if on { t.foreground } else { t.muted_foreground })
                        .when(on, |l| l.font_medium())
                        .cursor_pointer()
                        .focusable()
                        .hover(move |s| s.text_color(t.foreground))
                        .focus_visible(move |s| s.ring(2.0, t.ring).rounded_sm())
                        .on_click(move |_| nav.reveal(key))
                }))
        });
        div().flex().flex_1().min_h(0.0).child(column).child(toc)
    }

    /// The command palette over every page.
    fn palette(&self) -> Element {
        let cmd = &self.search.cmd;
        let item = |route: Route| {
            let title = self.catalog.title(route);
            let (nav, dialog) = (self.nav.clone(), self.search.dialog.clone());
            let glyph = match route {
                Route::Page(_) if self.catalog.group(route) == "Components" => Icon::Layers,
                _ => Icon::FileText,
            };
            command_item(cmd, &title, false, move || {
                nav.go(route);
                dialog.close();
            })
            .map(|i| {
                i.child(icon(glyph).text_color(theme().muted_foreground))
                    .child(text(title.clone()))
            })
        };
        dialog_content(&self.search.dialog, |c| {
            // The root first: it starts the frame the items then register in.
            let root = command(cmd).child(command_input(cmd, "Search documentation..."));
            let started = command_group(
                Some("Getting Started"),
                self.catalog.getting_started.iter().map(|&r| item(r)),
            );
            let components = command_group(Some("Components"), self.catalog.components.iter().map(|&r| item(r)));
            let between = (started.is_some() && components.is_some()).then(command_separator);
            c.named("docs-palette")
                .p(0.0)
                .gap(0.0)
                .max_w_px(560.0)
                .overflow_hidden()
                .child(
                    root.child(
                        command_list()
                            .max_h_px(360.0)
                            .child(started)
                            .child(between)
                            .child(components)
                            .child(command_empty(cmd, "No results found.")),
                    ),
                )
        })
    }
}

/// Open the docs in a window with the debug tools `debug` turns on.
pub fn run(debug: gui::DebugTools) -> ! {
    gui::provide(Host::Docs);
    let mut docs = Docs::new(Route::Introduction);
    let options = RunOptions {
        debug,
        ..RunOptions::default()
    };
    gui::run("shadcn-rs docs", move || docs.render(), options)
}

/// The docs as a page's render function, opened at the introduction: what
/// `--inspect docs` drives headless.
pub fn app() -> Render {
    gui::provide(Host::Docs);
    let mut docs = Docs::new(Route::Introduction);
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
fn click_node<C: FnMut() -> Element>(
    s: &mut Scenario<C>,
    what: &str,
    pick: impl Fn(&[FrameNode], usize) -> bool,
) -> Result<(), ScenarioError> {
    let nodes = &s.last().nodes;
    let b = (0..nodes.len())
        .find(|&i| pick(nodes, i))
        .and_then(|i| nodes.get(i))
        .map(|n| n.bounds)
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
        .is_some_and(|n| matches!(&n.content, Content::Text { content, .. } if content == label))
}

/// Walk the docs headless at 1440×900, 2x, a frame per step into `dir`: the
/// Introduction, a component (Button), its Code tab, "On this page", the
/// palette (⌘K, "cal", Enter), dark mode, Theming, Fidelity and Tab focus.
/// Prints how many demos' code was found in their page's source.
pub fn trace(dir: &Path) -> Result<(), ScenarioError> {
    gui::provide(Host::Docs);
    coverage();
    let mut docs = Docs::new(Route::Introduction);
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
fn sweep<C: FnMut() -> Element>(s: &mut Scenario<C>, nav: &Nav, catalog: &Catalog) -> Result<(), ScenarioError> {
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
            let Some(box_) = preview.and_then(|p| nodes.get(p)).map(|p| p.bounds) else {
                continue;
            };
            let past = n.bounds.x < box_.x - 1.0 || n.bounds.right() > box_.right() + 1.0;
            let demo = std::iter::successors(Some(i), |&p| nodes.get(p).and_then(|p| p.parent))
                .find_map(|p| nodes.get(p).and_then(|p| p.name).filter(|name| name.ends_with(".vue")))
                .or(Some("playground"));
            if past {
                let line = format!("{}:{}", entry.page.name, demo.unwrap_or("?"));
                let list = if n.clip.is_none() {
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
