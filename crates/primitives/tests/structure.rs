//! The structure primitives in a window, by behaviour (research §12,
//! ADR-0025 group 8): Collapsible, Accordion, Tabs, Pagination, Avatar,
//! AspectRatio, ScrollArea, Splitter and Carousel. Each: its keyboard table,
//! its timings, controlled and uncontrolled, disabled, and what a screen
//! reader is told of it.

#![expect(
    clippy::disallowed_types,
    reason = "test and example state the test reads back, not a component's"
)]
#![expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a scenario that cannot run, or a node not found, is a failed test; counters are small"
)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::{Scenario, ScenarioError};
use gui::events::{Modifiers, NamedKey};
use gui::list::ListState;
use gui::prelude::*;
use gui::{Axis, Bind, Bounds, Direction, Div, Element, FocusHandle, Signal, component, div, text, use_model};
use primitives::Availability;
use primitives::accordion::{Accordion, AccordionOptions, SectionItem};
use primitives::aspect_ratio::frame;
use primitives::avatar::{Avatar, Fallback, ImageLoad, Status};
use primitives::carousel::{Carousel, CarouselOptions, Rotation, SlideKey};
use primitives::collapsible::Collapsible;
use primitives::pagination::{Entry, Pagination, PaginationOptions, Step};
use primitives::scroll_area::{Bars, LINE_STEP, Policy, ScrollArea, ScrollAreaOptions, VirtualizerOptions};
use primitives::splitter::{Constraints, Splitter, SplitterOptions};
use primitives::tabs::{Activation, Tabs, TabsOptions};

/// `f` in a window, in a box of its own: the window's root fills it, the
/// box sizes to what is inside.
fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let wrapped = move || div().align_items(Align::Start).child(div().child(f())).into_element();
    let mut s = Scenario::measuring(wrapped, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

fn key(s: &mut Scenario, k: NamedKey, modifiers: Modifiers) {
    s.press(k, modifiers);
    s.settle().expect("settled");
}

fn plain(s: &mut Scenario, k: NamedKey) {
    key(s, k, Modifiers::default());
}

fn shift(s: &mut Scenario, k: NamedKey) {
    key(
        s,
        k,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
}

fn click(s: &mut Scenario, label: &str) {
    s.click_text(label).expect("a press reaches a handler");
    s.settle().expect("settled");
}

/// Press the text `label` where nothing may take a click: a disabled
/// control, or one acting as the button goes down.
fn press(s: &mut Scenario, label: &str) {
    if let Err(e) = s.click_text(label) {
        assert!(matches!(e, ScenarioError::Unhandled(_)), "{label:?} is there: {e:?}");
    }
    s.settle().expect("settled");
}

fn focused(h: FocusHandle) -> bool {
    gui::focus::focused() == Some(h.id())
}

fn focus(s: &mut Scenario, h: FocusHandle) {
    s.act("focus", || gui::focus::set_focus(Some(h.id()), true));
    s.settle().expect("settled");
}

/// The bounds of the node named `name` in the last frame.
fn bounds(s: &Scenario, name: &str) -> Option<Bounds> {
    s.last().nodes.iter().find(|n| n.name == Some(name)).map(|n| n.bounds)
}

fn has(s: &Scenario, name: &str) -> bool {
    bounds(s, name).is_some()
}

fn a11y(s: &Scenario) -> String {
    let mut ts = gui::text::TextSystem::new();
    dump(&gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts))
}

fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The tree from the window down, a line per node: role, name, state and
/// relations by their targets' names.
fn dump(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let named = |id: &NodeId| {
        nodes
            .get(id)
            .and_then(|n| name(n).or_else(|| Some(format!("{:?}", n.role()))))
            .unwrap_or_else(|| "?".to_owned())
    };
    let mut out = String::new();
    let mut stack = vec![(NodeId(0), 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
        if let Some(nm) = name(n) {
            line.push_str(&format!(" {nm:?}"));
        }
        if let Some(d) = n.role_description() {
            line.push_str(&format!(" described={d:?}"));
        }
        if let Some(l) = n.level() {
            // AccessKit's level counts from 0, `aria-level` from 1.
            line.push_str(&format!(" aria-level={}", l + 1));
        }
        if let Some(o) = n.orientation() {
            line.push_str(&format!(" {o:?}"));
        }
        if let Some(e) = n.is_expanded() {
            line.push_str(if e { " expanded" } else { " collapsed" });
        }
        if let Some(sel) = n.is_selected() {
            line.push_str(if sel { " selected" } else { " unselected" });
        }
        if let Some(c) = n.aria_current() {
            line.push_str(&format!(" current={c:?}"));
        }
        if let Some(l) = n.live() {
            line.push_str(&format!(" live={l:?}"));
        }
        if let (Some(min), Some(max)) = (n.min_numeric_value(), n.max_numeric_value()) {
            let now = n.numeric_value().map_or_else(|| "none".to_owned(), |v| v.to_string());
            line.push_str(&format!(" value={now} in {min}..={max}"));
        }
        if n.is_disabled() {
            line.push_str(" disabled");
        }
        for (what, ids) in [("labelled-by", n.labelled_by()), ("controls", n.controls())] {
            if !ids.is_empty() {
                let names: Vec<String> = ids.iter().map(named).collect();
                line.push_str(&format!(" {what}={}", names.join(",")));
            }
        }
        out.push_str(&line);
        out.push('\n');
        for c in n.children().iter().rev() {
            stack.push((*c, depth + 1));
        }
    }
    out
}

fn label(name: &'static str, s: &str) -> Div {
    div().named(name).width(100.0).height(24.0).child(text(s.to_owned()))
}

// ---------------------------------------------------------------- Collapsible

type Held<T> = Rc<Cell<Option<T>>>;

fn collapsible(
    open: Bind<bool>,
    disabled: Signal<bool>,
    keep: Signal<bool>,
    held: &Held<Collapsible<Bind<bool>>>,
) -> Element {
    let held = Rc::clone(held);
    component(open, move |cx| {
        let c = Collapsible::new(
            use_model(cx.props(), |b: &Bind<bool>| b),
            move || Availability::from_disabled(disabled.get()),
            move || keep.get(),
        );
        held.set(Some(c));
        move || {
            div()
                .child(c.trigger(label("trigger", "Toggle")))
                .child(c.content(label("content", "Body")))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn collapsible_opens_by_its_trigger_unless_disabled() {
    let (disabled, keep) = (Signal::new(false), Signal::new(false));
    let held: Held<Collapsible<Bind<bool>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || collapsible(Bind::default_value(false), disabled, keep, &h));
    let c = held.get().expect("made");
    assert!(!has(&s, "content"), "closed: the content is gone");
    click(&mut s, "Toggle");
    assert!(c.is_open() && has(&s, "content"));
    // Enter and Space press the trigger (a button).
    plain(&mut s, NamedKey::Enter);
    assert!(!c.is_open(), "Enter toggles");
    plain(&mut s, NamedKey::Space);
    assert!(c.is_open(), "Space toggles");
    // Disabled: the user cannot, the caller can.
    s.act("disable", || disabled.set(true));
    s.settle().expect("settled");
    press(&mut s, "Toggle");
    assert!(c.is_open(), "a disabled trigger does nothing");
    s.act("close", || c.set_open(false));
    s.settle().expect("settled");
    assert!(!c.is_open(), "the caller still opens and closes it");
    // Kept mounted: there while closed, hidden from a screen reader.
    s.act("keep", || keep.set(true));
    s.settle().expect("settled");
    assert!(has(&s, "content"), "kept mounted");
    assert!(!a11y(&s).contains("Body"), "kept content is not read while closed");
}

#[test]
fn collapsible_controlled_by_a_value_only_asks() {
    let heard: Rc<RefCell<Vec<bool>>> = Rc::default();
    let h2 = Rc::clone(&heard);
    let (disabled, keep) = (Signal::new(false), Signal::new(false));
    let held: Held<Collapsible<Bind<bool>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h2 = Rc::clone(&h2);
        collapsible(
            Bind::value(false).on_change(move |v| h2.borrow_mut().push(v)),
            disabled,
            keep,
            &h,
        )
    });
    let c = held.get().expect("made");
    click(&mut s, "Toggle");
    assert_eq!(*heard.borrow(), [true], "the caller is asked");
    assert!(!c.is_open(), "and what it holds is shown");
}

#[test]
fn collapsible_a11y() {
    let (disabled, keep) = (Signal::new(false), Signal::new(false));
    let held: Held<Collapsible<Bind<bool>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || collapsible(Bind::default_value(true), disabled, keep, &h));
    insta::assert_snapshot!("collapsible_a11y_open", a11y(&s));
    click(&mut s, "Toggle");
    insta::assert_snapshot!("collapsible_a11y_closed", a11y(&s));
}

// ------------------------------------------------------------------ Accordion

#[derive(Clone, Copy)]
struct Sections {
    accordion: Accordion<&'static str>,
}

fn accordion(open: Bind<Option<&'static str>>, options: Signal<AccordionOptions>, held: &Held<Sections>) -> Element {
    let held = Rc::clone(held);
    component(open, move |cx| {
        let accordion = Accordion::single(
            use_model(cx.props(), |b: &Bind<Option<&'static str>>| b),
            move || options.get(),
            Direction::Ltr,
        );
        held.set(Some(Sections { accordion }));
        let section = |value: &'static str, availability| {
            accordion.section(SectionItem {
                availability,
                ..SectionItem::new(value)
            })
        };
        let sections = [
            section("a", Availability::Enabled),
            section("b", Availability::Disabled),
            section("c", Availability::Enabled),
        ];
        move || {
            div()
                .children(sections.iter().zip(["A", "B", "C"]).map(|(sec, name)| {
                    div()
                        .child(sec.header(div()).child(sec.trigger(label("trigger", name))))
                        .child(sec.content(label("content", &format!("{name} body"))))
                        .into_element()
                }))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn accordion_single_collapsible_and_not() {
    let options = Signal::new(AccordionOptions::default());
    let held: Held<Sections> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || accordion(Bind::default_value(None), options, &h));
    let a = held.get().expect("made").accordion;
    click(&mut s, "A");
    assert!(a.is_open(&"a"));
    click(&mut s, "A");
    assert!(a.is_open(&"a"), "one open, not collapsible: it stays open");
    click(&mut s, "C");
    assert!(a.is_open(&"c") && !a.is_open(&"a"), "one at most");
    s.act("collapsible", || {
        options.set(AccordionOptions {
            collapsible: true,
            ..AccordionOptions::default()
        });
    });
    s.settle().expect("settled");
    click(&mut s, "C");
    assert!(!a.is_open(&"c"), "the option counts as soon as it changes");
    press(&mut s, "B");
    assert!(!a.is_open(&"b"), "a disabled section does not open");
}

#[test]
fn accordion_keys_walk_its_own_usable_triggers() {
    let options = Signal::new(AccordionOptions::default());
    let held: Held<Sections> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        div()
            .child(accordion(Bind::default_value(None), options, &h))
            .into_element()
    });
    let a = held.get().expect("made").accordion;
    click(&mut s, "A");
    let at = || {
        let f = gui::focus::focused();
        a.triggers()
            .with(|list| list.iter().position(|t| Some(t.focus.id()) == f))
    };
    assert_eq!(at(), Some(0));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(at(), Some(2), "ArrowDown skips the disabled B");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(at(), Some(0), "and wraps");
    plain(&mut s, NamedKey::ArrowUp);
    assert_eq!(at(), Some(2), "ArrowUp wraps back");
    plain(&mut s, NamedKey::Home);
    assert_eq!(at(), Some(0), "Home: the first usable");
    plain(&mut s, NamedKey::End);
    assert_eq!(at(), Some(2), "End: the last usable, not the disabled one");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(at(), Some(2), "vertical: across arrows do nothing");
    plain(&mut s, NamedKey::Enter);
    assert!(a.is_open(&"c"), "Enter on a trigger opens its section");
}

#[test]
fn accordion_a11y() {
    let options = Signal::new(AccordionOptions::default());
    let held: Held<Sections> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || accordion(Bind::default_value(Some("a")), options, &h));
    insta::assert_snapshot!("accordion_a11y", a11y(&s));
}

// ----------------------------------------------------------------------- Tabs

type TabsHeld = Held<(Tabs<Bind<&'static str>, &'static str>, [FocusHandle; 3])>;

fn tabs(value: Bind<&'static str>, options: Signal<TabsOptions>, held: &TabsHeld) -> Element {
    let held = Rc::clone(held);
    component(value, move |cx| {
        let tabs = Tabs::new(
            use_model(cx.props(), |b: &Bind<&'static str>| b),
            move || options.get(),
            Direction::Ltr,
        );
        let focus = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
        held.set(Some((tabs, focus)));
        move || {
            let tab =
                |i: usize, v: &'static str, name: &'static str, a| tabs.trigger(label("tab", name), v, a, focus[i]);
            div()
                .child(
                    tabs.list(div().display(Display::Flex))
                        .child(tab(0, "a", "One", Availability::Enabled))
                        .child(tab(1, "b", "Two", Availability::Disabled))
                        .child(tab(2, "c", "Three", Availability::Enabled)),
                )
                .child(tabs.panel(label("panel", "Panel one"), &"a"))
                .child(tabs.panel(label("panel", "Panel two"), &"b"))
                .child(tabs.panel(label("panel", "Panel three"), &"c"))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn tabs_automatic_selects_as_focus_moves_and_skips_disabled() {
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || tabs(Bind::default_value("a"), options, &h));
    let (t, f) = held.get().expect("made");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[0]), "Tab reaches the selected tab");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[2]), "ArrowRight skips the disabled tab");
    assert_eq!(t.value(), "c", "automatic: focus selects");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[0]) && t.value() == "a", "and wraps");
    plain(&mut s, NamedKey::End);
    assert!(focused(f[2]) && t.value() == "c");
    plain(&mut s, NamedKey::Home);
    assert!(focused(f[0]) && t.value() == "a");
    plain(&mut s, NamedKey::Tab);
    assert!(!focused(f[0]) && !focused(f[2]), "Tab leaves the list for the panel");
    press(&mut s, "Two");
    assert_eq!(t.value(), "a", "a disabled tab is not selected by a press");
    click(&mut s, "Three");
    assert_eq!(t.value(), "c", "a press selects");
}

#[test]
fn tabs_manual_selects_on_enter_and_space_only() {
    let options = Signal::new(TabsOptions {
        activation: Activation::Manual,
        ..TabsOptions::default()
    });
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || tabs(Bind::default_value("a"), options, &h));
    let (t, f) = held.get().expect("made");
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[2]));
    assert_eq!(t.value(), "a", "manual: focus alone selects nothing");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(t.value(), "c", "Enter selects");
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::Space);
    assert_eq!(t.value(), "a", "Space selects");
    // Vertical: the arrows turn with the orientation, read as it changes.
    s.act("vertical", || {
        options.set(TabsOptions {
            orientation: gui::Axis::Vertical,
            activation: Activation::Manual,
            ..TabsOptions::default()
        });
    });
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(f[2]), "vertical: ArrowDown");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f[2]), "vertical: across arrows do nothing");
}

#[test]
fn tabs_automatic_selects_on_focus_from_code_too() {
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || tabs(Bind::default_value("a"), options, &h));
    let (t, f) = held.get().expect("made");
    click(&mut s, "One");
    assert_eq!(gui::focus::modality(), gui::Modality::Pointer);
    s.act("focus from code", || gui::focus::set_focus(Some(f[2].id()), false));
    s.settle().expect("settled");
    assert_eq!(t.value(), "c", "focus from code selects, the pointer's modality or not");
    s.act("focus a disabled one", || gui::focus::set_focus(Some(f[1].id()), false));
    s.settle().expect("settled");
    assert_eq!(t.value(), "c", "a disabled tab is not selected");
}

/// The indicator in its own instance, counting its renders.
fn tabs_indicator(t: Tabs<Bind<&'static str>, &'static str>, renders: &Rc<Cell<usize>>) -> Element {
    let renders = Rc::clone(renders);
    component((), move |_| {
        let renders = Rc::clone(&renders);
        move || {
            // The cold check's render of every instance is not one of its own.
            if !gui::component::is_cold_render() {
                renders.set(renders.get() + 1);
            }
            t.indicator_part(div().named("bar").height(2.0))
                .map_or(Element::Empty, |d| d.into_element())
        }
    })
    .into_element()
}

#[test]
fn tabs_indicator_follows_the_selection_with_no_render() {
    let held: Held<Tabs<Bind<&'static str>, &'static str>> = Rc::default();
    let renders: Rc<Cell<usize>> = Rc::default();
    let (h, r) = (Rc::clone(&held), Rc::clone(&renders));
    let mut s = scenario(move || {
        let (h, r) = (Rc::clone(&h), Rc::clone(&r));
        component(Bind::default_value("a"), move |cx| {
            let t = Tabs::new(
                use_model(cx.props(), |b: &Bind<&'static str>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            h.set(Some(t));
            let focus = [FocusHandle::new(), FocusHandle::new()];
            let r = Rc::clone(&r);
            move || {
                t.list(div().display(Display::Flex).position(Position::Relative))
                    .child(t.trigger(label("tab-a", "One"), "a", Availability::Enabled, focus[0]))
                    .child(t.trigger(
                        div().named("tab-b").width(160.0).height(24.0).child(text("Two")),
                        "b",
                        Availability::Enabled,
                        focus[1],
                    ))
                    .child(tabs_indicator(t, &r))
                    .into_element()
            }
        })
        .into_element()
    });
    let a = bounds(&s, "tab-a").expect("laid out");
    let bar = bounds(&s, "bar").expect("shown once the tab is laid out");
    assert_eq!((bar.x, bar.width), (a.x, a.width));
    let before = renders.get();
    click(&mut s, "Two");
    let b = bounds(&s, "tab-b").expect("laid out");
    let bar = bounds(&s, "bar").expect("shown");
    assert_eq!((bar.x, bar.width), (b.x, 160.0), "at the selected tab, its length");
    assert_eq!(renders.get(), before, "the indicator moved with no render");
}

#[test]
fn tabs_a11y() {
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || tabs(Bind::default_value("a"), options, &h));
    insta::assert_snapshot!("tabs_a11y", a11y(&s));
}

// ----------------------------------------------------------------- Pagination

fn pagination(page: Bind<usize>, options: Signal<PaginationOptions>, held: &Held<Pagination<Bind<usize>>>) -> Element {
    let held = Rc::clone(held);
    component(page, move |cx| {
        let p = Pagination::new(use_model(cx.props(), |b: &Bind<usize>| b), move || options.get());
        held.set(Some(p));
        move || {
            let pages = p.entries().into_iter().map(|e| match e {
                Entry::Page(n) => p.page_button(label("page", &n.to_string()), n).into_element(),
                Entry::Ellipsis => label("ellipsis", "…").into_element(),
            });
            p.root(div().display(Display::Flex))
                .child(p.step_button(label("step", "First"), Step::First))
                .child(p.step_button(label("step", "Prev"), Step::Previous))
                .children(pages)
                .child(p.step_button(label("step", "Next"), Step::Next))
                .child(p.step_button(label("step", "Last"), Step::Last))
                .into_element()
        }
    })
    .into_element()
}

fn ten_pages() -> PaginationOptions {
    PaginationOptions {
        total: 100,
        items_per_page: 10,
        siblings: 1,
        show_edges: true,
        ..PaginationOptions::default()
    }
}

#[test]
fn pagination_steps_and_pages_within_the_count() {
    let options = Signal::new(ten_pages());
    let held: Held<Pagination<Bind<usize>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || pagination(Bind::default_value(1), options, &h));
    let p = held.get().expect("made");
    assert_eq!(p.page_count(), 10);
    assert_eq!(
        p.step_availability(Step::Previous),
        Availability::Disabled,
        "on the first page"
    );
    press(&mut s, "Prev");
    assert_eq!(p.page(), 1, "nothing before the first");
    click(&mut s, "Next");
    assert_eq!(p.page(), 2);
    click(&mut s, "Last");
    assert_eq!(p.page(), 10);
    assert_eq!(
        p.step_availability(Step::Next),
        Availability::Disabled,
        "on the last page"
    );
    click(&mut s, "First");
    click(&mut s, "3");
    assert_eq!(p.page(), 3, "a page button goes to its page");
    assert!(p.is_current(3));
    assert_eq!(
        p.entries(),
        [
            Entry::Page(1),
            Entry::Page(2),
            Entry::Page(3),
            Entry::Page(4),
            Entry::Page(5),
            Entry::Ellipsis,
            Entry::Page(10)
        ],
        "always 2 × siblings + 5 entries"
    );
    // Disabled: every button.
    s.act("disable", || {
        options.set(PaginationOptions {
            availability: Availability::Disabled,
            ..ten_pages()
        });
    });
    s.settle().expect("settled");
    press(&mut s, "Next");
    assert_eq!(p.page(), 3, "a disabled pagination moves nowhere");
}

#[test]
fn pagination_clamps_a_page_written_out_of_range_when_asked() {
    let heard: Rc<RefCell<Vec<usize>>> = Rc::default();
    let clamped = |clamp: bool, heard: &Rc<RefCell<Vec<usize>>>| {
        let options = Signal::new(PaginationOptions { clamp, ..ten_pages() });
        let held: Held<Pagination<Bind<usize>>> = Rc::default();
        let (h, h2) = (Rc::clone(&held), Rc::clone(heard));
        let s = scenario(move || {
            let h2 = Rc::clone(&h2);
            pagination(
                Bind::default_value(15).on_change(move |v| h2.borrow_mut().push(v)),
                options,
                &h,
            )
        });
        (s, options, held.get().expect("made"))
    };
    // Not asked: a page written past the count stays as written, and no
    // button is the current one.
    let (s, _, p) = clamped(false, &heard);
    assert_eq!(p.page(), 15);
    assert!((1..=10).all(|n| !p.is_current(n)));
    assert!(heard.borrow().is_empty(), "nothing to tell");
    assert!(!a11y(&s).contains("current="), "no page is current");
    // Asked: corrected from the first value on, before the first frame,
    // and told once.
    let (mut s, options, p) = clamped(true, &heard);
    assert_eq!(p.page(), 10, "page 15 of 10 is page 10 from the start");
    assert_eq!(*heard.borrow(), [10], "told once");
    assert!(
        a11y(&s).contains("\"Page 10\" current=Page"),
        "the first frame shows page 10:\n{}",
        a11y(&s)
    );
    s.act("fewer items", || {
        options.set(PaginationOptions {
            total: 30,
            clamp: true,
            ..ten_pages()
        });
    });
    s.settle().expect("settled");
    assert_eq!(p.page(), 3, "fewer pages: the last there is");
    assert_eq!(*heard.borrow(), [10, 3], "told once more");
    s.act("past the end", || p.go(9));
    s.settle().expect("settled");
    assert_eq!(p.page(), 3, "the user goes no further than the last");
    assert_eq!(*heard.borrow(), [10, 3], "an equal write tells nothing");
}

#[test]
fn pagination_a11y() {
    let options = Signal::new(ten_pages());
    let held: Held<Pagination<Bind<usize>>> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || pagination(Bind::default_value(1), options, &h));
    insta::assert_snapshot!("pagination_a11y", a11y(&s));
}

// --------------------------------------------------------------------- Avatar

#[test]
fn avatar_status_follows_the_load_and_the_fallback_waits_its_delay() {
    // 0: missing, 1: pending, 2: failed, 3: decoded (an `ImageLoad` has no equality).
    let load = Signal::new(1_u8);
    let picture = gui::ImageSource::from_rgba(1, 1, &[0, 0, 0, 255]).expect("one pixel");
    let told: Rc<RefCell<Vec<Status>>> = Rc::default();
    let t2 = Rc::clone(&told);
    let held: Held<(Avatar, Fallback, Fallback)> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        let t2 = Rc::clone(&t2);
        let picture = picture.clone();
        component((), move |_| {
            let t2 = Rc::clone(&t2);
            let picture = picture.clone();
            let avatar = Avatar::new();
            let _image = avatar.image(
                move || match load.get() {
                    0 => ImageLoad::Missing,
                    1 => ImageLoad::Pending,
                    2 => ImageLoad::Failed,
                    _ => ImageLoad::Ready(picture.clone()),
                },
                move |st| t2.borrow_mut().push(st),
            );
            let late = Fallback::new(Some(Duration::from_millis(600)), move || avatar.status());
            let now = Fallback::new(Some(Duration::ZERO), move || avatar.status());
            h.set(Some((avatar, late, now)));
            move || {
                div()
                    .when(late.shown(), |d| d.child(label("late", "Late")))
                    .when(now.shown(), |d| d.child(label("now", "Now")))
                    .into_element()
            }
        })
        .into_element()
    });
    let (avatar, ..) = held.get().expect("made");
    assert_eq!(avatar.status(), Status::Loading);
    assert!(has(&s, "now"), "a delay of 0 shows at once");
    assert!(!has(&s, "late"), "before its delay");
    s.at(0.59);
    s.settle().expect("settled");
    assert!(!has(&s, "late"), "still before 600 ms");
    s.at(0.61);
    s.settle().expect("settled");
    assert!(has(&s, "late"), "after 600 ms");
    s.act("failed", || load.set(2));
    s.settle().expect("settled");
    assert_eq!(avatar.status(), Status::Error);
    assert!(has(&s, "late"), "the fallback stays for an error");
    s.act("missing", || load.set(0));
    s.settle().expect("settled");
    assert_eq!(avatar.status(), Status::Error, "no source is an error");
    assert_eq!(
        *told.borrow(),
        [Status::Loading, Status::Error],
        "told each change from the first"
    );
    s.act("decoded", || load.set(3));
    s.settle().expect("settled");
    assert!(!has(&s, "late") && !has(&s, "now"), "a picture hides the fallbacks");
    // A new source on its way: the delay counts again from here (reka's
    // counts from the mount only, and would show it at once).
    s.act("a new source", || load.set(1));
    s.settle().expect("settled");
    assert!(has(&s, "now"));
    assert!(!has(&s, "late"), "the delay counts afresh");
    s.at(1.0);
    s.settle().expect("settled");
    assert!(!has(&s, "late"), "390 ms on: still waiting");
    s.at(1.3);
    s.settle().expect("settled");
    assert!(has(&s, "late"), "600 ms after the picture went");
}

// ---------------------------------------------------------------- AspectRatio

#[test]
fn aspect_ratio_follows_its_width_and_content_never_sizes_it() {
    let s = scenario(move || {
        div()
            .width(320.0)
            .child(frame(
                16.0 / 9.0,
                div()
                    .named("ratio")
                    .child(div().named("tall").width(10.0).height(900.0)),
            ))
            .into_element()
    });
    let b = bounds(&s, "ratio").expect("laid out");
    assert_eq!(b.width, 320.0, "as wide as its container");
    assert_eq!(b.height, 180.0, "its width over the ratio, whatever is inside");
    let tall = bounds(&s, "tall").expect("laid out");
    assert_eq!((tall.x, tall.y), (b.x, b.y), "the content fills it from its corner");
    let fill = bounds(&s, "aspect-ratio-content").expect("laid out");
    assert_eq!((fill.width, fill.height), (b.width, b.height));
}

// ----------------------------------------------------------------- ScrollArea

#[test]
fn scroll_area_scrolls_by_chromes_keys_and_chains_at_the_edge() {
    let inner_handle: Held<ScrollArea> = Rc::default();
    let outer_handle: Held<ScrollArea> = Rc::default();
    let (i2, o2) = (Rc::clone(&inner_handle), Rc::clone(&outer_handle));
    let mut s = scenario(move || {
        let (i2, o2) = (Rc::clone(&i2), Rc::clone(&o2));
        component((), move |_| {
            let outer = ScrollArea::new(ScrollAreaOptions::default, Direction::Ltr);
            let inner = ScrollArea::new(ScrollAreaOptions::default, Direction::Ltr);
            o2.set(Some(outer));
            i2.set(Some(inner));
            move || {
                outer
                    .viewport(div().named("outer").height(300.0).width(400.0))
                    .child(
                        inner
                            .viewport(div().named("inner").height(200.0).width(300.0))
                            .child(div().height(1000.0).width(10.0)),
                    )
                    .child(div().height(1000.0).width(10.0))
                    .into_element()
            }
        })
        .into_element()
    });
    let inner = inner_handle.get().expect("made");
    let outer = outer_handle.get().expect("made");
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Tab);
    let y = |a: &ScrollArea| a.handle().offset().1;
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(y(&inner), LINE_STEP, "ArrowDown: 40 px");
    plain(&mut s, NamedKey::PageDown);
    assert_eq!(y(&inner), LINE_STEP + 200.0 * 0.875, "PageDown: 87.5 % of the viewport");
    plain(&mut s, NamedKey::Space);
    assert_eq!(
        y(&inner),
        LINE_STEP + 2.0 * 175.0,
        "Space, the viewport focused: a page"
    );
    shift(&mut s, NamedKey::Space);
    assert_eq!(y(&inner), LINE_STEP + 175.0, "Shift+Space: a page up");
    plain(&mut s, NamedKey::End);
    assert_eq!(y(&inner), 800.0, "End: the end");
    assert_eq!(y(&outer), 0.0, "the outer one has not moved");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(y(&inner), 800.0);
    assert_eq!(
        y(&outer),
        LINE_STEP,
        "at the edge the key goes on to the scroller around it"
    );
    plain(&mut s, NamedKey::Home);
    assert_eq!(y(&inner), 0.0, "Home: the start");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(
        inner.handle().offset().0,
        0.0,
        "no horizontal bar: across does not scroll"
    );
}

#[test]
fn scroll_area_keys_scroll_the_view_physically_right_to_left() {
    let held: Held<ScrollArea> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let area = ScrollArea::new(
                || ScrollAreaOptions {
                    bars: Bars::Both,
                    ..ScrollAreaOptions::default()
                },
                Direction::Rtl,
            );
            h.set(Some(area));
            move || {
                area.viewport(
                    div()
                        .named("viewport")
                        .width(200.0)
                        .height(100.0)
                        .direction(Direction::Rtl),
                )
                .child(div().width(1000.0).height(10.0))
                .into_element()
            }
        })
        .into_element()
    });
    let area = held.get().expect("made");
    plain(&mut s, NamedKey::Tab);
    let x = || area.handle().offset().0;
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(x(), LINE_STEP, "right to left, ArrowLeft goes on from the right edge");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(x(), 2.0 * LINE_STEP);
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(x(), LINE_STEP, "ArrowRight back towards the right edge");
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(x(), 0.0, "at the edge: stays");
}

#[test]
fn scroll_area_a_scroll_moves_the_thumb_with_no_render() {
    let held: Held<ScrollArea> = Rc::default();
    let renders: Rc<Cell<usize>> = Rc::default();
    let (h, r) = (Rc::clone(&held), Rc::clone(&renders));
    let mut s = scenario(move || {
        let (h, r) = (Rc::clone(&h), Rc::clone(&r));
        component((), move |_| {
            let area = ScrollArea::new(
                || ScrollAreaOptions {
                    policy: Policy::Always,
                    ..ScrollAreaOptions::default()
                },
                Direction::Ltr,
            );
            h.set(Some(area));
            let r = Rc::clone(&r);
            move || {
                let r = Rc::clone(&r);
                let bar = component((), move |_| {
                    let r = Rc::clone(&r);
                    move || {
                        // The cold check's render of every instance is not one of its own.
                        if !gui::component::is_cold_render() {
                            r.set(r.get() + 1);
                        }
                        let thumb = area.thumb(div().named("thumb").width(8.0), Axis::Vertical);
                        area.scrollbar(
                            div()
                                .named("bar")
                                .position(Position::Absolute)
                                .top(0.0)
                                .bottom(0.0)
                                .right(0.0)
                                .width(10.0)
                                .when_some(thumb, Div::child),
                            Axis::Vertical,
                        )
                    }
                })
                .into_element();
                area.root(div().position(Position::Relative).width(200.0).height(100.0))
                    .child(
                        area.viewport(div().width(200.0).height(100.0))
                            .child(div().height(1000.0).width(10.0)),
                    )
                    .child(bar)
                    .into_element()
            }
        })
        .into_element()
    });
    let area = held.get().expect("made");
    s.wait(Duration::from_millis(50));
    s.settle().expect("settled");
    let top = bounds(&s, "thumb").expect("a thumb").y;
    let before = renders.get();
    s.act("scroll", || area.handle().scroll_to((0.0, 450.0)));
    s.settle().expect("settled");
    let moved = bounds(&s, "thumb").expect("a thumb").y;
    assert!(moved > top + 30.0, "the thumb moved: {top} -> {moved}");
    assert_eq!(renders.get(), before, "a scroll renders nothing");
}

/// A finger `id` on the window at `(x, y)`.
fn finger(s: &mut Scenario, id: u64, phase: gui::TouchPhase, x: f32, y: f32) {
    s.send(gui::window::Input::Touch(gui::TouchEvent {
        id,
        kind: gui::PointerKind::Touch,
        phase,
        x,
        y,
    }));
    s.settle().expect("settled");
}

#[test]
fn scroll_area_a_bar_follows_the_finger_that_took_it_and_no_other() {
    use gui::TouchPhase::{Ended, Moved, Started};
    let held: Held<ScrollArea> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let area = ScrollArea::new(
                || ScrollAreaOptions {
                    policy: Policy::Always,
                    ..ScrollAreaOptions::default()
                },
                Direction::Ltr,
            );
            h.set(Some(area));
            move || {
                let thumb = area.thumb(div().named("thumb").width(8.0), Axis::Vertical);
                let bar = area.scrollbar(
                    div()
                        .named("bar")
                        .position(Position::Absolute)
                        .top(0.0)
                        .bottom(0.0)
                        .right(0.0)
                        .width(10.0)
                        .when_some(thumb, Div::child),
                    Axis::Vertical,
                );
                area.root(div().position(Position::Relative).width(200.0).height(100.0))
                    .child(
                        area.viewport(div().width(200.0).height(100.0))
                            .child(div().height(1000.0).width(10.0)),
                    )
                    .child(bar)
                    .into_element()
            }
        })
        .into_element()
    });
    let area = held.get().expect("made");
    s.wait(Duration::from_millis(50));
    s.settle().expect("settled");
    let y = || area.handle().offset().1;
    let bar = bounds(&s, "bar").expect("a bar");
    let thumb = bounds(&s, "thumb").expect("a thumb");
    let x = bar.x + bar.width / 2.0;
    let grab = thumb.y + thumb.height / 2.0;

    // The first finger takes the thumb and drags it down.
    finger(&mut s, 1, Started, x, grab);
    finger(&mut s, 1, Moved, x, grab + 20.0);
    let dragged = y();
    assert!(dragged > 0.0, "the first finger scrolls");
    // A second finger presses the track below and pans: the bar keeps
    // following the first, so nothing jumps.
    finger(&mut s, 2, Started, x, bar.y + 80.0);
    finger(&mut s, 2, Moved, x, bar.y + 95.0);
    assert_eq!(y(), dragged, "a second finger on the bar moves nothing");
    // It lifts: the first finger's drag goes on.
    finger(&mut s, 2, Ended, x, bar.y + 95.0);
    finger(&mut s, 1, Moved, x, grab + 40.0);
    assert!(y() > dragged, "the first finger still drags: {dragged} -> {}", y());
    finger(&mut s, 1, Ended, x, grab + 40.0);
}

#[test]
fn scroll_area_virtualizer_builds_the_rows_in_view_and_keeps_the_focused_ones() {
    const ROWS: u64 = 1_000_000;
    let held: Rc<RefCell<Option<(ScrollArea, ListState)>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let area = ScrollArea::new(
                || ScrollAreaOptions {
                    policy: Policy::Always,
                    ..ScrollAreaOptions::default()
                },
                Direction::Ltr,
            );
            let list = area.virtualizer(VirtualizerOptions::default());
            *h.borrow_mut() = Some((area, list));
            move || {
                let thumb = area.thumb(div().named("thumb").width(8.0), Axis::Vertical);
                let rows = area
                    .virtual_viewport(
                        ROWS,
                        |i| i,
                        |i| {
                            div()
                                .focusable()
                                .height(28.0)
                                .child(text(format!("row {i}")))
                                .into_element()
                        },
                        |v| v.named("viewport"),
                    )
                    .height(140.0);
                area.root(div().position(Position::Relative).width(200.0).height(140.0))
                    .child(rows)
                    .child(area.scrollbar(
                        div().named("bar").width(10.0).when_some(thumb, Div::child),
                        Axis::Vertical,
                    ))
                    .into_element()
            }
        })
        .into_element()
    });
    let (area, list) = held.borrow().clone().expect("made");
    s.settle().expect("settled");
    let built = |s: &Scenario, i: u64| s.locate(&format!("row {i}")).is_ok();
    // 140 px shows five rows; twelve rows' worth (28 px each) beyond it.
    assert!(built(&s, 0) && built(&s, 4), "the rows in view");
    assert!(built(&s, 16), "the overscan below");
    assert!(!built(&s, 40), "and no further: {:?}", list.window());
    assert!(
        bounds(&s, "thumb").is_some_and(|b| (b.height - 18.0).abs() < 0.5),
        "a million rows: the shortest thumb"
    );

    // Chrome's keys on the viewport, exact over the whole extent.
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(
        (list.view_top() - f64::from(LINE_STEP)).abs() < 0.5,
        "{}",
        list.view_top()
    );
    plain(&mut s, NamedKey::End);
    s.settle().expect("settled");
    assert!(built(&s, ROWS - 1), "End: the last row");
    let thumb_end = bounds(&s, "thumb").expect("a thumb");
    let bar = bounds(&s, "bar").expect("a bar");
    assert!(
        (thumb_end.bottom() - bar.bottom()).abs() < 1.0,
        "the thumb at the track's end"
    );
    plain(&mut s, NamedKey::Home);
    s.settle().expect("settled");
    assert!(built(&s, 0) && list.view_top() < 0.5, "Home: the first row");

    // The focused row and its neighbours stay built scrolled away; Tab goes
    // on to the next row and brings it into view.
    let (x, y) = s.locate("row 3").expect("in view");
    s.click(x, y);
    s.settle().expect("settled");
    assert_eq!(list.focused_row(), Some(3));
    s.act("far", || list.scroll_to_row(500_000));
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert_eq!(list.pinned(), [2, 3, 4], "the focused row and its neighbours");
    plain(&mut s, NamedKey::Tab);
    s.settle().expect("settled");
    assert_eq!(list.focused_row(), Some(4), "Tab: the next row, not the rows in view");
    let row4 = s.locate("row 4").expect("built");
    assert!(row4.1 >= 0.0 && row4.1 <= 140.0, "brought into view: {row4:?}");
    shift(&mut s, NamedKey::Tab);
    s.settle().expect("settled");
    assert_eq!(list.focused_row(), Some(3), "Shift+Tab: the previous row");

    // The caller goes back to the top as reka's `scrollTop`.
    s.act("far", || list.scroll_to_row(700_000));
    s.settle().expect("settled");
    s.act("top", || area.scroll_top());
    s.settle().expect("settled");
    assert!(list.view_top() < 0.5);
}

#[test]
fn scroll_area_virtualizer_a11y() {
    let mut s = scenario(|| {
        component((), |_| {
            let area = ScrollArea::new(ScrollAreaOptions::default, Direction::Ltr);
            let _list = area.virtualizer(VirtualizerOptions::default());
            move || {
                area.root(div().position(Position::Relative).width(200.0).height(84.0))
                    .child(
                        area.virtual_viewport(
                            1000,
                            |i| i,
                            |i| div().height(28.0).child(text(format!("row {i}"))).into_element(),
                            |v| v,
                        )
                        .height(84.0)
                        .max_height(84.0),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let tree = a11y(&s);
    let mut ts = gui::text::TextSystem::new();
    let update = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let list = update
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::List)
        .expect("a list");
    assert_eq!(list.1.row_count(), Some(1000), "{tree}");
    let row = update
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::ListItem && n.position_in_set() == Some(2))
        .expect("row 1");
    assert_eq!(row.1.size_of_set(), Some(1000), "each row its place in all of them");
}

// ------------------------------------------------------------------- Splitter

type SplitHeld = Held<(Splitter<Bind<Vec<f64>>>, [FocusHandle; 2])>;

fn splitter(layout: Bind<Vec<f64>>, direction: Direction, held: &SplitHeld) -> Element {
    splitter_with(layout, direction, held, None)
}

/// [`splitter`] with the handle after panel `disabled` disabled.
fn splitter_with(layout: Bind<Vec<f64>>, direction: Direction, held: &SplitHeld, disabled: Option<usize>) -> Element {
    let held = Rc::clone(held);
    component(layout, move |cx| {
        let sp = Splitter::new(
            use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
            SplitterOptions::default,
            direction,
        );
        sp.register(
            0,
            Constraints {
                min: 20.0,
                collapsible: true,
                collapsed: 5.0,
                ..Constraints::default()
            },
        );
        sp.register(1, Constraints::default());
        sp.register(
            2,
            Constraints {
                min: 10.0,
                max: Some(50.0),
                ..Constraints::default()
            },
        );
        let focus = [FocusHandle::new(), FocusHandle::new()];
        held.set(Some((sp, focus)));
        move || {
            let handle = |i: usize| {
                let availability = if disabled == Some(i) {
                    Availability::Disabled
                } else {
                    Availability::Enabled
                };
                sp.handle(div().named("handle").width(2.0).focusable(), i, availability, focus[i])
            };
            sp.group(div().named("group").display(Display::Flex).width(604.0).height(100.0))
                .child(sp.panel(div().named("p0").flex(Flex::grow(1.0)), 0))
                .child(handle(0))
                .child(sp.panel(div().named("p1").flex(Flex::grow(1.0)), 1))
                .child(handle(1))
                .child(sp.panel(div().named("p2").flex(Flex::grow(1.0)), 2))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn splitter_keys_move_collapse_and_cycle() {
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || splitter(Bind::default_value(vec![40.0, 30.0, 30.0]), Direction::Ltr, &h));
    let (sp, f) = held.get().expect("made");
    assert_eq!(
        bounds(&s, "p0").expect("laid out").width,
        240.0,
        "40 % of the room the handles leave"
    );
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(sp.layout(), [50.0, 20.0, 30.0], "ArrowRight: 10 %");
    shift(&mut s, NamedKey::ArrowLeft);
    assert_eq!(
        sp.layout(),
        [5.0, 65.0, 30.0],
        "Shift: 100 %; below half its min it collapses"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(
        sp.layout(),
        [50.0, 20.0, 30.0],
        "Enter expands, to the size before collapsing"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(sp.layout(), [5.0, 65.0, 30.0], "Enter collapses");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(sp.layout(), [20.0, 50.0, 30.0], "one key from collapsed: its min");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(sp.layout(), [5.0, 65.0, 30.0], "one key from its min: collapsed");
    plain(&mut s, NamedKey::End);
    assert_eq!(sp.layout(), [90.0, 0.0, 10.0], "End: as far as the others' mins allow");
    plain(&mut s, NamedKey::Home);
    assert_eq!(sp.layout()[0], 5.0, "Home: collapsed");
    plain(&mut s, NamedKey::F6);
    assert!(focused(f[1]), "F6: the next handle");
    plain(&mut s, NamedKey::F6);
    assert!(focused(f[0]), "and wraps");
    shift(&mut s, NamedKey::F6);
    assert!(focused(f[1]), "Shift+F6: the previous");
    let before = sp.layout()[2];
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(sp.layout()[2], before + 10.0, "the last panel grows");
    shift(&mut s, NamedKey::ArrowLeft);
    assert_eq!(sp.layout()[2], 50.0, "held at its max");
}

#[test]
fn splitter_right_to_left_flips_the_arrows_not_home_and_end() {
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || splitter(Bind::default_value(vec![40.0, 30.0, 30.0]), Direction::Rtl, &h));
    let (sp, f) = held.get().expect("made");
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(
        sp.layout(),
        [50.0, 20.0, 30.0],
        "right to left: ArrowLeft grows the first panel"
    );
    plain(&mut s, NamedKey::Home);
    assert_eq!(sp.layout()[0], 5.0, "Home still shrinks it");
}

#[test]
fn splitter_drag_moves_by_the_share_of_the_room() {
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || splitter(Bind::default_value(vec![40.0, 30.0, 30.0]), Direction::Ltr, &h));
    let (sp, _) = held.get().expect("made");
    let b = bounds(&s, "handle").expect("laid out");
    let (x, y) = (b.x + 1.0, b.y + 50.0);
    s.drag((x, y), (x + 60.0, y));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), [50.0, 20.0, 30.0], "60 px of 600: 10 %");
    assert_eq!(sp.dragging(), None, "the drag ended");
}

#[test]
fn splitter_disabled_handle_keeps_focus_moves_nothing_and_f6_passes_it() {
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s =
        scenario(move || splitter_with(Bind::default_value(vec![40.0, 30.0, 30.0]), Direction::Ltr, &h, Some(1)));
    let (sp, f) = held.get().expect("made");
    let start = sp.layout();
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[1]), "a disabled handle stays in the Tab order");
    for k in [
        NamedKey::ArrowRight,
        NamedKey::ArrowLeft,
        NamedKey::Home,
        NamedKey::End,
        NamedKey::Enter,
    ] {
        plain(&mut s, k);
        assert_eq!(sp.layout(), start, "{k:?} on a disabled handle moves nothing");
    }
    shift(&mut s, NamedKey::ArrowLeft);
    assert_eq!(sp.layout(), start, "nor Shift with an arrow");
    let b = s
        .last()
        .nodes
        .iter()
        .filter(|n| n.name == Some("handle"))
        .nth(1)
        .map(|n| n.bounds)
        .expect("the second handle");
    let (x, y) = (b.x + 1.0, b.y + 50.0);
    s.drag((x, y), (x + 60.0, y));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), start, "nor a drag");
    plain(&mut s, NamedKey::F6);
    assert!(focused(f[0]), "F6 leaves it");
    plain(&mut s, NamedKey::F6);
    assert!(focused(f[0]), "and passes it: the only usable handle wraps to itself");
    shift(&mut s, NamedKey::F6);
    assert!(focused(f[0]), "Shift+F6 passes it too");
    let tree = a11y(&s);
    assert!(
        tree.lines()
            .any(|l| l.contains("Splitter \"Resize panel 2\"") && l.contains("disabled")),
        "announced disabled: {tree}"
    );
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(sp.layout(), [50.0, 20.0, 30.0], "the usable handle still moves");
}

/// A horizontal group of two panels, each its own instance counting its
/// renders, the first at least 40 %: the first holds a box that hears the
/// pointer's hover, the second a vertical group of two (its own instance).
type Nested = Held<(Splitter<Bind<Vec<f64>>>, Splitter<Bind<Vec<f64>>>)>;

fn counted_panel(
    sp: Splitter<Bind<Vec<f64>>>,
    index: usize,
    renders: &Rc<Cell<usize>>,
    child: Option<Element>,
) -> Element {
    let renders = Rc::clone(renders);
    component(index, move |cx| {
        let index = cx.props().get_untracked();
        let renders = Rc::clone(&renders);
        let child = child.clone();
        move || {
            // The cold check's render of every instance is not one of its own.
            if !gui::component::is_cold_render() {
                renders.set(renders.get() + 1);
            }
            let d = div().named(if index == 0 { "p0" } else { "p1" }).flex(Flex::grow(1.0));
            let d = match child.clone() {
                Some(c) => d.child(c),
                None => d,
            };
            sp.panel(d, index).into_element()
        }
    })
    .into_element()
}

fn nested_splitters(held: &Nested, renders: &Rc<Cell<usize>>, hovered: &Rc<Cell<bool>>) -> Element {
    nested_splitters_while(held, renders, hovered, None)
}

/// [`nested_splitters`], the inner splitter mounted while `inner_shown`
/// holds (always without one).
fn nested_splitters_while(
    held: &Nested,
    renders: &Rc<Cell<usize>>,
    hovered: &Rc<Cell<bool>>,
    inner_shown: Option<Signal<bool>>,
) -> Element {
    let (held, renders, hovered) = (Rc::clone(held), Rc::clone(renders), Rc::clone(hovered));
    component(Bind::default_value(vec![50.0, 50.0]), move |cx| {
        let outer = Splitter::new(
            use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
            SplitterOptions::default,
            Direction::Ltr,
        );
        outer.register(
            0,
            Constraints {
                min: 40.0,
                ..Constraints::default()
            },
        );
        outer.register(1, Constraints::default());
        let focus = FocusHandle::new();
        let (held, renders, hovered) = (Rc::clone(&held), Rc::clone(&renders), Rc::clone(&hovered));
        move || {
            let held = Rc::clone(&held);
            let h2 = Rc::clone(&hovered);
            let hears = div()
                .named("hears")
                .size(pct(100.0))
                .on_hover(move |inside| h2.set(inside))
                .into_element();
            let inner = component(Bind::default_value(vec![50.0, 50.0]), move |cx| {
                let inner = Splitter::new(
                    use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
                    || SplitterOptions {
                        orientation: Axis::Vertical,
                        ..SplitterOptions::default()
                    },
                    Direction::Ltr,
                );
                inner.register(0, Constraints::default());
                inner.register(1, Constraints::default());
                held.set(Some((outer, inner)));
                let focus = FocusHandle::new();
                move || {
                    inner
                        .group(
                            div()
                                .named("inner")
                                .display(Display::Flex)
                                .flex_direction(FlexDirection::Column)
                                .width(pct(100.0))
                                .height(pct(100.0)),
                        )
                        .child(inner.panel(div().named("q0"), 0))
                        .child(inner.handle(div().named("inner-handle").height(2.0), 0, Availability::Enabled, focus))
                        .child(inner.panel(div().named("q1"), 1))
                        .into_element()
                }
            })
            .into_element();
            // Behind a component of its own that renders as the signal
            // changes: the panel holding it renders once.
            let inner = match inner_shown {
                Some(shown) => component((), move |_| {
                    let inner = inner.clone();
                    move || {
                        if shown.get() {
                            inner.clone()
                        } else {
                            div().into_element()
                        }
                    }
                })
                .into_element(),
                None => inner,
            };
            outer
                .group(div().named("outer").display(Display::Flex).width(604.0).height(202.0))
                .child(counted_panel(outer, 0, &renders, Some(hears)))
                .child(outer.handle(div().named("outer-handle").width(2.0), 0, Availability::Enabled, focus))
                .child(counted_panel(outer, 1, &renders, Some(inner)))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn splitter_handles_crossing_drag_together_in_two_dimensions() {
    use gui::window::Input;
    let held: Nested = Rc::default();
    let renders: Rc<Cell<usize>> = Rc::default();
    let hovered: Rc<Cell<bool>> = Rc::default();
    let (h, r, v) = (Rc::clone(&held), Rc::clone(&renders), Rc::clone(&hovered));
    let mut s = scenario(move || nested_splitters(&h, &r, &v));
    let (outer, inner) = held.get().expect("made");
    let o = bounds(&s, "outer-handle").expect("laid out");
    let i = bounds(&s, "inner-handle").expect("laid out");
    assert_eq!(i.x, o.x + o.width, "the inner group starts where the outer handle ends");
    // Inside the inner group, on its handle, 3 px from the outer one.
    let (x, y) = (i.x + 3.0, i.y + 1.0);
    pointer(&mut s, Input::PointerMoved { x, y });
    assert_eq!(outer.handle_state(0), primitives::splitter::HandleState::Hover);
    assert_eq!(inner.handle_state(0), primitives::splitter::HandleState::Hover);
    assert_eq!(outer.cursor(0), gui::Cursor::Move, "where they cross: the move cursor");
    // On the outer handle itself, 1 px from the inner one's end.
    pointer(&mut s, Input::PointerMoved { x: o.x + 1.0, y });
    assert_eq!(
        inner.handle_state(0),
        primitives::splitter::HandleState::Hover,
        "reached from outside"
    );
    pointer(&mut s, Input::PointerMoved { x: i.x + 100.0, y });
    assert_eq!(
        outer.handle_state(0),
        primitives::splitter::HandleState::Inactive,
        "out of the outer's reach"
    );
    assert_eq!(inner.cursor(0), gui::Cursor::ResizeNorthSouth);
    // A press where they cross drags both, each along its axis.
    let before = renders.get();
    pointer(&mut s, Input::PointerMoved { x, y });
    pointer(&mut s, Input::MouseDown { x, y });
    let start_renders = renders.get();
    for d in [6.0, 12.0, 18.0, 24.0, 30.0] {
        pointer(&mut s, Input::PointerMoved { x: x + d, y: y + d });
    }
    assert_eq!(outer.dragging(), Some(0));
    assert_eq!(inner.dragging(), Some(0));
    let room_x = 604.0 - 2.0;
    let room_y = 202.0 - 2.0;
    let (ol, il) = (outer.layout(), inner.layout());
    assert!((ol[0] - (50.0 + 30.0 / room_x * 100.0)).abs() < 1e-6, "{ol:?}");
    assert!((il[0] - (50.0 + 30.0 / room_y * 100.0)).abs() < 1e-6, "{il:?}");
    assert_eq!(
        renders.get(),
        start_renders,
        "the moves lay the panels out and render none (the press rendered them: {before} -> {start_renders})"
    );
    pointer(
        &mut s,
        Input::MouseUp {
            x: x + 30.0,
            y: y + 30.0,
        },
    );
    assert_eq!((outer.dragging(), inner.dragging()), (None, None));
    assert!(
        renders.get() <= start_renders + 2,
        "the drag's end renders each panel once"
    );
}

#[test]
fn a_splitter_unmounted_while_it_drags_across_lets_its_partner_go() {
    use gui::window::Input;
    let held: Nested = Rc::default();
    let renders: Rc<Cell<usize>> = Rc::default();
    let hovered: Rc<Cell<bool>> = Rc::default();
    let inner_shown = Signal::new(true);
    let (h, r, v) = (Rc::clone(&held), Rc::clone(&renders), Rc::clone(&hovered));
    let mut s = scenario(move || nested_splitters_while(&h, &r, &v, Some(inner_shown)));
    let (outer, inner) = held.get().expect("made");
    let i = bounds(&s, "inner-handle").expect("laid out");
    // Pressed where the handles cross, inside the inner group: the inner
    // splitter drags, the outer one with it.
    let (x, y) = (i.x + 3.0, i.y + 1.0);
    pointer(&mut s, Input::PointerMoved { x, y });
    pointer(&mut s, Input::MouseDown { x, y });
    pointer(&mut s, Input::PointerMoved { x: x + 6.0, y: y + 6.0 });
    assert_eq!((outer.dragging(), inner.dragging()), (Some(0), Some(0)));
    // The inner splitter goes mid-drag: its drag is cancelled while it is
    // alive, and lets its partner go; the outer handle drags no more.
    s.act("unmount", || inner_shown.set(false));
    s.settle().expect("settled");
    assert_eq!(outer.dragging(), None, "the partner was let go");
    let was = outer.layout();
    pointer(
        &mut s,
        Input::PointerMoved {
            x: x + 40.0,
            y: y + 6.0,
        },
    );
    assert_eq!(outer.layout(), was, "the outer splitter does not follow the pointer");
    pointer(
        &mut s,
        Input::MouseUp {
            x: x + 40.0,
            y: y + 6.0,
        },
    );
}

#[test]
fn splitter_panels_take_no_pointer_while_a_drag_runs() {
    use gui::window::Input;
    let held: Nested = Rc::default();
    let renders: Rc<Cell<usize>> = Rc::default();
    let hovered: Rc<Cell<bool>> = Rc::default();
    let (h, r, v) = (Rc::clone(&held), Rc::clone(&renders), Rc::clone(&hovered));
    let mut s = scenario(move || nested_splitters(&h, &r, &v));
    let o = bounds(&s, "outer-handle").expect("laid out");
    let (x, y) = (o.x + 1.0, o.y + 20.0);
    pointer(&mut s, Input::PointerMoved { x: 100.0, y });
    assert!(hovered.get(), "no drag: the pointer hovers what a panel holds");
    pointer(&mut s, Input::PointerMoved { x, y });
    assert!(!hovered.get());
    pointer(&mut s, Input::MouseDown { x, y });
    pointer(&mut s, Input::PointerMoved { x: x - 10.0, y });
    // Past the first panel's min the handle stops; the pointer goes on over it.
    pointer(&mut s, Input::PointerMoved { x: 100.0, y });
    let (outer, _) = held.get().expect("made");
    assert_eq!(outer.layout()[0], 40.0, "held at its min");
    assert!(!hovered.get(), "a drag runs: what the panels hold is not hovered");
    pointer(&mut s, Input::MouseUp { x: 100.0, y });
    pointer(&mut s, Input::PointerMoved { x: 101.0, y });
    assert!(hovered.get(), "after it, hovered again");
}

#[test]
fn splitter_controlled_by_a_value_only_asks() {
    let heard: Rc<RefCell<Vec<Vec<f64>>>> = Rc::default();
    let h2 = Rc::clone(&heard);
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h2 = Rc::clone(&h2);
        splitter(
            Bind::value(vec![40.0, 30.0, 30.0]).on_change(move |v| h2.borrow_mut().push(v)),
            Direction::Ltr,
            &h,
        )
    });
    let (sp, f) = held.get().expect("made");
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(*heard.borrow(), [vec![50.0, 20.0, 30.0]]);
    assert_eq!(sp.layout(), [40.0, 30.0, 30.0], "the caller's value is shown");
}

#[test]
fn splitter_a11y() {
    let held: SplitHeld = Rc::default();
    let h = Rc::clone(&held);
    // The second handle disabled: read as disabled, still named and valued.
    let s = scenario(move || splitter_with(Bind::default_value(vec![40.0, 30.0, 30.0]), Direction::Ltr, &h, Some(1)));
    insta::assert_snapshot!("splitter_a11y", a11y(&s));
}

// ------------------------------------------------------------------- Carousel

type CarouselHeld = Held<Carousel<Bind<usize>>>;

/// What a carousel test sets: its options, whether it has a group of
/// indicators, and who hears a press on a slide's control.
#[derive(Clone, Default)]
struct CarouselSetup {
    options: CarouselOptions,
    rotation: Option<Rotation>,
    indicators: bool,
    /// Indicators with no group: buttons, each going to its slide.
    lone_indicators: bool,
    opened: Rc<Cell<usize>>,
    settled: Rc<RefCell<Vec<usize>>>,
    /// The carousel's renders.
    renders: Rc<Cell<usize>>,
    /// Each slide's control is a Tab stop, focused through these.
    focusable: bool,
    focus: Rc<RefCell<Vec<FocusHandle>>>,
    /// Each slide's height (24 unless set).
    heights: Option<[f32; 4]>,
    /// The slides' keys, in order.
    keys: Rc<RefCell<Vec<SlideKey>>>,
    /// The track eases to its snap over the move's duration, as the look's does.
    animated: bool,
}

/// A group of indicators: its own instance, so it joins and leaves.
fn indicator_group(c: Carousel<Bind<usize>>) -> Element {
    component((), move |_| {
        c.join_indicators();
        let focus: Vec<FocusHandle> = (0..4).map(|_| FocusHandle::new()).collect();
        move || {
            c.indicators(div().named("dots").display(Display::Flex))
                .children((0..c.snaps()).map(|k| {
                    c.indicator(label("dot", &format!("Dot {}", k + 1)), k, focus[k])
                        .into_element()
                }))
                .into_element()
        }
    })
    .into_element()
}

fn carousel(index: Bind<usize>, setup: &CarouselSetup, held: &CarouselHeld) -> Element {
    let held = Rc::clone(held);
    let setup = setup.clone();
    component(index, move |cx| {
        let options = setup.options;
        let c = Carousel::new(
            use_model(cx.props(), |b: &Bind<usize>| b),
            move || options,
            setup.rotation,
            Direction::Ltr,
        );
        let heard = Rc::clone(&setup.settled);
        c.on_settle(move |k| heard.borrow_mut().push(k));
        let keys: Vec<_> = (0..4).map(|_| c.slide_key()).collect();
        *setup.keys.borrow_mut() = keys.clone();
        held.set(Some(c));
        let (opened, indicators) = (Rc::clone(&setup.opened), setup.indicators);
        let lone = setup.lone_indicators;
        let dot_focus: Vec<FocusHandle> = (0..4).map(|_| FocusHandle::new()).collect();
        let handles: Vec<FocusHandle> = (0..4).map(|_| FocusHandle::new()).collect();
        *setup.focus.borrow_mut() = handles.clone();
        let (renders, focusable, heights) = (Rc::clone(&setup.renders), setup.focusable, setup.heights);
        move || {
            // The cold check's render of every instance is not one of its own.
            if !gui::component::is_cold_render() {
                renders.set(renders.get() + 1);
            }
            let slides = keys.iter().enumerate().map(|(n, &k)| {
                let opened = Rc::clone(&opened);
                let open = label("open", &format!("Open {}", n + 1)).on_click(move |_| opened.set(opened.get() + 1));
                let open = if focusable {
                    open.track_focus(handles[n]).focusable()
                } else {
                    open
                };
                c.slide(
                    div()
                        .named("slide")
                        .width(300.0)
                        .height(heights.map_or(24.0, |h| h[n]))
                        .flex_shrink(0.0)
                        .child(open),
                    k,
                )
                .into_element()
            });
            let track = div()
                .named("track")
                .display(Display::Flex)
                .translate_x(move || c.offset())
                .children(slides);
            let track = if setup.animated {
                track
                    .transition(gui::motion::Properties::TRANSFORM)
                    .duration_ms(move || c.move_duration().as_secs_f32() * 1000.0)
            } else {
                track
            };
            let mut root = c
                .root(div().named("root").width(300.0))
                .child(
                    c.content(div().named("content").overflow(Overflow::Hidden))
                        .child(track),
                )
                .child(c.step_button(label("prev", "Prev"), false))
                .child(c.step_button(label("next", "Next"), true));
            if let Some(b) = c.rotation_button(label("rotate", "Rotate")) {
                root = root.child(b);
            }
            if indicators {
                root = root.child(indicator_group(c));
            }
            if lone {
                root = root.children((0..c.snaps()).map(|k| {
                    c.indicator(label("dot", &format!("Dot {}", k + 1)), k, dot_focus[k])
                        .into_element()
                }));
            }
            root.into_element()
        }
    })
    .into_element()
}

fn plain_carousel(held: &CarouselHeld) -> Scenario {
    let h = Rc::clone(held);
    scenario(move || carousel(Bind::default_value(0), &CarouselSetup::default(), &h))
}

fn pointer(s: &mut Scenario, input: gui::window::Input) {
    s.send(input);
    s.settle().expect("settled");
}

/// Press at `from`, move through `path` (x, at seconds), release at the
/// last place: a drag along the row.
fn drag_along(s: &mut Scenario, from: (f32, f32), path: &[(f32, f32)]) {
    use gui::window::Input;
    let (x, y) = from;
    pointer(s, Input::PointerMoved { x, y });
    pointer(s, Input::MouseDown { x, y });
    let mut at = x;
    for &(to, t) in path {
        s.at(t);
        pointer(s, Input::PointerMoved { x: to, y });
        at = to;
    }
    pointer(s, Input::MouseUp { x: at, y });
}

#[test]
fn carousel_steps_by_buttons_and_keys_within_its_snaps() {
    let held: CarouselHeld = Rc::default();
    let mut s = plain_carousel(&held);
    let c = held.get().expect("made");
    assert_eq!(c.snaps(), 4, "counted once laid out");
    assert_eq!(
        c.step_availability(false),
        Availability::Unavailable,
        "nothing before the first"
    );
    click(&mut s, "Next");
    assert_eq!(c.index(), 1);
    assert_eq!(c.position(), 1.0);
    // The root is a Tab stop and takes the keys while it holds focus.
    plain(&mut s, NamedKey::Tab);
    assert!(gui::focus::focused().is_some(), "the root is a Tab stop");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(c.index(), 2, "ArrowRight: the next snap");
    key(
        &mut s,
        NamedKey::ArrowRight,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    assert_eq!(c.index(), 2, "with Control: not handled");
    plain(&mut s, NamedKey::End);
    assert_eq!(c.index(), 3, "End: the last");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(c.index(), 3, "no wrap: stays");
    assert_eq!(c.step_availability(true), Availability::Unavailable);
    plain(&mut s, NamedKey::Home);
    assert_eq!(c.index(), 0, "Home: the first");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(c.index(), 0);
}

/// Whether the text `label` is painted (laid out and not hidden).
fn painted(s: &Scenario, label: &str) -> bool {
    s.last().nodes.iter().any(|n| {
        !n.look.invisible && matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == label)
    })
}

/// A slide sleeps only once the track has left it: on Next, the one going
/// out still paints (and is read) while the track moves, both on screen,
/// and sleeps once the track gets to the next snap.
#[test]
fn carousel_slides_sleep_once_the_track_has_left_them() {
    let held: CarouselHeld = Rc::default();
    let setup = CarouselSetup::default();
    let (h, st) = (Rc::clone(&held), setup.clone());
    let mut s = scenario(move || carousel(Bind::default_value(0), &st, &h));
    let c = held.get().expect("made");
    let keys = setup.keys.borrow().clone();
    assert!(
        painted(&s, "Open 1") && !painted(&s, "Open 2"),
        "at rest, the first alone"
    );
    click(&mut s, "Next");
    assert_eq!(c.index(), 1);
    assert!(
        painted(&s, "Open 1"),
        "the slide going out paints while the track moves"
    );
    assert!(painted(&s, "Open 2"), "and the one coming in");
    assert!(gui::untracked(|| c.in_view(keys[0])), "and both are read");
    frames(&mut s, 0.0, 1.0);
    assert!(!painted(&s, "Open 1"), "arrived, the one gone out sleeps");
    assert!(!gui::untracked(|| c.in_view(keys[0])));
    assert!(painted(&s, "Open 2"));
}

/// A slow drag that shows part of the next slide and lets go short of
/// the threshold: the track goes back, and the slide it showed paints
/// until the track has left it.
#[test]
fn carousel_snapping_back_keeps_the_slide_it_showed_until_it_leaves() {
    let held: CarouselHeld = Rc::default();
    let setup = CarouselSetup::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 250.0, b.y + 12.0);
    drag_along(&mut s, (x, y), &[(x - 60.0, 1.0), (x - 80.0, 1.5), (x - 80.0, 2.0)]);
    assert_eq!(c.index(), 0, "short of half, slowly: back");
    assert!(
        painted(&s, "Open 2"),
        "the slide the drag showed paints as the track goes back"
    );
    frames(&mut s, 2.0, 1.0);
    assert!(!painted(&s, "Open 2"), "back at the snap, it sleeps");
    assert!(painted(&s, "Open 1"));
}

#[test]
fn carousel_counts_its_slides_once_a_pass_down_to_none() {
    let held: CarouselHeld = Rc::default();
    let shown = Signal::new(3_usize);
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(0), move |cx| {
            let c = Carousel::new(
                use_model(cx.props(), |b: &Bind<usize>| b),
                CarouselOptions::default,
                None,
                Direction::Ltr,
            );
            h.set(Some(c));
            let keys: Vec<_> = (0..3).map(|_| c.slide_key()).collect();
            move || {
                let slides = keys[..shown.get()].iter().map(|&k| {
                    c.slide(div().width(300.0).height(24.0).flex_shrink(0.0), k)
                        .into_element()
                });
                c.root(div().width(300.0))
                    .child(c.content(div()).child(div().display(Display::Flex).children(slides)))
                    .into_element()
            }
        })
        .into_element()
    });
    let c = held.get().expect("made");
    assert_eq!(c.count(), 3);
    shown.set(2);
    s.settle().expect("settled");
    assert_eq!(c.count(), 2, "one gone");
    shown.set(0);
    s.settle().expect("settled");
    assert_eq!(c.count(), 0, "the last gone: none");
}

#[test]
fn carousel_wraps_when_asked() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        options: CarouselOptions {
            wrap: true,
            ..CarouselOptions::default()
        },
        ..CarouselSetup::default()
    };
    let setup_keys = Rc::clone(&setup.keys);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    assert!(c.looping(), "four slides a view each: one spare either side");
    assert!(c.can_step(false), "wrapping: Previous works on the first");
    let first = s.locate("Open 1").expect("shown");
    click(&mut s, "Prev");
    assert_eq!(c.index(), 3, "back past the first: the last");
    assert_eq!(c.position(), -1.0, "one slide back, not three on");
    let last = setup_keys.borrow()[3];
    assert!(c.in_view(last), "the last is placed before the first");
    s.at(1.0);
    s.settle().expect("settled");
    assert_eq!(c.position(), 3.0, "at rest, back in the first lap");
    assert_eq!(s.locate("Open 4").expect("shown"), first, "nothing moved on screen");
    click(&mut s, "Next");
    assert_eq!(c.index(), 0, "on past the last: the first");
    assert_eq!(c.position(), 4.0, "one slide on, round");
    s.at(2.0);
    s.settle().expect("settled");
    assert_eq!(c.position(), 0.0);
    assert_eq!(s.locate("Open 1").expect("shown"), first);
    // The pointer past the first: no end, the last comes in.
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 50.0, b.y + 12.0);
    drag_along(&mut s, (x, y), &[(x + 100.0, 3.0), (x + 200.0, 3.5), (x + 200.0, 4.0)]);
    assert_eq!(c.index(), 3, "dragged round to the last");
    assert_eq!(c.position(), -1.0);
    // Going round, a far snap is reached the shorter way.
    s.at(5.0);
    s.settle().expect("settled");
    s.act("go", || c.go(0));
    s.settle().expect("settled");
    assert_eq!(c.position(), 4.0, "from the last to the first: one on, not three back");
}

#[test]
fn carousel_fades_one_slide_over_another_in_place() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        options: CarouselOptions {
            fade: true,
            ..CarouselOptions::default()
        },
        ..CarouselSetup::default()
    };
    let settled = Rc::clone(&setup.settled);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    assert_eq!(c.snaps(), 4, "a snap a slide");
    let place = s.locate("Open 1").expect("shown");
    click(&mut s, "Next");
    assert_eq!(c.index(), 1);
    assert_eq!(c.offset().px, 0.0, "the track does not move");
    assert_eq!(
        s.locate("Open 2").expect("shown"),
        place,
        "the slides lie on one another"
    );
    assert_eq!(*settled.borrow(), [1], "nothing travels: settled at once");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 250.0, b.y + 12.0);
    // 20 px over a second: neither far nor fast enough.
    drag_along(&mut s, (x, y), &[(x - 10.0, 1.0), (x - 20.0, 2.0), (x - 20.0, 2.5)]);
    assert_eq!(c.index(), 1, "a short slow swipe stays");
    assert_eq!(c.offset().px, 0.0, "a drag does not move the track");
    // 50 px: past the 40 px a swipe needs.
    drag_along(&mut s, (x, y), &[(x - 25.0, 3.5), (x - 50.0, 4.5), (x - 50.0, 5.0)]);
    assert_eq!(c.index(), 2, "a long swipe steps on");
    // 30 px 50 ms from the press: under 40 px, faster than 0.3 px/ms.
    s.at(6.0);
    drag_along(&mut s, (x - 50.0, y), &[(x - 45.0, 6.02), (x - 20.0, 6.05)]);
    assert_eq!(c.index(), 1, "a quick swipe steps back");
}

/// Let `seconds` pass from `from` a frame at a time, as a display would.
fn frames(s: &mut Scenario, from: f32, seconds: f32) {
    let mut t = from;
    while t < from + seconds {
        t += 0.016;
        s.at(t);
        s.settle().expect("settled");
    }
}

#[test]
fn carousel_auto_scrolls_at_its_speed_and_rests_on_a_snap_when_paused() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        rotation: Some(Rotation::scroll(60.0)),
        ..CarouselSetup::default()
    };
    let renders = Rc::clone(&setup.renders);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    s.hover(700.0, 500.0);
    assert!(c.rotating());
    // The first frame brings the second slide into view (no longer inert):
    // one render; the frames after it move the track alone.
    frames(&mut s, 0.0, 0.016);
    let before = renders.get();
    frames(&mut s, 0.016, 0.984);
    let px = c.offset().px;
    assert!((-64.0..=-56.0).contains(&px), "60 px a second: {px}");
    assert_eq!(renders.get(), before, "a scroll frame renders nothing");
    frames(&mut s, 1.0, 1.6);
    assert_eq!(c.index(), 1, "past half a slide, the next is current");
    // The mouse over it: it rests on the nearest snap.
    let b = bounds(&s, "root").expect("laid out");
    s.hover(b.x + 10.0, b.y + 10.0);
    frames(&mut s, 2.6, 1.0);
    assert_eq!(c.position(), 1.0, "rests on its nearest snap");
    assert!(c.playing() && !c.rotating());
    s.hover(700.0, 500.0);
    frames(&mut s, 3.6, 12.0);
    assert_eq!(c.index(), 3, "on to the end");
    assert_eq!(c.position(), 3.0, "and not past it");
    assert!(!c.playing(), "a track that does not go round stops at its end");
}

#[test]
fn carousel_auto_height_follows_the_slides_in_view() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        options: CarouselOptions {
            auto_height: true,
            ..CarouselOptions::default()
        },
        heights: Some([24.0, 60.0, 40.0, 24.0]),
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    assert_eq!(c.height(), Some(24.0));
    assert_eq!(
        bounds(&s, "content").expect("laid out").height,
        24.0,
        "the viewport is the first's height"
    );
    click(&mut s, "Next");
    s.at(1.0);
    s.settle().expect("settled");
    assert_eq!(c.height(), Some(60.0));
    assert_eq!(
        bounds(&s, "content").expect("laid out").height,
        60.0,
        "and then the second's"
    );
}

#[test]
fn carousel_hands_focus_on_to_the_slide_coming_in() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        focusable: true,
        ..CarouselSetup::default()
    };
    let handles = Rc::clone(&setup.focus);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let first = handles.borrow()[0];
    let second = handles.borrow()[1];
    focus(&mut s, first);
    assert!(focused(first));
    s.act("go", || c.go(1));
    s.settle().expect("settled");
    s.at(1.0);
    s.settle().expect("settled");
    assert!(
        focused(second),
        "focus in the slide going out moves to the one coming in"
    );
}

#[test]
fn carousel_a_drag_moves_with_no_render() {
    use gui::window::Input;
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup::default();
    let renders = Rc::clone(&setup.renders);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 250.0, b.y + 12.0);
    pointer(&mut s, Input::PointerMoved { x, y });
    pointer(&mut s, Input::MouseDown { x, y });
    s.at(0.1);
    pointer(&mut s, Input::PointerMoved { x: x - 10.0, y });
    assert!(c.dragging());
    let started = renders.get();
    for (n, t) in [(20.0, 0.2), (40.0, 0.3), (60.0, 0.4)] {
        s.at(t);
        pointer(&mut s, Input::PointerMoved { x: x - n, y });
    }
    assert_eq!(c.offset().px, -60.0, "the track follows");
    assert_eq!(renders.get(), started, "a drag's moves render nothing");
    pointer(&mut s, Input::MouseUp { x: x - 60.0, y });
}

#[test]
fn carousel_drags_one_snap_by_distance_or_speed_and_not_across() {
    let held: CarouselHeld = Rc::default();
    let setup = CarouselSetup::default();
    let h = Rc::clone(&held);
    let opened = Rc::clone(&setup.opened);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 250.0, b.y + 12.0);
    // Two pixels: no drag.
    drag_along(&mut s, (x, y), &[(x - 2.0, 0.1)]);
    assert!(!c.dragging());
    assert_eq!(c.offset().px, 0.0, "under 3 px nothing moves");
    // Past half a slide, slowly: the nearest snap.
    drag_along(&mut s, (x, y), &[(x - 100.0, 1.0), (x - 180.0, 1.5), (x - 180.0, 2.0)]);
    assert_eq!(c.index(), 1, "past half: the next snap");
    assert_eq!(c.offset().px, 0.0, "the track goes to the snap");
    // A short quick flick: the velocity projects it on.
    drag_along(&mut s, (x, y), &[(x - 20.0, 3.0), (x - 60.0, 3.03)]);
    assert_eq!(c.index(), 2, "a flick of 60 px at 1.3 px/ms: the next snap");
    // A long fast fling still moves one snap.
    drag_along(&mut s, (x, y), &[(x - 100.0, 4.0), (x - 290.0, 4.02)]);
    assert_eq!(c.index(), 3, "one snap at most");
    // A short slow drag goes back.
    drag_along(
        &mut s,
        (x - 200.0, y),
        &[(x - 150.0, 5.0), (x - 140.0, 5.5), (x - 140.0, 6.0)],
    );
    assert_eq!(c.index(), 3, "60 px slowly: back to where it was");
    // Mostly across the axis: the page's, nothing moves.
    use gui::window::Input;
    pointer(&mut s, Input::PointerMoved { x, y });
    pointer(&mut s, Input::MouseDown { x, y });
    pointer(
        &mut s,
        Input::PointerMoved {
            x: x - 5.0,
            y: y + 20.0,
        },
    );
    pointer(
        &mut s,
        Input::PointerMoved {
            x: x - 80.0,
            y: y + 20.0,
        },
    );
    assert!(!c.dragging(), "a move across the axis is the page's");
    pointer(
        &mut s,
        Input::MouseUp {
            x: x - 80.0,
            y: y + 20.0,
        },
    );
    assert_eq!(c.index(), 3);
    assert_eq!(opened.get(), 0, "no drag was a click on a slide's control");
}

#[test]
fn carousel_overscroll_resists_and_bounces_back() {
    let held: CarouselHeld = Rc::default();
    let mut s = plain_carousel(&held);
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 50.0, b.y + 12.0);
    use gui::window::Input;
    pointer(&mut s, Input::PointerMoved { x, y });
    pointer(&mut s, Input::MouseDown { x, y });
    pointer(&mut s, Input::PointerMoved { x: x + 100.0, y });
    assert!(c.dragging());
    assert!((c.offset().px - 30.0).abs() < 1e-3, "100 px before the first moves 30");
    assert_eq!(c.move_duration(), Duration::ZERO, "the track follows the pointer");
    pointer(&mut s, Input::MouseUp { x: x + 100.0, y });
    assert_eq!(c.index(), 0);
    assert_eq!(c.offset().px, 0.0);
    assert_eq!(
        c.move_duration(),
        primitives::carousel::BOUNCE,
        "it springs back over 250 ms"
    );
}

#[test]
fn carousel_a_drag_is_no_click_and_a_press_is() {
    let held: CarouselHeld = Rc::default();
    let setup = CarouselSetup::default();
    let h = Rc::clone(&held);
    let opened = Rc::clone(&setup.opened);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let b = bounds(&s, "open").expect("laid out");
    let (x, y) = (b.x + 50.0, b.y + 12.0);
    drag_along(&mut s, (x, y), &[(x - 40.0, 0.5), (x - 40.0, 1.0)]);
    assert_eq!(opened.get(), 0, "a drag of 40 px is not a click");
    click(&mut s, "Open 1");
    assert_eq!(opened.get(), 1, "a press is");
    drag_along(&mut s, (x, y), &[(x - 5.0, 1.5)]);
    assert_eq!(opened.get(), 2, "a drag under 10 px still clicks");
}

#[test]
fn carousel_drag_free_glides_and_rests_off_its_snap() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        options: CarouselOptions {
            drag_free: true,
            ..CarouselOptions::default()
        },
        ..CarouselSetup::default()
    };
    let heard = Rc::clone(&setup.settled);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    let (x, y) = (b.x + 250.0, b.y + 12.0);
    drag_along(&mut s, (x, y), &[(x - 20.0, 1.0), (x - 50.0, 1.03)]);
    let after = c.offset().px;
    s.at(4.0);
    s.settle().expect("settled");
    // 1 px/ms, decaying by 0.996 a millisecond: about 250 px further.
    let total = 300.0 * -c.offset().fraction - c.offset().px;
    assert!(
        total > 200.0 && total < 320.0,
        "it glides on: {total} px (released at {after})"
    );
    assert_eq!(c.index(), 1, "the nearest snap");
    assert!(c.offset().px.abs() > 1.0, "resting where it stopped, off the snap");
    assert_eq!(*heard.borrow(), [1], "settled once, at rest");
}

#[test]
fn carousel_wheel_pages_one_snap_a_gesture() {
    use gui::window::Input;
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        options: CarouselOptions {
            wheel: true,
            ..CarouselOptions::default()
        },
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    s.hover(b.x + 100.0, b.y + 12.0);
    s.wheel(-40.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0, "towards the start at the first: the page's");
    s.at(1.0);
    s.wheel(20.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0, "under 30: no page yet");
    s.wheel(15.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "30 of delta: one snap");
    s.wheel(80.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "one gesture, one snap");
    s.at(1.5);
    s.send(Input::Wheel { dx: 40.0, dy: 0.0 });
    s.settle().expect("settled");
    assert_eq!(c.index(), 2, "after a pause, along the axis: a new gesture");
}

#[test]
fn carousel_wheel_across_the_axis_is_the_pages_unless_asked() {
    let held: CarouselHeld = Rc::default();
    let mut s = plain_carousel(&held);
    let c = held.get().expect("made");
    let b = bounds(&s, "content").expect("laid out");
    s.hover(b.x + 100.0, b.y + 12.0);
    s.wheel(80.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0);
}

#[test]
fn carousel_rotates_on_its_delay_and_pauses_under_the_mouse() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        rotation: Some(Rotation::step(Duration::from_secs(2))),
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    assert!(c.rotating());
    s.hover(700.0, 500.0);
    s.at(1.9);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0, "before its delay");
    s.at(2.1);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "a step every delay");
    let b = bounds(&s, "root").expect("laid out");
    s.hover(b.x + 10.0, b.y + 10.0);
    s.at(6.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "paused while the mouse is over it");
    assert!(!c.rotating() && c.playing(), "paused, still playing");
    s.hover(700.0, 500.0);
    s.at(7.9);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "leaving counts the delay again");
    s.at(8.1);
    s.settle().expect("settled");
    assert_eq!(c.index(), 2);
    s.at(12.1);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0, "after the last, the first");
    click(&mut s, "Next");
    assert!(!c.playing(), "a press stops it when asked");
    s.at(20.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1);
    click(&mut s, "Rotate");
    assert!(c.playing(), "the rotation button plays it again");
    s.hover(700.0, 500.0);
    s.at(22.1);
    s.settle().expect("settled");
    assert_eq!(c.index(), 2);
}

#[test]
fn carousel_rotation_pauses_hidden_and_stops_for_keyboard_focus_and_reduced_motion() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        rotation: Some(Rotation::step(Duration::from_secs(2))),
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    s.act("hide", || gui::environment::set_window_visible(false));
    s.settle().expect("settled");
    assert!(c.paused(), "a hidden window pauses it");
    s.at(3.0);
    s.settle().expect("settled");
    assert_eq!(c.index(), 0);
    s.act("show", || gui::environment::set_window_visible(true));
    s.at(5.1);
    s.settle().expect("settled");
    assert_eq!(c.index(), 1, "seen again, it counts its delay afresh");
    s.act("window", || gui::environment::set_window_size((800.0, 600.0)));
    s.settle().expect("settled");
    assert!(!c.paused(), "in the window");
    // Keyboard focus coming in stops it.
    plain(&mut s, NamedKey::Tab);
    assert!(!c.playing(), "keyboard focus in the carousel stops it");
    click(&mut s, "Rotate");
    assert!(c.playing());
    s.act("reduce", || gui::environment::set_reduced_motion(true));
    s.settle().expect("settled");
    assert!(!c.playing(), "reduced motion stops it");
    assert_eq!(c.move_duration(), Duration::ZERO, "and moves at once");
}

#[test]
fn carousel_slides_out_of_view_are_hidden_and_named_by_place() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        animated: true,
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    s.play_motion();
    let tree = a11y(&s);
    assert!(tree.contains("\"1 of 4\""), "a slide is named by its place:\n{tree}");
    assert!(!tree.contains("\"2 of 4\""), "slides out of view are hidden:\n{tree}");
    let view = bounds(&s, "content").expect("laid out");
    click(&mut s, "Next");
    // Part way through the move: the track is between the two snaps, both
    // slides on screen, and both paint.
    s.at(0.1);
    s.settle().expect("settled");
    let on_screen = |s: &Scenario, label: &str| {
        s.last().nodes.iter().any(|n| {
            !n.look.invisible
                && n.bounds.x + n.bounds.width > view.x + 1.0
                && n.bounds.x < view.x + view.width - 1.0
                && matches!(&n.content, gui::layout::Content::Text { content, .. } if &**content == label)
        })
    };
    assert!(
        on_screen(&s, "Open 1"),
        "the slide going out is still on screen and paints"
    );
    assert!(on_screen(&s, "Open 2"), "and the one coming in");
    let tree = a11y(&s);
    assert!(
        tree.contains("\"2 of 4\"") && tree.contains("\"1 of 4\""),
        "on its way, both on screen are read:\n{tree}"
    );
    s.wait(Duration::from_millis(1000));
    s.settle().expect("settled");
    let tree = a11y(&s);
    assert!(tree.contains("\"2 of 4\"") && !tree.contains("\"1 of 4\""), "{tree}");
}

#[test]
fn carousel_settles_once_per_snap_after_its_move() {
    let held: CarouselHeld = Rc::default();
    let setup = CarouselSetup::default();
    let h = Rc::clone(&held);
    let heard = Rc::clone(&setup.settled);
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    click(&mut s, "Next");
    assert!(heard.borrow().is_empty(), "on its way");
    s.at(0.5);
    s.settle().expect("settled");
    assert_eq!(*heard.borrow(), [1], "at rest after 400 ms");
}

#[test]
fn carousel_indicators_are_tabs_moving_by_arrows() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        indicators: true,
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    click(&mut s, "Dot 3");
    assert_eq!(c.index(), 2, "a dot shows its snap");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(c.index(), 3, "arrows move to the next tab and show it");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(c.index(), 0, "wrapping");
    plain(&mut s, NamedKey::End);
    assert_eq!(c.index(), 3);
    // Arrived: the slides it passed on its way sleep again.
    s.wait(Duration::from_millis(1000));
    s.settle().expect("settled");
    insta::assert_snapshot!("carousel_indicators_a11y", a11y(&s));
}

#[test]
fn carousel_a11y() {
    let held: CarouselHeld = Rc::default();
    let s = plain_carousel(&held);
    insta::assert_snapshot!("carousel_a11y", a11y(&s));
}

#[test]
fn carousel_lone_indicators_go_to_their_slide_and_control_the_content() {
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let setup = CarouselSetup {
        lone_indicators: true,
        ..CarouselSetup::default()
    };
    let mut s = scenario(move || carousel(Bind::default_value(0), &setup, &h));
    let c = held.get().expect("made");
    insta::assert_snapshot!("carousel_lone_indicators_a11y", a11y(&s));
    click(&mut s, "Dot 3");
    s.wait(Duration::from_millis(1000));
    s.settle().expect("settled");
    assert_eq!(c.index(), 2, "a lone indicator goes to its slide");
    press(&mut s, "Dot 3");
    assert_eq!(c.index(), 2, "the current one is unavailable");
    let tree = a11y(&s);
    assert!(
        tree.lines()
            .any(|l| l.contains("\"Go to slide 3\"") && l.contains("disabled") && l.contains("controls=")),
        "{tree}"
    );
}

// ------------------------------------------------- Accordion, more of it

fn accordion_many(open: Bind<Vec<&'static str>>, held: &Held<Sections>) -> Element {
    let held = Rc::clone(held);
    component(open, move |cx| {
        let accordion = Accordion::multiple(
            use_model(cx.props(), |b: &Bind<Vec<&'static str>>| b),
            AccordionOptions::default,
            Direction::Ltr,
        );
        held.set(Some(Sections { accordion }));
        let sections = ["a", "b", "c"].map(|v| accordion.section(SectionItem::new(v)));
        move || {
            div()
                .children(sections.iter().zip(["A", "B", "C"]).map(|(sec, name)| {
                    div()
                        .child(sec.header(div()).child(sec.trigger(label("trigger", name))))
                        .child(sec.content(label("content", &format!("{name} body"))))
                        .into_element()
                }))
                .into_element()
        }
    })
    .into_element()
}

#[test]
fn accordion_multiple_opens_any_and_the_one_opened_goes_last() {
    let heard: Rc<RefCell<Vec<Vec<&'static str>>>> = Rc::default();
    let h2 = Rc::clone(&heard);
    let held: Held<Sections> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h2 = Rc::clone(&h2);
        accordion_many(
            Bind::default_value(vec!["b"]).on_change(move |v| h2.borrow_mut().push(v)),
            &h,
        )
    });
    let a = held.get().expect("made").accordion;
    click(&mut s, "C");
    click(&mut s, "A");
    assert!(a.is_open(&"a") && a.is_open(&"b") && a.is_open(&"c"), "any number open");
    click(&mut s, "B");
    assert!(!a.is_open(&"b"), "a press closes an open one, collapsible or not");
    assert_eq!(
        *heard.borrow(),
        [vec!["b", "c"], vec!["b", "c", "a"], vec!["c", "a"]],
        "the one opened goes last"
    );
}

#[test]
fn accordion_controlled_by_a_value_only_asks() {
    let heard: Rc<RefCell<Vec<Option<&'static str>>>> = Rc::default();
    let h2 = Rc::clone(&heard);
    let options = Signal::new(AccordionOptions::default());
    let held: Held<Sections> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h2 = Rc::clone(&h2);
        accordion(
            Bind::value(Some("a")).on_change(move |v| h2.borrow_mut().push(v)),
            options,
            &h,
        )
    });
    let a = held.get().expect("made").accordion;
    click(&mut s, "C");
    assert_eq!(*heard.borrow(), [Some("c")], "asked");
    assert!(a.is_open(&"a") && !a.is_open(&"c"), "the caller's value is shown");
}

// ------------------------------------------------------ Tabs, more of it

#[test]
fn tabs_indicator_follows_the_selected_tab_from_the_padding_box() {
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value("a"), move |cx| {
            let tabs = Tabs::new(
                use_model(cx.props(), |b: &Bind<&'static str>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            let focus = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
            h.set(Some((tabs, focus)));
            move || {
                let tab = |i: usize, v: &'static str, name: &'static str| {
                    tabs.trigger(label("tab", name), v, Availability::Enabled, focus[i])
                };
                let mut list = tabs
                    .list(div().named("list").display(Display::Flex).padding(4.0).border(2.0))
                    .child(tab(0, "a", "One"))
                    .child(tab(1, "b", "Two"))
                    .child(tab(2, "c", "Three"));
                if let Some(bar) = tabs.indicator_part(div().named("indicator").height(2.0)) {
                    list = list.child(bar);
                }
                list.into_element()
            }
        })
        .into_element()
    });
    let (t, _) = held.get().expect("made");
    let at = t.indicator().expect("measured");
    assert_eq!(
        (at.position, at.size, at.thickness),
        (4.0, 100.0, 24.0),
        "from the padding box"
    );
    click(&mut s, "Three");
    let at = t.indicator().expect("measured");
    assert_eq!(at.position, 204.0, "it follows the selection");
    let list = bounds(&s, "list").expect("laid out");
    let bar = bounds(&s, "indicator").expect("laid out");
    assert_eq!(
        (bar.x - list.x, bar.width),
        (2.0 + 204.0, 100.0),
        "the part is placed by it"
    );
}

#[test]
fn tabs_keys_with_modifiers_pages_and_leaving_backwards() {
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        div()
            .child(label("before", "Before").focusable())
            .child(tabs(Bind::default_value("a"), options, &h))
            .into_element()
    });
    let (t, f) = held.get().expect("made");
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::PageDown);
    assert!(focused(f[2]) && t.value() == "c", "PageDown: the last usable");
    plain(&mut s, NamedKey::PageUp);
    assert!(focused(f[0]) && t.value() == "a", "PageUp: the first");
    for m in [
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
        Modifiers {
            alt: true,
            ..Modifiers::default()
        },
        Modifiers {
            command: true,
            ..Modifiers::default()
        },
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    ] {
        key(&mut s, NamedKey::ArrowRight, m);
        assert!(focused(f[0]) && t.value() == "a", "{m:?}: not handled");
    }
    shift(&mut s, NamedKey::Tab);
    assert!(
        !focused(f[0]) && !focused(f[2]) && gui::focus::focused().is_some(),
        "Shift+Tab leaves the list at once"
    );
}

#[test]
fn tabs_control_press_neither_selects_nor_focuses() {
    use gui::window::Input;
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || tabs(Bind::default_value("a"), options, &h));
    let (t, f) = held.get().expect("made");
    let (x, y) = s.locate("Three").expect("shown");
    s.send(Input::Modifiers(Modifiers {
        control: true,
        ..Modifiers::default()
    }));
    s.send(Input::PointerMoved { x, y });
    s.send(Input::MouseDown { x, y });
    s.send(Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(t.value(), "a", "a Control press is a secondary click: no selection");
    assert!(!focused(f[2]), "and no focus");
    s.send(Input::Modifiers(Modifiers::default()));
    click(&mut s, "Three");
    assert!(focused(f[2]) && t.value() == "c");
}

#[test]
fn tabs_controlled_by_a_value_only_asks() {
    let heard: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let h2 = Rc::clone(&heard);
    let options = Signal::new(TabsOptions::default());
    let held: TabsHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h2 = Rc::clone(&h2);
        tabs(
            Bind::value("a").on_change(move |v| h2.borrow_mut().push(v)),
            options,
            &h,
        )
    });
    let (t, _) = held.get().expect("made");
    click(&mut s, "Three");
    assert_eq!(*heard.borrow(), ["c"]);
    assert_eq!(t.value(), "a", "the caller's value is shown");
}

// ------------------------------------------------------- Avatar, AspectRatio

#[test]
fn avatar_a11y() {
    let s = scenario(move || {
        component((), move |_| {
            let avatar = Avatar::new();
            let _image = avatar.image(|| ImageLoad::Failed, |_| {});
            let fallback = Fallback::new(None, move || avatar.status());
            move || {
                div()
                    .named("avatar")
                    .when(fallback.shown(), |d| d.child(label("fallback", "CN")))
                    .into_element()
            }
        })
        .into_element()
    });
    insta::assert_snapshot!("avatar_a11y", a11y(&s));
}

#[test]
fn avatar_without_an_image_is_idle_and_shows_its_fallback() {
    let held: Held<Avatar> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let avatar = Avatar::new();
            h.set(Some(avatar));
            let fallback = Fallback::new(None, move || avatar.status());
            move || {
                div()
                    .when(fallback.shown(), |d| d.child(label("fallback", "CN")))
                    .into_element()
            }
        })
        .into_element()
    });
    assert_eq!(held.get().expect("made").status(), Status::Idle);
    assert!(has(&s, "fallback"));
}

#[test]
fn aspect_ratio_a11y() {
    let s = scenario(move || {
        div()
            .width(320.0)
            .child(frame(4.0 / 3.0, div().named("ratio").child(label("photo", "Photo"))))
            .into_element()
    });
    insta::assert_snapshot!("aspect_ratio_a11y", a11y(&s));
}

// ------------------------------------------------- ScrollArea, more of it

type AreaHeld = Held<ScrollArea>;

/// A 300 by 200 area over content `w` by `h`, its bars 10 px thick, their
/// thumbs filling them across.
fn area(options: Signal<ScrollAreaOptions>, direction: Direction, (w, h): (f32, f32), held: &AreaHeld) -> Element {
    let held = Rc::clone(held);
    component((), move |_| {
        let a = ScrollArea::new(move || options.get(), direction);
        held.set(Some(a));
        move || {
            let bar = |axis: Axis, name: &'static str| {
                let d = div().named(name).display(Display::Flex);
                let d = match axis {
                    Axis::Vertical => d.width(10.0),
                    Axis::Horizontal => d.height(10.0),
                };
                let thumb = a.thumb(div().named("thumb").flex_grow(1.0), axis);
                a.scrollbar(d.child(thumb), axis)
            };
            let mut root = a
                .root(
                    div()
                        .named("area")
                        .position(Position::Relative)
                        .width(300.0)
                        .height(200.0),
                )
                .child(
                    a.viewport(div().named("viewport").size(pct(100.0)))
                        .child(div().named("body").width(w).height(h)),
                );
            let bars = a.options().bars;
            if bars.vertical() {
                root = root.child(bar(Axis::Vertical, "vbar"));
            }
            if bars.horizontal() {
                root = root.child(bar(Axis::Horizontal, "hbar"));
            }
            if let Some(c) = a.corner(div().named("corner")) {
                root = root.child(c);
            }
            root.into_element()
        }
    })
    .into_element()
}

fn area_options(policy: Policy, bars: Bars) -> Signal<ScrollAreaOptions> {
    Signal::new(ScrollAreaOptions {
        policy,
        bars,
        ..ScrollAreaOptions::default()
    })
}

#[test]
fn scroll_area_hover_shows_at_once_and_hides_600_ms_after_leaving() {
    let options = area_options(Policy::Hover, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 1000.0), &h));
    let a = held.get().expect("made");
    assert!(!a.shown(Axis::Vertical), "hidden until the pointer comes");
    // The overflow is checked 10 ms after the content's size settles.
    s.at(0.1);
    s.settle().expect("settled");
    let b = bounds(&s, "area").expect("laid out");
    s.hover(b.x + 100.0, b.y + 100.0);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "the pointer in: shown at once");
    s.at(1.0);
    s.hover(700.0, 550.0);
    s.settle().expect("settled");
    s.at(1.59);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "before 600 ms");
    s.at(1.61);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "600 ms after leaving");
}

#[test]
fn scroll_area_hover_never_shows_a_bar_without_overflow() {
    let options = area_options(Policy::Hover, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 100.0), &h));
    let a = held.get().expect("made");
    let b = bounds(&s, "area").expect("laid out");
    s.hover(b.x + 100.0, b.y + 100.0);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical));
    assert!(!a.has_thumb(Axis::Vertical));
}

#[test]
fn scroll_area_always_and_auto() {
    let options = area_options(Policy::Always, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 100.0), &h));
    let a = held.get().expect("made");
    assert!(a.shown(Axis::Vertical), "always: even with nothing to scroll");
    assert!(!a.has_thumb(Axis::Vertical), "but no thumb");
    s.act("auto", || {
        options.set(ScrollAreaOptions {
            policy: Policy::Auto,
            ..options.get_untracked()
        })
    });
    s.at(0.1);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "auto: hidden without overflow");
}

#[test]
fn scroll_area_auto_shows_10_ms_after_the_content_overflows() {
    let options = area_options(Policy::Auto, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let tall = Signal::new(100.0_f32);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let a = ScrollArea::new(move || options.get(), Direction::Ltr);
            h.set(Some(a));
            move || {
                a.root(
                    div()
                        .named("area")
                        .position(Position::Relative)
                        .width(300.0)
                        .height(200.0),
                )
                .child(
                    a.viewport(div().size(pct(100.0)))
                        .child(div().width(10.0).height(move || Length::Px(tall.get()))),
                )
                .into_element()
            }
        })
        .into_element()
    });
    let a = held.get().expect("made");
    s.at(0.1);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical));
    s.act("taller", || tall.set(1000.0));
    s.settle().expect("settled");
    let t = gui::clock::current_seconds();
    s.at(t + 0.005);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "checked 10 ms after the change");
    s.at(t + 0.02);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "overflowing: shown");
}

#[test]
fn scroll_area_scroll_shows_while_scrolling_then_waits_the_delay() {
    let options = area_options(Policy::Scroll, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 1000.0), &h));
    let a = held.get().expect("made");
    assert!(!a.shown(Axis::Vertical));
    s.at(1.0);
    s.act("scroll", || a.handle().scroll_to((0.0, 100.0)));
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "a scroll shows it");
    s.at(1.69);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "100 ms to end the scroll, then 600 ms");
    s.at(1.71);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "hidden after both");
    // The pointer on the bar keeps it.
    s.at(2.0);
    s.act("scroll", || a.handle().scroll_to((0.0, 200.0)));
    s.settle().expect("settled");
    let bar = bounds(&s, "vbar").expect("shown");
    s.hover(bar.x + 5.0, bar.y + 50.0);
    s.at(4.0);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "the pointer on the bar keeps it");
    s.hover(700.0, 550.0);
    s.at(4.59);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical));
    s.at(4.61);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "leaving it: the delay");
}

#[test]
fn scroll_area_glimpse_shows_on_entry_and_hides_though_the_pointer_stays() {
    let options = area_options(Policy::Glimpse, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 1000.0), &h));
    let a = held.get().expect("made");
    let b = bounds(&s, "area").expect("laid out");
    s.at(1.0);
    s.hover(b.x + 100.0, b.y + 100.0);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical), "entering shows it");
    s.at(1.59);
    s.settle().expect("settled");
    assert!(a.shown(Axis::Vertical));
    s.at(1.61);
    s.settle().expect("settled");
    assert!(!a.shown(Axis::Vertical), "hidden after the delay, the pointer still in");
}

#[test]
fn scroll_area_thumb_sizes_drags_keeping_its_grab_and_the_track_jumps() {
    let options = area_options(Policy::Always, Bars::Vertical);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || area(options, Direction::Ltr, (10.0, 1000.0), &h));
    let a = held.get().expect("made");
    let thumb = a.thumb_at(Axis::Vertical).expect("a thumb");
    assert_eq!(thumb.length, 40.0, "200 of 1000 of a 200 px track");
    let bar = bounds(&s, "vbar").expect("shown");
    // Grab the thumb 10 px into it and drag 16 px: 16 of 160 px of travel.
    s.drag((bar.x + 5.0, bar.y + 10.0), (bar.x + 5.0, bar.y + 26.0));
    s.settle().expect("settled");
    assert_eq!(a.handle().offset().1, 80.0, "16 px of 160: a tenth of 800");
    assert_eq!(
        a.thumb_at(Axis::Vertical).expect("a thumb").offset,
        16.0,
        "the grab point stays under the pointer"
    );
    // A press on the track: the thumb's centre comes under it.
    s.drag((bar.x + 5.0, bar.y + 150.0), (bar.x + 5.0, bar.y + 150.0));
    s.settle().expect("settled");
    assert_eq!(
        a.thumb_at(Axis::Vertical).expect("a thumb").offset,
        130.0,
        "centred at 150"
    );
    assert_eq!(a.handle().offset().1, 650.0);
    // The wheel over the bar scrolls its axis.
    s.hover(bar.x + 5.0, bar.y + 100.0);
    assert!(s.wheel(-100.0), "taken between the ends");
    s.settle().expect("settled");
    assert_eq!(a.handle().offset().1, 550.0);
}

#[test]
fn scroll_area_corner_and_right_to_left_placement() {
    let options = area_options(Policy::Always, Bars::Both);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || area(options, Direction::Rtl, (1000.0, 1000.0), &h));
    let a = held.get().expect("made");
    assert_eq!(a.corner_size(), Some((10.0, 10.0)), "both bars show: the corner");
    let root = bounds(&s, "area").expect("laid out");
    let v = bounds(&s, "vbar").expect("shown");
    let hb = bounds(&s, "hbar").expect("shown");
    let corner = bounds(&s, "corner").expect("shown");
    assert_eq!(v.x, root.x, "right to left the vertical bar is on the left");
    assert_eq!(v.height, 190.0, "down to the corner");
    assert_eq!((hb.x, hb.width), (root.x + 10.0, 290.0), "from the right to the corner");
    assert_eq!((corner.x, corner.y), (root.x, root.y + 190.0), "bottom left");
}

#[test]
fn scroll_area_a11y() {
    let options = area_options(Policy::Always, Bars::Both);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || area(options, Direction::Ltr, (1000.0, 1000.0), &h));
    insta::assert_snapshot!("scroll_area_a11y", a11y(&s));
}

// --------------------------------------------------- Splitter, more of it

/// A panel's index, its new size and the one it was last told.
type Resized = (usize, f64, Option<f64>);

/// What a pixel splitter test hears.
#[derive(Clone, Default)]
struct SplitHeard {
    layouts: Rc<RefCell<Vec<Vec<f64>>>>,
    resized: Rc<RefCell<Vec<Resized>>>,
    collapsed: Rc<Cell<usize>>,
    expanded: Rc<Cell<usize>>,
}

type PxHeld = Held<(Splitter<Bind<Vec<f64>>>, FocusHandle)>;

/// Two panels in a group `width` wide (a 4 px handle between): the first
/// 120 px by default, 60 px at least, collapsible to 0; the second in
/// percent. Saved in `storage` when given.
fn px_splitter(
    width: Signal<f32>,
    storage: Option<Rc<dyn primitives::splitter::Storage>>,
    heard: &SplitHeard,
    held: &PxHeld,
) -> Element {
    use primitives::splitter::{Panel, Unit};
    let (held, heard) = (Rc::clone(held), heard.clone());
    component(Bind::default_value(Vec::new()), move |cx| {
        let layouts = Rc::clone(&heard.layouts);
        let mut sp = Splitter::new(
            use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
            SplitterOptions::default,
            Direction::Ltr,
        )
        .on_layout(move |l: Vec<f64>| layouts.borrow_mut().push(l));
        if let Some(storage) = storage.clone() {
            sp = sp.persist("test", storage);
        }
        let (r0, r1) = (Rc::clone(&heard.resized), Rc::clone(&heard.resized));
        let (c0, e0) = (Rc::clone(&heard.collapsed), Rc::clone(&heard.expanded));
        sp.register(
            0,
            Panel {
                constraints: Constraints {
                    min: 60.0,
                    collapsible: true,
                    default: Some(120.0),
                    unit: Unit::Pixels,
                    ..Constraints::default()
                },
                on_resize: Some(gui::Callback::new(move |(now, was)| {
                    r0.borrow_mut().push((0, now, was))
                })),
                on_collapse: Some(gui::Callback::new(move |()| c0.set(c0.get() + 1))),
                on_expand: Some(gui::Callback::new(move |()| e0.set(e0.get() + 1))),
                ..Panel::default()
            },
        );
        sp.register(
            1,
            Panel {
                on_resize: Some(gui::Callback::new(move |(now, was)| {
                    r1.borrow_mut().push((1, now, was))
                })),
                ..Panel::default()
            },
        );
        let focus = FocusHandle::new();
        held.set(Some((sp, focus)));
        move || {
            sp.group(
                div()
                    .named("group")
                    .display(Display::Flex)
                    .width(move || Length::Px(width.get()))
                    .height(100.0),
            )
            .child(sp.panel(div().named("p0").child(label("inside", "Inside")), 0))
            .child(sp.handle(
                div().named("handle").width(4.0).focusable(),
                0,
                Availability::Enabled,
                focus,
            ))
            .child(sp.panel(div().named("p1"), 1))
            .into_element()
        }
    })
    .into_element()
}

#[test]
fn splitter_pixel_panel_keeps_its_pixels_and_every_layout_is_told() {
    let width = Signal::new(604.0_f32);
    let heard = SplitHeard::default();
    let held: PxHeld = Rc::default();
    let (h, hd) = (Rc::clone(&held), heard.clone());
    let mut s = scenario(move || px_splitter(width, None, &hd, &h));
    let (sp, f) = held.get().expect("made");
    assert_eq!(bounds(&s, "p0").expect("laid out").width, 120.0, "120 px of 600");
    assert_eq!(sp.layout(), [20.0, 80.0]);
    assert_eq!(
        *heard.layouts.borrow(),
        [vec![120.0, 80.0]],
        "the first layout is told, in each panel's unit"
    );
    assert_eq!(
        *heard.resized.borrow(),
        [(0, 120.0, None), (1, 80.0, None)],
        "each panel hears its first size"
    );
    assert_eq!(
        heard.expanded.get(),
        1,
        "an open collapsible panel is told it expanded, the first time too"
    );
    s.act("wider", || width.set(1204.0));
    s.settle().expect("settled");
    assert_eq!(
        bounds(&s, "p0").expect("laid out").width,
        120.0,
        "the pixel panel keeps its pixels"
    );
    assert_eq!(sp.layout(), [10.0, 90.0]);
    assert_eq!(heard.layouts.borrow().last(), Some(&vec![120.0, 90.0]));
    focus(&mut s, f);
    shift(&mut s, NamedKey::ArrowLeft);
    assert_eq!(sp.layout()[0], 0.0, "pushed under its min: collapsed");
    assert_eq!(heard.collapsed.get(), 1);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(heard.expanded.get(), 2);
}

#[test]
fn splitter_saves_its_layout_100_ms_after_the_last_change_and_starts_from_it() {
    use primitives::splitter::{MemoryStorage, Storage};
    let width = Signal::new(604.0_f32);
    let shown = Signal::new(true);
    let storage = MemoryStorage::default();
    let store: Rc<dyn Storage> = Rc::new(storage.clone());
    let heard = SplitHeard::default();
    let held: PxHeld = Rc::default();
    let (h, hd) = (Rc::clone(&held), heard);
    let mut s = scenario(move || {
        if shown.get() {
            px_splitter(width, Some(Rc::clone(&store)), &hd, &h)
        } else {
            Element::Empty
        }
    });
    let (sp, f) = held.get().expect("made");
    s.at(1.0);
    s.settle().expect("settled");
    focus(&mut s, f);
    plain(&mut s, NamedKey::ArrowRight);
    let after = sp.layout();
    assert_eq!(after, [30.0, 70.0]);
    let key_of = |s: &MemoryStorage| s.load(&find_key(s)).map(|l| l.layout);
    s.at(1.09);
    s.settle().expect("settled");
    assert_ne!(
        key_of(&storage),
        Some(after.clone()),
        "not yet: 100 ms after the last change"
    );
    s.at(1.11);
    s.settle().expect("settled");
    assert_eq!(key_of(&storage), Some(after.clone()), "saved");
    s.act("unmount", || shown.set(false));
    s.settle().expect("settled");
    s.act("mount", || shown.set(true));
    s.settle().expect("settled");
    let (sp, _) = held.get().expect("made again");
    assert_eq!(sp.layout(), after, "it starts from what it saved");
}

/// The one key a [`primitives::splitter::MemoryStorage`] holds.
fn find_key(s: &primitives::splitter::MemoryStorage) -> String {
    format!("{s:?}")
        .split('"')
        .nth(1)
        .map(str::to_owned)
        .unwrap_or_default()
}

#[test]
fn splitter_handle_reach_cursor_and_context_menu() {
    use gui::window::Input;
    let width = Signal::new(604.0_f32);
    let heard = SplitHeard::default();
    let held: PxHeld = Rc::default();
    let (h, hd) = (Rc::clone(&held), heard);
    let mut s = scenario(move || px_splitter(width, None, &hd, &h));
    let (sp, _) = held.get().expect("made");
    let b = bounds(&s, "handle").expect("laid out");
    let y = b.y + 50.0;
    // 4 px left of the handle, over the panel: within its 5 px reach.
    s.drag((b.x - 4.0, y), (b.x + 56.0, y));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), [30.0, 70.0], "a press within 5 px drags it");
    // 8 px off: out of reach.
    s.drag((b.x + 60.0 - 8.0, y), (b.x + 120.0, y));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), [30.0, 70.0], "8 px off the handle: nothing");
    assert_eq!(sp.cursor(0), gui::Cursor::ResizeEastWest, "both ways");
    // Dragged past the second panel's room: one way only.
    let b = bounds(&s, "handle").expect("laid out");
    pointer(&mut s, Input::PointerMoved { x: b.x + 2.0, y });
    pointer(&mut s, Input::MouseDown { x: b.x + 2.0, y });
    pointer(&mut s, Input::PointerMoved { x: b.x - 300.0, y });
    assert_eq!(sp.dragging(), Some(0));
    assert_eq!(sp.layout()[0], 0.0, "collapsed at the start");
    assert_eq!(
        sp.cursor(0),
        gui::Cursor::ResizeEastWest,
        "a move that changed the layout"
    );
    // A move that cannot change it: past the limit (reka's constraint report).
    pointer(&mut s, Input::PointerMoved { x: b.x - 400.0, y });
    assert_eq!(
        sp.cursor(0),
        gui::Cursor::ResizeEast,
        "past its limit: only the way it can go"
    );
    let g = bounds(&s, "group").expect("laid out");
    s.send(Input::ButtonDown {
        x: g.x + 300.0,
        y,
        button: gui::layer::PointerButton::Secondary,
    });
    s.settle().expect("settled");
    assert_eq!(sp.dragging(), None, "the context menu ends the drag");
    pointer(&mut s, Input::MouseUp { x: b.x - 400.0, y });
}

type LeavingSplitter = Splitter<(Bind<Vec<f64>>, bool)>;

#[test]
fn splitter_forgets_what_it_kept_of_a_panel_that_left() {
    let shown = Signal::new(true);
    let held: Held<(LeavingSplitter, [FocusHandle; 2])> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((Bind::default_value(Vec::<f64>::new()), shown.get()), move |cx| {
            let props = cx.props();
            let sp = Splitter::new(
                use_model(props, |b: &(Bind<Vec<f64>>, bool)| &b.0),
                SplitterOptions::default,
                Direction::Ltr,
            );
            let focus = [FocusHandle::new(), FocusHandle::new()];
            h.set(Some((sp, focus)));
            let panel = move |i: usize| {
                component(i, move |cx| {
                    let i = cx.props().with_untracked(|i| *i);
                    sp.register(i, Constraints::default());
                    move || sp.panel(div().named("panel").flex(Flex::grow(1.0)), i).into_element()
                })
                .into_element()
            };
            move || {
                let mut group = sp
                    .group(div().display(Display::Flex).width(602.0).height(50.0))
                    .child(panel(0))
                    .child(sp.handle(div().width(2.0).focusable(), 0, Availability::Enabled, focus[0]));
                if props.with(|p| p.1) {
                    group = group.child(panel(1));
                }
                group.into_element()
            }
        })
        .into_element()
    });
    let (sp, _) = held.get().expect("made");
    assert_eq!(sp.layout(), [50.0, 50.0]);
    s.act("leave", || shown.set(false));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), [100.0], "the panel left: worked out afresh");
}

/// A panel replaced at its index in one update (keyed anew): the new one
/// registers before the old one leaves, and the old one's leaving takes
/// back only its own registration, so the group still has both panels.
#[test]
fn splitter_keeps_a_panel_replaced_at_its_index_in_one_update() {
    let which = Signal::new(false);
    let held: Held<(LeavingSplitter, [FocusHandle; 2])> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((Bind::default_value(Vec::<f64>::new()), true), move |cx| {
            let props = cx.props();
            let sp = Splitter::new(
                use_model(props, |b: &(Bind<Vec<f64>>, bool)| &b.0),
                SplitterOptions::default,
                Direction::Ltr,
            );
            let focus = [FocusHandle::new(), FocusHandle::new()];
            h.set(Some((sp, focus)));
            let panel = move |i: usize| {
                component(i, move |cx| {
                    let i = cx.props().with_untracked(|i| *i);
                    sp.register(i, Constraints::default());
                    move || sp.panel(div().named("panel").flex(Flex::grow(1.0)), i).into_element()
                })
                .into_element()
            };
            move || {
                sp.group(div().display(Display::Flex).width(602.0).height(50.0))
                    .child(panel(0))
                    .child(sp.handle(div().width(2.0).focusable(), 0, Availability::Enabled, focus[0]))
                    .child(
                        div()
                            .key(gui::key_of(if which.get() { "second" } else { "first" }))
                            .display(Display::Flex)
                            .flex(Flex::grow(1.0))
                            .child(panel(1)),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let (sp, _) = held.get().expect("made");
    assert_eq!(sp.layout(), [50.0, 50.0]);
    s.act("replace panel 1", || which.set(true));
    s.settle().expect("settled");
    assert_eq!(sp.layout(), [50.0, 50.0], "both panels are still the group's");
}

/// A group nested in a panel that only a cold frame set up never mounted:
/// it is none of the family's groups, which the pointer moving over the
/// family tells of what it reaches.
#[test]
fn splitter_a_nested_group_a_cold_frame_alone_set_up_is_not_the_familys() {
    let held: Held<Splitter<Bind<Vec<f64>>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(Vec::<f64>::new()), move |cx| {
            let sp = Splitter::new(
                use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
                SplitterOptions::default,
                Direction::Ltr,
            );
            h.set(Some(sp));
            let focus = FocusHandle::new();
            let panel = move |i: usize| {
                component(i, move |cx| {
                    let i = cx.props().with_untracked(|i| *i);
                    sp.register(i, Constraints::default());
                    move || {
                        // In the panel, a group only the cold frame sets up.
                        let ghost = gui::component::is_cold_render().then(|| {
                            component(Bind::default_value(Vec::<f64>::new()), |cx| {
                                let _nested = Splitter::new(
                                    use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
                                    SplitterOptions::default,
                                    Direction::Ltr,
                                );
                                || Element::Empty
                            })
                            .into_element()
                        });
                        sp.panel(div().named("panel").flex(Flex::grow(1.0)), i)
                            .children(ghost)
                            .into_element()
                    }
                })
                .into_element()
            };
            move || {
                sp.group(div().named("group").display(Display::Flex).width(602.0).height(50.0))
                    .child(panel(0))
                    .child(sp.handle(div().width(2.0).focusable(), 0, Availability::Enabled, focus))
                    .child(panel(1))
                    .into_element()
            }
        })
        .into_element()
    });
    s.check_cold(true);
    for name in ["one", "two", "three"] {
        s.frame(name).expect("a checked frame");
    }
    let b = bounds(&s, "group").expect("the group");
    s.hover(b.x + 301.0, b.y + 25.0);
    s.settle().expect("settled");
    s.hover(b.x + 100.0, b.y + 25.0);
    s.settle().expect("settled");
    let sp = held.get().expect("made");
    assert_eq!(sp.layout(), [50.0, 50.0], "the group is as it was");
}

/// A handle whose panel never mounts names no panel it controls, and the
/// group keeps no node for that panel: the node is the registered panel's
/// own, so a handle rendered after an index never registered (here a
/// thousand of them, one after another) leaves nothing behind.
#[test]
fn a_splitter_handle_whose_panel_never_mounts_leaves_no_entry() {
    let at = Signal::new(1_usize);
    let held: Held<Splitter<Bind<Vec<f64>>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(vec![100.0]), move |cx| {
            let sp = Splitter::new(
                use_model(cx.props(), |b: &Bind<Vec<f64>>| b),
                SplitterOptions::default,
                Direction::Ltr,
            );
            sp.register(0, Constraints::default());
            let focus = FocusHandle::new();
            h.set(Some(sp));
            move || {
                sp.group(div().display(Display::Flex).width(300.0).height(50.0))
                    .child(sp.panel(div().named("p0"), 0))
                    .child(sp.handle(
                        div().named("handle").width(2.0).focusable(),
                        at.get(),
                        Availability::Enabled,
                        focus,
                    ))
                    .into_element()
            }
        })
        .into_element()
    });
    let sp = held.get().expect("the splitter");
    for i in 1..=1000 {
        s.act("the handle moves past a panel never mounted", || at.set(i));
        let tree = a11y(&s);
        assert!(
            !tree.contains("controls="),
            "round {i}: the handle controls nothing:\n{tree}"
        );
    }
    assert_eq!(sp.kept_panel_nodes(), 1, "only the mounted panel's node");
}

#[test]
fn a_tab_panel_whose_tab_never_mounts_leaves_nothing_behind() {
    let round = Signal::new(0_u32);
    let held: Held<Tabs<Bind<u32>, u32>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(0_u32), move |cx| {
            let tabs = Tabs::new(
                use_model(cx.props(), |b: &Bind<u32>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            let focus = [FocusHandle::new(), FocusHandle::new()];
            h.set(Some(tabs));
            move || {
                // A tab whose value comes and goes now and then, and a
                // panel whose tab never mounts, every round.
                let n = round.get();
                div()
                    .child(
                        tabs.list(div().display(Display::Flex))
                            .child(tabs.trigger(label("tab", "One"), 0, Availability::Enabled, focus[0]))
                            .child(tabs.trigger(label("tab", "Two"), 1 + n / 100 % 2, Availability::Enabled, focus[1])),
                    )
                    .child(tabs.panel(label("panel", "Panel one"), &0))
                    .child(tabs.panel(label("panel", "Loose"), &(1_000 + n)))
                    .into_element()
            }
        })
        .into_element()
    });
    let tabs = held.get().expect("the tabs");
    let mut most = 0;
    for n in 1..=1000 {
        s.act("values come and go", || round.set(n));
        most = most.max(tabs.kept_tabs());
    }
    // Each value held by the parts of the last two renders (what may be on
    // screen): the two tabs' and the panels', bounded however many come.
    assert!(most <= 5, "kept {most} values, 2 tabs and 2 panels shown");
    let tree = a11y(&s);
    assert!(
        tree.contains("Tab \"One\" selected controls=TabPanel") && tree.contains("TabPanel labelled-by=One"),
        "the tab still controls its own panel:\n{tree}"
    );
}

/// A panel of its own instance, rendered once: what names it is what it
/// was given as it rendered.
fn own_panel(tabs: Tabs<Bind<u32>, u32>) -> Element {
    component((), move |_| {
        move || tabs.panel(label("panel", "Panel one"), &0).into_element()
    })
    .into_element()
}

/// Panel one stays mounted while its tab leaves for two renders (so no
/// render that drew the tab is kept) and comes back: the pair they share
/// lives while either does, so the tab controls the panel again and the
/// panel is named by it.
#[test]
fn a_tab_that_leaves_for_a_layout_finds_its_panel_again() {
    let shown = Signal::new(true);
    let tick = Signal::new(0_u32);
    let mut s = scenario(move || {
        component(Bind::default_value(0_u32), move |cx| {
            let tabs = Tabs::new(
                use_model(cx.props(), |b: &Bind<u32>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            let focus = FocusHandle::new();
            move || {
                tick.get();
                let tab = shown
                    .get()
                    .then(|| tabs.trigger(label("tab", "One"), 0, Availability::Enabled, focus));
                div()
                    .child(tabs.list(div().display(Display::Flex)).children(tab))
                    .child(own_panel(tabs))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    s.act("the tab leaves", || shown.set(false));
    s.settle().expect("settled");
    s.act("a render without it again", || tick.set(1));
    s.settle().expect("settled");
    s.act("the tab comes back", || shown.set(true));
    s.settle().expect("settled");
    let tree = a11y(&s);
    assert!(
        tree.contains("Tab \"One\" selected controls=TabPanel") && tree.contains("TabPanel labelled-by=One"),
        "the tab controls its panel again, which it names:\n{tree}"
    );
}

/// The comparisons of [`Counted`] values made so far (one test uses them).
static COMPARED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A tab value that counts its comparisons.
#[derive(Clone, Copy, Debug, Eq)]
struct Counted(usize);

impl PartialEq for Counted {
    fn eq(&self, other: &Self) -> bool {
        COMPARED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.0 == other.0
    }
}

impl std::hash::Hash for Counted {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.0.hash(h);
    }
}

/// A thousand tabs, then a hundred: what the tabs keep of the values that
/// left is let go in work linear in the tabs, not their square.
#[test]
fn tabs_let_go_of_a_thousand_values_in_linear_work() {
    let count = Signal::new(1000_usize);
    let held: Held<Tabs<Bind<Counted>, Counted>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(Counted(0)), move |cx| {
            let tabs = Tabs::new(
                use_model(cx.props(), |b: &Bind<Counted>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            h.set(Some(tabs));
            let focus: Vec<FocusHandle> = (0..1000).map(|_| FocusHandle::new()).collect();
            move || {
                let n = count.get();
                tabs.list(div().display(Display::Flex))
                    .children(focus.iter().take(n).enumerate().map(|(i, f)| {
                        tabs.trigger(div().width(1.0).height(1.0), Counted(i), Availability::Enabled, *f)
                    }))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let tabs = held.get().expect("the tabs");
    COMPARED.store(0, std::sync::atomic::Ordering::Relaxed);
    s.act("most tabs leave", || count.set(100));
    s.settle().expect("settled");
    s.act("again", || count.set(99));
    s.settle().expect("settled");
    let compared = COMPARED.load(std::sync::atomic::Ordering::Relaxed);
    assert!(compared <= 20 * 1000, "{compared} comparisons for 1000 tabs");
    assert!(tabs.kept_tabs() <= 100, "kept {}", tabs.kept_tabs());
}

/// A slide of its own instance, so its key is made at its mount.
fn churned_slide(c: Carousel<Bind<usize>>, n: usize) -> Element {
    component((), move |_| {
        let key = c.slide_key();
        move || {
            c.slide(
                div()
                    .named("slide")
                    .width(300.0)
                    .height(20.0 + f32::from(u8::try_from(n).unwrap_or(0)))
                    .flex_shrink(0.0)
                    .child(text(format!("Slide {}", n + 1))),
                key,
            )
            .into_element()
        }
    })
    .into_element()
}

#[test]
fn carousel_slides_that_come_and_go_leave_nothing_behind() {
    let count = Signal::new(4_usize);
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(0_usize), move |cx| {
            let options = CarouselOptions {
                auto_height: true,
                ..CarouselOptions::default()
            };
            let c = Carousel::new(
                use_model(cx.props(), |b: &Bind<usize>| b),
                move || options,
                None,
                Direction::Ltr,
            );
            h.set(Some(c));
            move || {
                let track = div()
                    .named("track")
                    .display(Display::Flex)
                    .translate_x(move || c.offset())
                    .children((0..count.get()).map(|n| churned_slide(c, n)));
                c.root(div().named("root").width(300.0))
                    .child(
                        c.content(div().named("content").overflow(Overflow::Hidden))
                            .child(track),
                    )
                    .child(indicator_group(c))
                    .into_element()
            }
        })
        .into_element()
    });
    let c = held.get().expect("the carousel");
    let mut most = 0;
    let mut before = 4;
    for round in 1..=1000_u32 {
        let n = if round.is_multiple_of(2) { 4 } else { 1 };
        s.act("slides come and go", || count.set(n));
        s.settle().expect("settled");
        // A snap's nodes are held by the renders whose output may be on
        // screen: this one's and the one before.
        assert!(
            c.kept_slides() <= n.max(before),
            "round {round}: kept {}, {n} shown, {before} before",
            c.kept_slides()
        );
        before = n;
        most = most.max(c.kept_slides());
    }
    assert_eq!(most, 4, "the most slides shown at once");
    let tree = a11y(&s);
    assert!(
        tree.contains("Tab \"Slide 1\" selected controls=TabPanel") && tree.contains("TabPanel labelled-by=Slide 1"),
        "the first dot still controls its slide, and names it:\n{tree}"
    );
}

/// A thousand slides, then one fewer, then a thousand again: letting go of
/// the heights of the slides that left is work linear in the slides.
#[test]
fn a_carousel_lets_go_of_heights_in_linear_work() {
    let count = Signal::new(1000_usize);
    let held: CarouselHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component(Bind::default_value(0_usize), move |cx| {
            let options = CarouselOptions {
                auto_height: true,
                ..CarouselOptions::default()
            };
            let c = Carousel::new(
                use_model(cx.props(), |b: &Bind<usize>| b),
                move || options,
                None,
                Direction::Ltr,
            );
            h.set(Some(c));
            move || {
                let track = div()
                    .named("track")
                    .display(Display::Flex)
                    .translate_x(move || c.offset())
                    .children((0..count.get()).map(|n| churned_slide(c, n)));
                c.root(div().named("root").width(300.0))
                    .child(
                        c.content(div().named("content").overflow(Overflow::Hidden))
                            .child(track),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let c = held.get().expect("the carousel");
    let before = c.prune_work();
    for n in [999, 1000, 999] {
        s.act("a slide comes or goes", || count.set(n));
        s.settle().expect("settled");
    }
    let work = c.prune_work() - before;
    assert!(work <= 3 * 4 * 1000, "{work} steps for three changes of 1000 slides");
    assert!(c.kept_slides() <= 999, "kept {}", c.kept_slides());
}

/// A child part makes the area a virtualizer, then leaves; the area lays
/// a plain viewport out. The list left with its part: scrolling to the
/// top scrolls the viewport, not the list that is gone.
#[test]
fn a_scroll_area_lets_go_of_the_list_its_leaving_child_made() {
    let virtual_rows = Signal::new(true);
    let held: AreaHeld = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let area = ScrollArea::new(ScrollAreaOptions::default, Direction::Ltr);
            h.set(Some(area));
            move || {
                let root = area.root(div().position(Position::Relative).width(200.0).height(84.0));
                if virtual_rows.get() {
                    root.child(component((), move |_| {
                        let _list = area.virtualizer(VirtualizerOptions::default());
                        move || {
                            area.virtual_viewport(
                                1000,
                                |i| i,
                                |i| div().height(28.0).child(text(format!("row {i}"))).into_element(),
                                |v| v,
                            )
                            .height(84.0)
                            .max_height(84.0)
                            .into_element()
                        }
                    }))
                    .into_element()
                } else {
                    root.child(
                        area.viewport(div().named("viewport").size(pct(100.0)))
                            .child(div().named("body").width(100.0).height(1000.0)),
                    )
                    .into_element()
                }
            }
        })
        .into_element()
    });
    let area = held.get().expect("the area");
    s.act("plain viewport", || virtual_rows.set(false));
    s.settle().expect("settled");
    s.act("down", || area.handle().scroll_to((0.0, 50.0)));
    s.settle().expect("settled");
    assert!(area.handle().offset().1 > 49.0, "the viewport scrolled");
    s.act("top", || area.scroll_top());
    s.settle().expect("settled");
    assert!(
        area.handle().offset().1 < 0.5,
        "the viewport went to the top, not a list that left: {:?}",
        area.handle().offset()
    );
}

/// A part that makes the area a virtualizer, its list kept for the test.
fn list_maker(area: ScrollArea, made: Rc<RefCell<Option<ListState>>>) -> Element {
    component((), move |_| {
        let list = area.virtualizer(VirtualizerOptions::default());
        *made.borrow_mut() = Some(list);
        || Element::Empty
    })
    .into_element()
}

/// The viewport renders before a sibling makes the area a virtualizer:
/// it shows the rows once the list is there; a new part replacing the
/// list, the viewport lays the rows on the new one.
#[test]
fn a_virtual_viewport_follows_the_list_a_sibling_makes_and_replaces() {
    let maker = Signal::new(0_u8);
    let made: Rc<RefCell<Option<ListState>>> = Rc::default();
    let m = Rc::clone(&made);
    let mut s = scenario(move || {
        let m = Rc::clone(&m);
        component((), move |_| {
            let area = ScrollArea::new(ScrollAreaOptions::default, Direction::Ltr);
            let m = Rc::clone(&m);
            move || {
                let viewport = component((), move |_| {
                    move || {
                        area.virtual_viewport(
                            1000,
                            |i| i,
                            |i| div().height(28.0).child(text(format!("row {i}"))).into_element(),
                            |v| v,
                        )
                        .height(84.0)
                        .max_height(84.0)
                        .into_element()
                    }
                });
                // Keyed by which part it is: a new one replaces the list.
                let part = match maker.get() {
                    0 => None,
                    n => Some(div().key(u64::from(n)).child(list_maker(area, Rc::clone(&m)))),
                };
                area.root(div().position(Position::Relative).width(200.0).height(84.0))
                    .child(viewport)
                    .children(part)
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    assert!(s.locate("row 0").is_err(), "no list yet, no rows");
    s.act("a sibling makes the list", || maker.set(1));
    s.settle().expect("settled");
    assert!(s.locate("row 0").is_ok(), "the rows show on the list");
    s.act("another part replaces it", || maker.set(2));
    s.settle().expect("settled");
    let second = made.borrow().clone().expect("the second list");
    s.act("the new list goes down", || second.scroll_to_row(500));
    s.settle().expect("settled");
    assert!(
        s.locate("row 500").is_ok(),
        "the viewport lays its rows on the new list"
    );
}

/// A tab value whose `Hash` writes the same bytes for every value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Twin(u8);

impl std::hash::Hash for Twin {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        0_u8.hash(h);
    }
}

/// Two tabs whose values hash alike but are not equal name nodes of their
/// own: the panel shown is named by its own tab, and the other tab
/// controls nothing.
#[test]
fn tabs_whose_values_hash_alike_name_nodes_of_their_own() {
    let mut s = scenario(move || {
        component(Bind::default_value(Twin(1)), move |cx| {
            let tabs = Tabs::new(
                use_model(cx.props(), |b: &Bind<Twin>| b),
                TabsOptions::default,
                Direction::Ltr,
            );
            let focus = [FocusHandle::new(), FocusHandle::new()];
            move || {
                div()
                    .child(
                        tabs.list(div().display(Display::Flex))
                            .child(tabs.trigger(label("tab", "One"), Twin(1), Availability::Enabled, focus[0]))
                            .child(tabs.trigger(label("tab", "Two"), Twin(2), Availability::Enabled, focus[1])),
                    )
                    .child(tabs.panel(label("panel", "Panel one"), &Twin(1)))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let tree = a11y(&s);
    assert!(
        tree.contains("Tab \"One\" selected controls=TabPanel") && tree.contains("TabPanel labelled-by=One\n"),
        "the panel is its own tab's:\n{tree}"
    );
    assert!(
        !tree.contains("Tab \"Two\" unselected controls="),
        "the other tab controls nothing:\n{tree}"
    );
}
