//! Group 5, select and lists, in a window by behaviour: each primitive's
//! keyboard table, its timings (typeahead's 1000 ms, the select's 10 px
//! press and 50 ms scroll step), dismissal and focus, its value controlled
//! and uncontrolled, what cannot be used, and what a screen reader is told
//! (research §9, ADR-0025 "Group 5").

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

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::list::virtual_list_in;
use gui::prelude::*;
use gui::{
    Bind, Direction, Element, FocusHandle, Positioning, Side, SideAlign, Signal, TextField, component, deferred, div,
    text, text_input, use_model,
};
use primitives::Availability;
use primitives::autocomplete::Autocomplete;
use primitives::collection::{Item, Items};
use primitives::combobox::{Combobox, ComboboxConfig};
use primitives::command::{Command, CommandItem, CommandSection, Row as CommandRow, options_of};
use primitives::filter::Filter;
use primitives::listbox::{Bound, Held as Keeps, ListConfig, ListCore, ListKey, Options};
use primitives::select::{PRESS_SLOP, SCROLL_EVERY, Select};
use primitives::tags_input::{TagsConfig, TagsInput};
use primitives::tree::{Rows, Tree, TreeConfig, TreeData};
use primitives::typeahead::RESET;

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    // Every frame is checked against a cold one (a render is pure),
    // with `GUI_COLD_CHECK` set or not, so the lists' virtual windows
    // cannot regress unseen.
    s.check_cold(true);
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

fn control(s: &mut Scenario, k: NamedKey) {
    key(
        s,
        k,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
}

/// Control + A.
fn select_all(s: &mut Scenario) {
    s.send(gui::window::Input::Key(gui::KeyEvent {
        key: gui::Key::Character("a".into()),
        text: Some("a".to_owned()),
        modifiers: Modifiers {
            control: true,
            ..Modifiers::default()
        },
    }));
    s.settle().expect("settled");
}

fn typed(s: &mut Scenario, t: &str) {
    s.type_text(t);
    s.settle().expect("settled");
}

/// The centre of the `nth` visible node named `name`.
fn spot(s: &Scenario, name: &str, nth: usize) -> (f32, f32) {
    let b = s
        .last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name) && !n.look.invisible)
        .nth(nth)
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .expect("the node on screen");
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

/// How far the first option is above the top of its viewport.
fn scrolled(s: &Scenario) -> f32 {
    let top = |name: &str| {
        s.last()
            .nodes
            .iter()
            .find(|n| n.name == Some(name))
            .map(|n| n.bounds(gui::inspect::OUTSIDE).y)
            .expect("the node")
    };
    top("viewport") - top("option")
}

fn visible(s: &Scenario, name: &str) -> usize {
    s.last()
        .nodes
        .iter()
        .filter(|n| n.name == Some(name) && !n.look.invisible)
        .count()
}

/// Whether `h` holds focus in `s`'s window, read in a turn of it (outside
/// any turn, focus is the thread window's: ADR-0015).
fn focused(s: &mut Scenario, h: FocusHandle) -> bool {
    s.read(gui::focus::focused) == Some(h.id())
}

/// The accessibility tree of `s`'s last frame, with its window's focus
/// (read in a turn of it: ADR-0015).
fn access_tree(s: &mut Scenario, ts: &mut gui::text::TextSystem) -> accesskit::TreeUpdate {
    let focus = s.read(gui::focus::focused);
    gui::access::tree(s.last(), focus, 1.0, "w", ts)
}

fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The tree from the window down, a line per node: role, name, the states a
/// list tells and relations by their targets' names.
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
        // A value beside a name: a select's choice.
        if let (Some(_), Some(v)) = (n.label(), n.value()) {
            line.push_str(&format!(" value={v:?}"));
        }
        if let Some(d) = n.description() {
            line.push_str(&format!(" description={d:?}"));
        }
        if let Some(level) = n.level() {
            line.push_str(&format!(" level={}", level + 1));
        }
        if let (Some(at), Some(of)) = (n.position_in_set(), n.size_of_set()) {
            line.push_str(&format!(" {at}/{of}"));
        }
        if let Some(expanded) = n.is_expanded() {
            line.push_str(&format!(" expanded={expanded}"));
        }
        if let Some(selected) = n.is_selected() {
            line.push_str(&format!(" selected={selected}"));
        }
        if let Some(t) = n.toggled() {
            line.push_str(&format!(" toggled={t:?}"));
        }
        if n.is_multiselectable() {
            line.push_str(" multiselectable");
        }
        if n.is_required() {
            line.push_str(" required");
        }
        if let Some(p) = n.has_popup() {
            line.push_str(&format!(" popup={p:?}"));
        }
        if let Some(a) = n.auto_complete() {
            line.push_str(&format!(" autocomplete={a:?}"));
        }
        if let Some(c) = n.aria_current() {
            line.push_str(&format!(" current={c:?}"));
        }
        if n.is_disabled() {
            line.push_str(" disabled");
        }
        if n.is_modal() {
            line.push_str(" modal");
        }
        if let Some(a) = n.active_descendant() {
            line.push_str(&format!(" active={}", named(&a)));
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

/// The name of the node assistive technology is told has focus (its own,
/// else its first named child's).
fn focused_label(s: &mut Scenario) -> Option<String> {
    let mut ts = gui::text::TextSystem::new();
    let update = access_tree(s, &mut ts);
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let n = nodes.get(&update.focus)?;
    name(n).or_else(|| n.children().iter().find_map(|c| nodes.get(c).and_then(|c| name(c))))
}

fn a11y(s: &mut Scenario) -> String {
    let mut ts = gui::text::TextSystem::new();
    dump(&access_tree(s, &mut ts))
}

// ---------------------------------------------------------------- Select

/// Value, label, disabled.
type Fruit = (&'static str, &'static str, bool);

const FRUITS: [Fruit; 5] = [
    ("apple", "Apple", false),
    ("banana", "Banana", false),
    ("blueberry", "Blueberry", true),
    ("cherry", "Cherry", false),
    ("grapes", "Grapes", false),
];

#[derive(Clone, PartialEq)]
struct SelectProps<M: Clone + PartialEq + 'static> {
    value: Bind<M>,
    open: Bind<bool>,
    /// The viewport's height, to make it scroll.
    height: Option<f32>,
}

gui::engine_local! {
    [Fork] static ONE: Cell<Option<Select<String, Option<String>>>> = Cell::new(None);
    [Fork] static MANY: Cell<Option<Select<String, Vec<String>>>> = Cell::new(None);
    /// While it holds, the banana option is `invisible`.
    [Fork] static HIDE_BANANA: Cell<Option<Signal<bool>>> = Cell::new(None);
}

trait Stash: primitives::listbox::Choice<String> {
    fn stash(s: Select<String, Self>);
}

impl Stash for Option<String> {
    fn stash(s: Select<String, Self>) {
        ONE.set(Some(s));
    }
}

impl Stash for Vec<String> {
    fn stash(s: Select<String, Self>) {
        MANY.set(Some(s));
    }
}

/// A trigger at the top left, its list below it, the fruits in it.
fn select_ui<M: Stash>(props: SelectProps<M>) -> Element {
    component(props, |cx| {
        let props = cx.props();
        let value = use_model(props, |p: &SelectProps<M>| &p.value);
        let open = use_model(props, |p: &SelectProps<M>| &p.open);
        let select: Select<String, M> = Select::new(Bound::new(value), Bound::new(open), Direction::Ltr);
        M::stash(select);
        select.set_required(true);
        let focus: Vec<FocusHandle> = FRUITS.iter().map(|_| FocusHandle::new()).collect();
        select.refresh();
        move || {
            let open = select.is_open();
            let height = props.with(|p| p.height);
            let hide = HIDE_BANANA.get();
            let options = FRUITS.iter().zip(&focus).map(|((v, l, disabled), f)| {
                let option = div().named("option").height(30.0).width(200.0).child(text(*l));
                let option = match hide {
                    Some(h) if *v == "banana" => option.invisible(move || h.get()),
                    _ => option,
                };
                select.item(
                    option,
                    (*v).to_owned(),
                    Rc::from(*l),
                    *f,
                    Availability::from_disabled(*disabled),
                )
            });
            let (up, down) = select.can_scroll();
            let viewport = select.viewport(
                div()
                    .named("viewport")
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .overflow_y(Overflow::Scroll)
                    .flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .children(options),
                false,
            );
            let content = select.content(
                div()
                    .named("list")
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .width(200.0)
                    .when_some(height, |d, h| d.height(h))
                    .child(up.then(|| select.scroll_button(div().named("up").height(10.0), true)))
                    .child(viewport)
                    .child(down.then(|| select.scroll_button(div().named("down").height(10.0), false))),
                open,
            );
            let content: Element = if open {
                deferred(
                    Positioning::Anchored {
                        side: Side::Bottom,
                        align: SideAlign::Start,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    },
                    content,
                )
                .into_element()
            } else {
                content.into_element()
            };
            let shown = text(select.shown().unwrap_or_else(|| "Pick a fruit".to_owned()));
            select
                .trigger(
                    div()
                        .named("trigger")
                        .aria_label("Fruit")
                        .width(200.0)
                        .height(30.0)
                        .child(shown)
                        .child(content),
                    open,
                )
                .into_element()
        }
    })
    .into_element()
}

fn one() -> Select<String, Option<String>> {
    ONE.get().expect("the select")
}

fn many() -> Select<String, Vec<String>> {
    MANY.get().expect("the select")
}

fn select_scenario(value: Signal<Option<String>>, open: Signal<bool>) -> Scenario {
    scenario(move || {
        select_ui(SelectProps {
            value: Bind::default().with_model(value),
            open: Bind::default().with_model(open),
            height: None,
        })
    })
}

fn focused_option(s: &mut Scenario) -> Option<String> {
    // The node assistive technology is told has focus, when it is an
    // option: focus, not the list's highlight.
    let mut ts = gui::text::TextSystem::new();
    let update = access_tree(s, &mut ts);
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let n = nodes.get(&update.focus)?;
    if n.role() != gui::Role::ListBoxOption {
        return None;
    }
    let label = focused_label(s)?;
    FRUITS
        .iter()
        .find(|(_, l, _)| *l == label)
        .map(|(v, ..)| (*v).to_owned())
}

trait Highlighted {
    fn list_highlight(self) -> Option<String>;
}

impl<M: primitives::listbox::Choice<String>> Highlighted for Select<String, M> {
    fn list_highlight(self) -> Option<String> {
        FRUITS
            .iter()
            .map(|(v, ..)| (*v).to_owned())
            .find(|v| gui::untracked(|| self.is_highlighted(v)))
    }
}

/// Codex (F1 review 1): a select owns its scroll through its scope, with
/// no cycle through the scroll's layout hook: an opened select (its list
/// scrolling, so the hook runs) mounted and unmounted again and again
/// leaves no scroll handle and no parts behind.
#[test]
fn an_opened_select_mounted_and_unmounted_leaves_nothing_behind() {
    let value = Signal::new(None);
    let open = Signal::new(true);
    let mounted = Signal::new(false);
    let mut s = scenario(move || {
        if mounted.get() {
            select_ui(SelectProps {
                value: Bind::default().with_model(value),
                open: Bind::default().with_model(open),
                height: Some(60.0),
            })
        } else {
            div().into_element()
        }
    });
    let baseline = gui::ScrollHandle::live();
    for _ in 0..5 {
        open.set(true);
        mounted.set(true);
        s.settle().expect("settled");
        assert!(one().can_scroll().1, "its list scrolls");
        let parts = one().parts_held();
        assert_eq!(gui::ScrollHandle::live(), baseline + 1, "one scroll, the select's");
        mounted.set(false);
        s.settle().expect("settled");
        assert_eq!(parts.strong_count(), 0, "the parts went with the select");
        assert_eq!(gui::ScrollHandle::live(), baseline, "and its scroll");
    }
}

/// Coordinator (F1 review 1): a render in a fork that asks for the other
/// alignment writes nothing live. Item-aligned, the list is placed again
/// (its scroll started afresh) as its up button first comes; this one
/// floats, so a wheel that brings the up button keeps it scrolled, even
/// after a fork rendered its viewport item-aligned.
#[test]
#[cfg(debug_assertions)] // Forks are built in debug builds only (ADR-0028, F2).
fn a_fork_rendering_the_other_alignment_leaves_the_live_select_as_it_was() {
    let open = Signal::new(false);
    let mut s = scenario(move || {
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default(),
            open: Bind::default().with_model(open),
            height: Some(80.0),
        })
    });
    s.act("open", || one().open(false));
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert_eq!(gui::untracked(|| one().can_scroll()), (false, true));
    gui::testing::in_check(|| {
        drop(one().viewport(div(), true));
    });
    let down = spot(&s, "down", 0);
    s.hover(down.0, down.1);
    s.settle().expect("settled");
    assert!(scrolled(&s) > 0.5, "a step down, not placed again from the top");
    assert!(gui::untracked(|| one().can_scroll()).0, "the up button came");
}

#[test]
fn a_closed_select_knows_its_options_and_shows_the_chosen_label() {
    let value = Signal::new(Some("cherry".to_owned()));
    let open = Signal::new(false);
    let s = select_scenario(value, open);
    let select = one();
    assert_eq!(gui::untracked(|| select.options().len()), 5, "known while closed");
    assert_eq!(gui::untracked(|| select.shown()).as_deref(), Some("Cherry"));
    assert_eq!(visible(&s, "option"), 0, "closed, nothing shows");
    // A value no option has shows the placeholder, yet is not empty.
    let mut s = s;
    s.act("unknown", || value.set(Some("kiwi".to_owned())));
    s.settle().expect("settled");
    assert_eq!(gui::untracked(|| select.shown()), None);
    assert!(!gui::untracked(|| select.is_empty()));
}

#[test]
fn the_select_keyboard_table() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    // ArrowDown opens on the anchor: nothing chosen, the first usable one.
    plain(&mut s, NamedKey::ArrowDown);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert_eq!(focused_option(&mut s).as_deref(), Some("apple"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(focused_option(&mut s).as_deref(), Some("banana"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        focused_option(&mut s).as_deref(),
        Some("cherry"),
        "the disabled one is skipped"
    );
    plain(&mut s, NamedKey::End);
    assert_eq!(focused_option(&mut s).as_deref(), Some("grapes"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(focused_option(&mut s).as_deref(), Some("grapes"), "stops at the end");
    plain(&mut s, NamedKey::Home);
    assert_eq!(focused_option(&mut s).as_deref(), Some("apple"));
    // Tab stays in the list.
    plain(&mut s, NamedKey::Tab);
    assert!(open.get_untracked());
    assert_eq!(focused_option(&mut s).as_deref(), Some("apple"));
    // Typeahead in the open list.
    typed(&mut s, "c");
    assert_eq!(focused_option(&mut s).as_deref(), Some("cherry"));
    // Enter chooses, closes and focuses the trigger.
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"));
    assert!(!open.get_untracked());
    assert!(focused(&mut s, select.trigger_focus()));
    // Opening again focuses the chosen one.
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    assert_eq!(focused_option(&mut s).as_deref(), Some("cherry"));
    // Escape closes and focuses the trigger, the value kept.
    plain(&mut s, NamedKey::Escape);
    assert!(!open.get_untracked());
    assert!(focused(&mut s, select.trigger_focus()));
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"));
}

/// An option hidden on its own (here `invisible`) is no option of the
/// list, closed or open: typeahead and the arrows pass it, and hidden
/// while highlighted, Enter does not choose it.
#[test]
fn a_select_never_reaches_an_option_hidden_on_its_own() {
    let hide = Signal::new(true);
    HIDE_BANANA.set(Some(hide));
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    typed(&mut s, "b");
    assert_eq!(
        value.get_untracked(),
        None,
        "closed, typeahead passes the hidden banana"
    );
    plain(&mut s, NamedKey::ArrowDown);
    assert!(open.get_untracked());
    assert_eq!(focused_option(&mut s).as_deref(), Some("apple"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        focused_option(&mut s).as_deref(),
        Some("cherry"),
        "the hidden banana is passed"
    );
    // Shown, highlighted, then hidden: Enter does not choose it.
    s.act("show", || hide.set(false));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Home);
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(select.list_highlight().as_deref(), Some("banana"));
    s.act("hide", || hide.set(true));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Enter);
    assert_ne!(
        value.get_untracked().as_deref(),
        Some("banana"),
        "a hidden option is not chosen"
    );
    HIDE_BANANA.set(None);
}

#[test]
fn a_select_opened_by_its_caller_focuses_its_anchor_option() {
    // SelectContentImpl.vue:139-146: the anchor option takes focus once
    // placed, whoever opened the list.
    let value = Signal::new(Some("cherry".to_owned()));
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert_eq!(focused_option(&mut s).as_deref(), Some("cherry"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(focused_option(&mut s).as_deref(), Some("grapes"));
    // Open from the start (shadcn's default_open).
    let value = Signal::new(Some("banana".to_owned()));
    let open = Signal::new(true);
    let mut s = select_scenario(value, open);
    s.settle().expect("settled");
    assert_eq!(focused_option(&mut s).as_deref(), Some("banana"));
}

#[test]
fn typing_on_a_closed_select_chooses_and_forgets_after_1000_ms() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    typed(&mut s, "b");
    assert_eq!(
        value.get_untracked().as_deref(),
        Some("banana"),
        "chosen without opening"
    );
    assert!(!open.get_untracked());
    // "bl" reaches only the disabled blueberry: nothing else is chosen.
    typed(&mut s, "l");
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    // Space mid-search is a character, neither opening nor choosing.
    typed(&mut s, " ");
    assert!(!open.get_untracked(), "space while searching does not open");
    s.wait(RESET + Duration::from_millis(10));
    s.settle().expect("settled");
    assert_eq!(gui::untracked(|| select.search()), "", "the search is forgotten");
    // A new search starts from the chosen one: c reaches cherry.
    typed(&mut s, "c");
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"));
    // Space with no search opens.
    s.wait(RESET + Duration::from_millis(10));
    plain(&mut s, NamedKey::Space);
    assert!(open.get_untracked());
}

#[test]
fn a_press_opens_and_a_release_past_10_px_chooses() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    // A press and release in place: opens, chooses nothing.
    let at = spot(&s, "trigger", 0);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert_eq!(value.get_untracked(), None);
    plain(&mut s, NamedKey::Escape);
    assert!(!open.get_untracked());
    // Pressed and dragged within the slop: chooses nothing.
    s.drag(at, (at.0 + PRESS_SLOP - 1.0, at.1 + PRESS_SLOP - 1.0));
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert_eq!(value.get_untracked(), None);
    plain(&mut s, NamedKey::Escape);
    // Pressed on the trigger and released on Banana: chosen.
    s.send(gui::window::Input::PointerMoved { x: at.0, y: at.1 });
    s.send(gui::window::Input::MouseDown { x: at.0, y: at.1 });
    s.settle().expect("settled");
    let banana = spot(&s, "option", 1);
    s.send(gui::window::Input::PointerMoved {
        x: banana.0,
        y: banana.1,
    });
    s.send(gui::window::Input::MouseUp {
        x: banana.0,
        y: banana.1,
    });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert!(!open.get_untracked(), "one value closes");
    // A click on an option chooses it; a disabled one does nothing.
    s.click(at.0, at.1);
    s.settle().expect("settled");
    let blueberry = spot(&s, "option", 2);
    s.click(blueberry.0, blueberry.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert!(open.get_untracked());
    let cherry = spot(&s, "option", 3);
    s.click(cherry.0, cherry.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"));
}

#[test]
fn a_scroll_button_steps_an_item_every_50_ms() {
    let open = Signal::new(false);
    let mut s = scenario(move || {
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default(),
            open: Bind::default().with_model(open),
            height: Some(80.0),
        })
    });
    s.act("open", || one().open(false));
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert_eq!(gui::untracked(|| one().can_scroll()), (false, true));
    let first = scrolled(&s);
    let down = spot(&s, "down", 0);
    s.hover(down.0, down.1);
    s.settle().expect("settled");
    let once = scrolled(&s);
    assert!(once > first, "a step at once: {first} → {once}");
    s.wait(SCROLL_EVERY - Duration::from_millis(10));
    s.settle().expect("settled");
    assert!((scrolled(&s) - once).abs() < 0.5, "no second step before 50 ms");
    // (A millisecond over: the clock counts in `f32` seconds.)
    s.wait(Duration::from_millis(11));
    s.settle().expect("settled");
    let twice = scrolled(&s);
    assert!(twice > once, "another at 50 ms: {once} → {twice}");
    // Leaving it stops.
    s.hover(700.0, 500.0);
    s.settle().expect("settled");
    let left = scrolled(&s);
    s.wait(SCROLL_EVERY * 3);
    s.settle().expect("settled");
    assert!((scrolled(&s) - left).abs() < 0.5);
}

#[test]
fn a_scroll_button_stops_stepping_once_the_list_cannot_scroll_its_way() {
    let open = Signal::new(false);
    let mut s = scenario(move || {
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default(),
            open: Bind::default().with_model(open),
            height: Some(80.0),
        })
    });
    s.act("open", || one().open(false));
    s.settle().expect("settled");
    s.settle().expect("settled");
    let down = spot(&s, "down", 0);
    s.hover(down.0, down.1);
    s.settle().expect("settled");
    // Five 30 px options in a 60 px viewport: three steps reach the end.
    for _ in 0..6 {
        s.wait(SCROLL_EVERY + Duration::from_millis(1));
        s.settle().expect("settled");
    }
    assert_eq!(gui::untracked(|| one().can_scroll()), (true, false), "at the end");
    assert_eq!(visible(&s, "down"), 0, "the down button went");
    s.wait(SCROLL_EVERY * 2);
    s.settle().expect("settled");
    assert_eq!(
        gui::clock::next_deadline(),
        None,
        "no step timer left waking the UI thread while the list stays open"
    );
    assert!(gui::untracked(|| one().is_open()));
}

/// What [`placed_select`] is given.
#[derive(Clone, PartialEq)]
struct PlacedProps {
    value: Bind<Option<String>>,
    open: Bind<bool>,
    direction: Direction,
    aligned: bool,
    /// Where the trigger sits in the window.
    at: (f32, f32),
}

/// A select whose list floats with reka's popper and its arrow, or lines
/// its anchor option's label up on the trigger's value (`aligned`).
fn placed_select(props: PlacedProps) -> Element {
    component(props, |cx| {
        let props = cx.props();
        let value = use_model(props, |p: &PlacedProps| &p.value);
        let open = use_model(props, |p: &PlacedProps| &p.open);
        let direction = props.with_untracked(|p| p.direction);
        let select: Select<String, Option<String>> = Select::new(Bound::new(value), Bound::new(open), direction);
        ONE.set(Some(select));
        let focus: Vec<FocusHandle> = FRUITS.iter().map(|_| FocusHandle::new()).collect();
        let placed: Signal<Option<gui::Placement>> = Signal::new(None);
        select.refresh();
        move || {
            let (aligned, at) = props.with(|p| (p.aligned, p.at));
            let open = select.is_open();
            let anchor = select.anchor();
            let options = FRUITS.iter().zip(&focus).map(|((v, l, disabled), f)| {
                let label = text(*l).named("label");
                let label = if aligned && anchor.as_deref() == Some(*v) {
                    label.key(gui::key_of("aligned"))
                } else {
                    label
                };
                select.item(
                    div()
                        .named("option")
                        .height(30.0)
                        .width(200.0)
                        .padding_left(8.0)
                        .child(label),
                    (*v).to_owned(),
                    Rc::from(*l),
                    *f,
                    Availability::from_disabled(*disabled),
                )
            });
            let viewport = select.viewport(
                div()
                    .named("viewport")
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .children(options),
                aligned,
            );
            let popper = primitives::select::POPPER.side_offset(4.0).arrow();
            let list = div()
                .named("list")
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .width(200.0)
                .child(viewport);
            let list = if aligned {
                list
            } else {
                list.child(Select::<String, Option<String>>::arrow(
                    div().named("arrow"),
                    &popper,
                    placed.get().as_ref(),
                ))
            };
            let content = select.content(list, open);
            let content: Element = match (open, aligned) {
                (false, _) => content.into_element(),
                (true, false) => deferred(popper.positioning(), content).placement(placed).into_element(),
                (true, true) => deferred(
                    Positioning::Over {
                        part: gui::key_of("aligned"),
                        anchor_part: gui::key_of("value"),
                        direction: select.direction(),
                        min_height: select.aligned_min_height(),
                    },
                    content,
                )
                .into_element(),
            };
            let shown = select.shown().unwrap_or_else(|| "Pick a fruit".to_owned());
            div()
                .padding_left(at.0)
                .padding_top(at.1)
                .child(
                    select.trigger(
                        div()
                            .named("trigger")
                            .width(200.0)
                            .height(30.0)
                            .padding_left(12.0)
                            .padding_right(12.0)
                            .child(text(shown).named("value").key(gui::key_of("value")))
                            .child(content),
                        open,
                    ),
                )
                .into_element()
        }
    })
    .into_element()
}

fn bounds_of(s: &Scenario, name: &str) -> gui::Bounds {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name) && !n.look.invisible)
        .map(|n| n.bounds(gui::inspect::OUTSIDE))
        .expect("the node on screen")
}

fn placed_scenario(direction: Direction, aligned: bool, value: Option<&str>, at: (f32, f32)) -> Scenario {
    let value = Signal::new(value.map(str::to_owned));
    let open = Signal::new(false);
    let mut s = scenario(move || {
        placed_select(PlacedProps {
            value: Bind::default().with_model(value),
            open: Bind::default().with_model(open),
            direction,
            aligned,
            at,
        })
    });
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    s.settle().expect("settled");
    s.settle().expect("settled");
    s
}

/// Coordinator (F1 review 1): mounted floating (A), its parts built
/// item-aligned (B) in a cold frame and rolled back, the list then opens
/// through the real layout where A puts it, as a control that only ran A,
/// in geometry where A and B differ.
#[test]
#[cfg(debug_assertions)] // Forks are built in debug builds only (ADR-0028, F2).
fn a_select_opens_where_its_live_alignment_puts_it_after_a_check_built_the_other() {
    let at = (300.0, 250.0);
    let control = bounds_of(&placed_scenario(Direction::Ltr, false, Some("cherry"), at), "list");
    let other = bounds_of(&placed_scenario(Direction::Ltr, true, Some("cherry"), at), "list");
    assert!(
        (control.y - other.y).abs() > 1.0,
        "A and B differ: {control:?} vs {other:?}"
    );
    let value = Signal::new(Some("cherry".to_owned()));
    let open = Signal::new(false);
    let mut s = scenario(move || {
        placed_select(PlacedProps {
            value: Bind::default().with_model(value),
            open: Bind::default().with_model(open),
            direction: Direction::Ltr,
            aligned: false,
            at,
        })
    });
    gui::testing::in_check(|| {
        let select = one();
        drop(select.viewport(div(), true));
        drop(select.content(div(), true));
    });
    s.act("open", || open.set(true));
    for _ in 0..3 {
        s.settle().expect("settled");
    }
    assert_eq!(bounds_of(&s, "list"), control, "where A puts it");
}

#[test]
fn a_popper_select_draws_reka_arrow_on_the_edge_facing_the_trigger() {
    let s = placed_scenario(Direction::Ltr, false, None, (100.0, 20.0));
    let trigger = bounds_of(&s, "trigger");
    let list = bounds_of(&s, "list");
    let arrow = bounds_of(&s, "arrow");
    assert_eq!((arrow.width, arrow.height), (10.0, 5.0), "reka's 10 × 5 arrow");
    assert!(
        (list.y - (trigger.y + trigger.height + 4.0 + 5.0)).abs() < 0.5,
        "the list leaves the side offset and the arrow's height below the trigger: {list:?}"
    );
    assert!(
        (arrow.y + arrow.height - list.y).abs() < 0.5,
        "on the list's top edge: {arrow:?}"
    );
    let centre = trigger.x + trigger.width / 2.0;
    assert!(
        (arrow.x + arrow.width / 2.0 - centre).abs() < 0.5,
        "pointing at the trigger's centre: {arrow:?} vs {centre}"
    );
}

#[test]
fn an_item_aligned_list_lines_the_chosen_label_up_on_the_value_by_reading_direction() {
    for direction in [Direction::Ltr, Direction::Rtl] {
        let s = placed_scenario(direction, true, Some("cherry"), (300.0, 250.0));
        let value = bounds_of(&s, "value");
        let label = s
            .last()
            .nodes
            .iter()
            .filter(|n| n.name == Some("label") && !n.look.invisible)
            .nth(3)
            .map(|n| n.bounds(gui::inspect::OUTSIDE))
            .expect("cherry's label");
        let trigger = bounds_of(&s, "trigger");
        assert!(
            (label.y + label.height / 2.0 - (trigger.y + trigger.height / 2.0)).abs() < 0.5,
            "{direction:?}: centred on the trigger: {label:?} vs {trigger:?}"
        );
        match direction {
            Direction::Ltr => assert!(
                (label.x - value.x).abs() < 0.5,
                "left to right, the start edges line up: {label:?} vs {value:?}"
            ),
            Direction::Rtl => assert!(
                (label.x + label.width - (value.x + value.width)).abs() < 0.5,
                "right to left, mirrored: the right edges line up: {label:?} vs {value:?}"
            ),
        }
    }
}

#[test]
fn an_item_aligned_list_short_of_room_below_moves_up_to_keep_five_rows() {
    // The trigger near the window's bottom, the first option chosen: lined
    // up on the value, the list would run 120 px past the window.
    let s = placed_scenario(Direction::Ltr, true, Some("apple"), (300.0, 540.0));
    let list = bounds_of(&s, "list");
    let trigger = bounds_of(&s, "trigger");
    assert!(list.height >= 150.0 - 0.5, "five 30 px rows at least: {list:?}");
    assert!(
        list.y + list.height <= 600.0 - 10.0 + 0.5,
        "10 px from the window's edge: {list:?}"
    );
    assert!(list.y < trigger.y, "moved up over the trigger: {list:?}");
}

/// A select (open as `open` says, holding `value`) and, away from it, a
/// node that counts its clicks.
fn select_beside(value: Signal<Option<String>>, open: Signal<bool>, clicks: Rc<Cell<u32>>) -> Scenario {
    scenario(move || {
        let c = Rc::clone(&clicks);
        div()
            .child(select_ui(SelectProps {
                value: Bind::default().with_model(value),
                open: Bind::default().with_model(open),
                height: None,
            }))
            .child(
                div()
                    .named("below")
                    .position(Position::Absolute)
                    .left(500.0)
                    .top(300.0)
                    .size(50.0)
                    .on_click(move |_| c.set(c.get() + 1)),
            )
            .into_element()
    })
}

fn touch(s: &mut Scenario, phase: gui::TouchPhase, at: (f32, f32)) {
    s.send(gui::window::Input::Touch(gui::TouchEvent {
        id: 1,
        kind: gui::PointerKind::Touch,
        phase,
        x: at.0,
        y: at.1,
    }));
    s.settle().expect("settled");
}

#[test]
fn a_press_outside_the_open_select_closes_it_reaching_nothing_below() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = select_beside(value, open, Rc::clone(&clicks));
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert_eq!(focused_option(&mut s).as_deref(), Some("apple"), "focus on an option");
    // The press outside: closed, the node below untouched, focus back on
    // the trigger (not left on an option that is now inert).
    let below = spot(&s, "below", 0);
    s.click(below.0, below.1);
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "a press outside closes it");
    assert_eq!(clicks.get(), 0, "and reaches nothing below (modal)");
    assert!(focused(&mut s, select.trigger_focus()), "focus returns to the trigger");
    assert_eq!(value.get_untracked(), None, "nothing chosen");
    // With the list closed, the node below takes its clicks again.
    s.click(below.0, below.1);
    s.settle().expect("settled");
    assert_eq!(clicks.get(), 1);
}

#[test]
fn a_press_dragged_past_the_slop_and_released_outside_the_list_closes_it() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let clicks = Rc::new(Cell::new(0_u32));
    let mut s = select_beside(value, open, Rc::clone(&clicks));
    let at = spot(&s, "trigger", 0);
    s.send(gui::window::Input::PointerMoved { x: at.0, y: at.1 });
    s.send(gui::window::Input::MouseDown { x: at.0, y: at.1 });
    s.settle().expect("settled");
    assert!(open.get_untracked(), "a press opens at once");
    let below = spot(&s, "below", 0);
    s.send(gui::window::Input::PointerMoved { x: below.0, y: below.1 });
    s.send(gui::window::Input::MouseUp { x: below.0, y: below.1 });
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "released outside the list: closed");
    assert_eq!(value.get_untracked(), None);
    assert_eq!(clicks.get(), 0, "the release reaches nothing below");
    assert!(focused(&mut s, one().trigger_focus()));
}

#[test]
fn a_touch_opens_the_select_as_the_tap_ends_and_chooses_only_by_a_tap() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let at = spot(&s, "trigger", 0);
    touch(&mut s, gui::TouchPhase::Started, at);
    assert!(!open.get_untracked(), "a touch does not open as it starts");
    touch(&mut s, gui::TouchPhase::Ended, at);
    assert!(open.get_untracked(), "it opens as the tap ends");
    assert_eq!(value.get_untracked(), None, "the tap chose nothing");
    // A finger that starts on Cherry and moves (a scroll) chooses nothing,
    // nor does it choose Banana where it ends.
    let cherry = spot(&s, "option", 3);
    let banana = spot(&s, "option", 1);
    touch(&mut s, gui::TouchPhase::Started, cherry);
    touch(&mut s, gui::TouchPhase::Moved, (cherry.0, cherry.1 - 30.0));
    touch(&mut s, gui::TouchPhase::Moved, banana);
    touch(&mut s, gui::TouchPhase::Ended, banana);
    assert_eq!(value.get_untracked(), None, "a moved touch is no tap");
    assert!(open.get_untracked());
    // A tap on Banana chooses it.
    touch(&mut s, gui::TouchPhase::Started, banana);
    touch(&mut s, gui::TouchPhase::Ended, banana);
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert!(!open.get_untracked());
}

#[test]
fn a_select_of_several_has_no_select_all() {
    let value: Signal<Vec<String>> = Signal::new(vec!["grapes".to_owned()]);
    let mut s = scenario(move || {
        select_ui(SelectProps {
            value: Bind::default().with_model(value),
            open: Bind::default(),
            height: None,
        })
    });
    let select = many();
    s.act("focus", || select.trigger_focus().focus());
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    assert!(gui::untracked(|| select.is_open()));
    select_all(&mut s);
    assert_eq!(
        value.get_untracked(),
        ["grapes"],
        "Control + A chooses nothing (reka's Select)"
    );
    shift(&mut s, NamedKey::ArrowUp);
    assert_eq!(value.get_untracked(), ["grapes"], "nor does a Shift range");
}

#[test]
fn a_select_of_several_toggles_and_stays_open() {
    let value: Signal<Vec<String>> = Signal::new(vec!["grapes".to_owned()]);
    let mut s = scenario(move || {
        select_ui(SelectProps {
            value: Bind::default().with_model(value),
            open: Bind::default(),
            height: None,
        })
    });
    let select = many();
    s.act("focus", || select.trigger_focus().focus());
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    assert!(gui::untracked(|| select.is_open()));
    assert_eq!(select.list_highlight().as_deref(), Some("grapes"), "the first chosen");
    plain(&mut s, NamedKey::Home);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), ["grapes", "apple"]);
    assert!(gui::untracked(|| select.is_open()), "stays open");
    plain(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), ["grapes"], "toggled off");
    assert_eq!(gui::untracked(|| select.shown()).as_deref(), Some("Grapes"));
}

#[test]
fn an_uncontrolled_disabled_select_does_not_open() {
    let mut s = scenario(move || {
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default_value(Some("apple".to_owned())),
            open: Bind::default(),
            height: None,
        })
    });
    let select = one();
    assert_eq!(gui::untracked(|| select.shown()).as_deref(), Some("Apple"));
    s.act("disable", || select.set_availability(Availability::Disabled));
    s.settle().expect("settled");
    let at = spot(&s, "trigger", 0);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert!(!gui::untracked(|| select.is_open()));
    s.act("enable", || select.set_availability(Availability::Enabled));
    s.settle().expect("settled");
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert!(gui::untracked(|| select.is_open()));
    assert_eq!(select.list_highlight().as_deref(), Some("apple"));
}

/// A select whose caller keeps `open` as a plain value, written back by
/// `on_change` when `accept` says so.
fn value_open_scenario(open: Signal<bool>, accept: Rc<Cell<bool>>) -> Scenario {
    scenario(move || {
        let accept = Rc::clone(&accept);
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default_value(Some("banana".to_owned())),
            open: Bind::value(open.get()).on_change(move |v| {
                if accept.get() {
                    open.set(v);
                }
            }),
            height: None,
        })
    })
}

#[test]
fn a_value_controlled_select_opened_by_keyboard_shows_focus_as_keyboard_focus() {
    let open = Signal::new(false);
    let mut s = value_open_scenario(open, Rc::new(Cell::new(true)));
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    // The pointer moved last; the opener says the keyboard opens it (a
    // shortcut's handler), and the caller takes the open a turn later.
    s.hover(700.0, 500.0);
    s.settle().expect("settled");
    s.act("open by keyboard", || select.open(true));
    s.settle().expect("settled");
    assert!(open.get_untracked(), "the caller took the open");
    assert_eq!(focused_option(&mut s).as_deref(), Some("banana"));
    assert!(
        s.read(gui::focus::focus_visible),
        "the opener's keyboard modality reaches the anchor option"
    );
}

#[test]
fn a_refused_keyboard_select_open_leaves_no_modality_for_the_caller_s_own_open() {
    let open = Signal::new(false);
    let accept = Rc::new(Cell::new(false));
    let mut s = value_open_scenario(open, Rc::clone(&accept));
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    s.hover(700.0, 500.0);
    s.settle().expect("settled");
    s.act("open by keyboard", || select.open(true));
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "the caller refused");
    // The caller opens it itself later: not from the keyboard.
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert_eq!(focused_option(&mut s).as_deref(), Some("banana"));
    assert!(!s.read(gui::focus::focus_visible), "a refused open's modality is gone");
}

#[test]
fn opening_an_open_select_keeps_its_scroll() {
    let open = Signal::new(false);
    let mut s = scenario(move || {
        select_ui::<Option<String>>(SelectProps {
            value: Bind::default(),
            open: Bind::default().with_model(open),
            height: Some(80.0),
        })
    });
    s.act("open", || one().open(false));
    s.settle().expect("settled");
    s.settle().expect("settled");
    let down = spot(&s, "down", 0);
    s.hover(down.0, down.1);
    s.settle().expect("settled");
    s.hover(700.0, 500.0);
    s.settle().expect("settled");
    let before = scrolled(&s);
    assert!(before > 0.5, "scrolled down a step: {before}");
    // Asked to open while open: nothing to do, the list stays where it is
    // and its scroll handle stays the one its viewport tracks.
    s.act("open again", || one().open(true));
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert!(
        (scrolled(&s) - before).abs() < 0.5,
        "kept at {before}, now {}",
        scrolled(&s)
    );
    assert_eq!(gui::untracked(|| one().can_scroll()), (true, true));
    let down = spot(&s, "down", 0);
    s.hover(down.0, down.1);
    s.settle().expect("settled");
    assert!(scrolled(&s) > before + 0.5, "the scroll button still scrolls the list");
}

#[test]
fn select_a11y() {
    let value = Signal::new(Some("banana".to_owned()));
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    // Closed, the trigger is read as Chrome reads a `<select>`: named by
    // its label, its value the chosen label, which its text (a label
    // inside it) carries too, for AT-SPI, which reads no text values; no
    // option shows.
    let mut ts = gui::text::TextSystem::new();
    let update = access_tree(&mut s, &mut ts);
    let combo = update
        .nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.role() == gui::Role::ComboBox)
        .expect("the trigger is read");
    assert_eq!(combo.label(), Some("Fruit"), "named by its label, not its text");
    assert_eq!(combo.value(), Some("Banana"), "its value is the chosen label");
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let inside: Vec<_> = combo.children().iter().filter_map(|c| nodes.get(c)).collect();
    assert!(
        inside
            .iter()
            .any(|n| n.role() == gui::Role::Label && n.value() == Some("Banana")),
        "its text is read inside it"
    );
    assert!(
        !inside.iter().any(|n| n.role() == gui::Role::ListBox),
        "closed, no list is read"
    );
    insta::assert_snapshot!("select_a11y_closed", a11y(&mut s));
    let at = spot(&s, "trigger", 0);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    insta::assert_snapshot!("select_a11y_open", a11y(&mut s));
}

/// A closed select's choice changing, Banana to Cherry: the update sends
/// the trigger with its new value (UI Automation's and the Mac's value
/// changes) and the label inside it with its new text (AT-SPI's name of
/// it changes), so each platform announces the change.
#[test]
fn a_closed_select_s_new_choice_is_sent_as_a_change() {
    let value = Signal::new(Some("banana".to_owned()));
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let mut ts = gui::text::TextSystem::new();
    let mut access = gui::access::Accessibility::default();
    let focus = s.read(gui::focus::focused);
    let _ = access.update(s.last(), focus, 1.0, "w", &mut ts);
    s.act("choose", || value.set(Some("cherry".to_owned())));
    s.settle().expect("settled");
    let focus = s.read(gui::focus::focused);
    let update = access.update(s.last(), focus, 1.0, "w", &mut ts);
    let sent = |role: gui::Role| {
        update
            .nodes
            .iter()
            .any(|(_, n)| n.role() == role && n.value() == Some("Cherry"))
    };
    assert!(sent(gui::Role::ComboBox), "the trigger's new value is sent");
    assert!(sent(gui::Role::Label), "and its label's new text");
    assert!(!open.get_untracked(), "closed all along");
}

// ---------------------------------------------------------------- Listbox

/// The fruits as focusable items.
fn fruit_items(focus: &[FocusHandle]) -> Vec<Item<String>> {
    FRUITS
        .iter()
        .zip(focus)
        .map(|((v, l, d), f)| {
            Item::new((*v).to_owned(), *f)
                .label(Rc::from(*l))
                .availability(Availability::from_disabled(*d))
        })
        .collect()
}

#[test]
fn a_listbox_of_several_selects_ranges_and_all_that_can_be_chosen() {
    let value: Signal<Vec<String>> = Signal::new(Vec::new());
    let focus: Rc<Vec<FocusHandle>> = Rc::new(FRUITS.iter().map(|_| FocusHandle::new()).collect());
    let list: Rc<Cell<Option<ListCore<String>>>> = Rc::default();
    let (f, l) = (Rc::clone(&focus), Rc::clone(&list));
    let mut s = scenario(move || {
        let (f, l) = (Rc::clone(&f), Rc::clone(&l));
        component((), move |_| {
            let listbox = ListCore::new(ListConfig::LISTBOX.behavior(primitives::SelectionBehavior::Replace));
            l.set(Some(listbox));
            let bound = Bound::new(value);
            move || {
                let items = fruit_items(&f);
                let rows = items.iter().enumerate().map(|(at, i)| {
                    let v = i.value.clone();
                    listbox
                        .option(
                            div()
                                .named("option")
                                .height(30.0)
                                .track_focus(i.focus)
                                .child(text(&*i.label)),
                            at,
                            &i.value,
                            i.availability,
                            value.with(|c| c.contains(&v)),
                            || {},
                        )
                        .tab_stop(false)
                });
                let keys = items.clone();
                div()
                    .role(gui::Role::ListBox)
                    .aria_multiselectable(true)
                    .on_key_down(move |ev| listbox.key(ev, &keys[..], Some(bound)).consumed())
                    .children(rows)
                    .into_element()
            }
        })
        .into_element()
    });
    let listbox = list.get().expect("the listbox");
    s.act("focus", || focus[0].focus());
    s.settle().expect("settled");
    // End with nothing highlighted goes to the last option.
    plain(&mut s, NamedKey::End);
    assert_eq!(gui::untracked(|| listbox.highlighted()).as_deref(), Some("grapes"));
    plain(&mut s, NamedKey::Home);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), ["apple"]);
    // Shift + ArrowDown extends from the anchor, the disabled one left out.
    shift(&mut s, NamedKey::ArrowDown);
    shift(&mut s, NamedKey::ArrowDown);
    assert_eq!(value.get_untracked(), ["apple", "banana", "cherry"]);
    // Control + A: everything that can be chosen.
    s.press(
        NamedKey::Enter,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        ["apple", "banana", "cherry"],
        "Control + Enter is the form's"
    );
    select_all(&mut s);
    assert_eq!(value.get_untracked(), ["apple", "banana", "cherry", "grapes"]);
}

// ---------------------------------------------------------------- Combobox

/// The fruits a combobox's list shows under `term`.
#[derive(Clone)]
struct Shown(Vec<Fruit>);

impl Shown {
    fn under(term: &str) -> Self {
        let filter = Filter::default();
        let search = filter.search(term);
        Self(FRUITS.iter().copied().filter(|(_, l, _)| search.contains(l)).collect())
    }
}

impl Items for Shown {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn availability(&self, index: usize) -> Availability {
        self.0
            .get(index)
            .map_or(Availability::Disabled, |f| Availability::from_disabled(f.2))
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        Cow::Borrowed(self.0.get(index).map_or("", |f| f.1))
    }
}

impl Options<String> for Shown {
    fn value(&self, index: usize) -> Option<String> {
        self.0.get(index).map(|f| f.0.to_owned())
    }
}

gui::engine_local! {
    [Fork] static COMBO: Cell<Option<Combobox<String, Option<String>>>> = Cell::new(None);
    [Fork] static COMBO_MANY: Cell<Option<Combobox<String, Vec<String>>>> = Cell::new(None);
}

trait StashCombo: primitives::listbox::Choice<String> {
    fn stash(c: Combobox<String, Self>);
    fn display(v: &Self) -> String;
}

impl StashCombo for Option<String> {
    fn stash(c: Combobox<String, Self>) {
        COMBO.set(Some(c));
    }

    fn display(v: &Self) -> String {
        v.clone().unwrap_or_default()
    }
}

impl StashCombo for Vec<String> {
    fn stash(c: Combobox<String, Self>) {
        COMBO_MANY.set(Some(c));
    }

    fn display(_: &Self) -> String {
        String::new()
    }
}

/// A field at the top left, its list below it.
fn combobox_ui<M: StashCombo>(value: Signal<M>, open: Signal<bool>) -> Element {
    combobox_ui_with(value, open, ComboboxConfig::REKA)
}

fn combobox_ui_with<M: StashCombo>(value: Signal<M>, open: Signal<bool>, config: ComboboxConfig) -> Element {
    combobox_ui_held(value, open, config)
}

/// A caller that keeps `open` and takes a write only while `accept` says so.
#[derive(Clone)]
struct Refusing {
    open: Signal<bool>,
    accept: Rc<Cell<bool>>,
}

impl Keeps<bool> for Refusing {
    /// A signal; `accept`, which a write only reads, is set by the test
    /// between frames.
    fn fork_policy(&self) -> primitives::listbox::HeldFork {
        primitives::listbox::HeldFork::Signals
    }

    fn get(&self) -> bool {
        self.open.get()
    }

    fn get_untracked(&self) -> bool {
        self.open.get_untracked()
    }

    fn set(&self, value: bool) {
        if self.accept.get() {
            self.open.set(value);
        }
    }
}

fn combobox_ui_held<M: StashCombo, H: Keeps<bool> + Clone>(
    value: Signal<M>,
    open: H,
    config: ComboboxConfig,
) -> Element {
    component((), move |_| {
        let field = TextField::new("");
        let combobox: Combobox<String, M> =
            Combobox::new(Bound::new(value), Bound::new(open.clone()), field, config, M::display);
        M::stash(combobox);
        combobox.set_source(Rc::new(move || {
            let shown: Rc<dyn Options<String>> = Rc::new(Shown::under(&combobox.term()));
            shown
        }));
        move || {
            let open = combobox.is_open();
            let input = combobox
                .input(text_input(&field).named("field").width(200.0).height(30.0), open)
                .on_key_down(move |ev| combobox.key(ev, false));
            let list = open.then(|| {
                let shown = Shown::under(&combobox.term());
                let rows = shown.0.iter().enumerate().map(|(i, (v, l, d))| {
                    combobox.option(
                        div().named("option").height(30.0).width(200.0).child(text(*l)),
                        i,
                        &(*v).to_owned(),
                        Availability::from_disabled(*d),
                        false,
                    )
                });
                deferred(
                    Positioning::Anchored {
                        side: Side::Bottom,
                        align: SideAlign::Start,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    },
                    combobox.content(combobox.popup(div().named("list")).children(rows)),
                )
            });
            div()
                .child(div().width(200.0).height(30.0).child(input).child(list))
                .child(div().named("elsewhere").size(30.0).focusable())
                .into_element()
        }
    })
    .into_element()
}

fn combo() -> Combobox<String, Option<String>> {
    COMBO.get().expect("the combobox")
}

fn text_of(field: TextField) -> String {
    field.edit.with_untracked(|e| e.text().to_owned())
}

#[test]
fn type_caller_refuses_close_reopen_by_click_highlights_the_chosen_option() {
    let value = Signal::new(Some("cherry".to_owned()));
    let open = Signal::new(false);
    let accept = Rc::new(Cell::new(false));
    let held = Refusing {
        open,
        accept: Rc::clone(&accept),
    };
    let config = ComboboxConfig {
        open_on_click: true,
        ..ComboboxConfig::REKA
    };
    let mut s = scenario(move || combobox_ui_held(value, held.clone(), config));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "a");
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "the caller refused the open typing asked for");
    s.act("close", || c.close(false));
    s.settle().expect("settled");
    accept.set(true);
    let at = spot(&s, "field", 0);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert!(open.get_untracked(), "a click opens it");
    assert_eq!(
        gui::untracked(|| c.list().highlighted()).as_deref(),
        Some("cherry"),
        "an opening that typing did not make highlights the chosen option"
    );
}

#[test]
fn each_reset_option_puts_the_text_back_after_a_choice_on_its_own() {
    // reka closes, then writes the value: either reset puts the chosen
    // option's text back; keeping the typed text takes turning off both.
    for (close, select, kept) in [
        (true, true, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let value = Signal::new(None);
        let open = Signal::new(false);
        let config = ComboboxConfig {
            reset_on_close: close,
            reset_on_select: select,
            ..ComboboxConfig::REKA
        };
        let mut s = scenario(move || combobox_ui_with(value, open, config));
        let c = combo();
        s.act("focus", || c.field().focus.focus());
        s.settle().expect("settled");
        typed(&mut s, "ban");
        s.settle().expect("settled");
        plain(&mut s, NamedKey::Enter);
        s.settle().expect("settled");
        assert_eq!(value.get_untracked().as_deref(), Some("banana"));
        assert!(!open.get_untracked(), "a choice of one value closes");
        let want = if kept { "ban" } else { "banana" };
        assert_eq!(
            text_of(c.field()),
            want,
            "reset_on_close {close}, reset_on_select {select}"
        );
    }
}

#[test]
fn typing_in_a_combobox_opens_filters_and_highlights_the_first_match() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "b");
    assert!(open.get_untracked(), "typing opens");
    assert_eq!(gui::untracked(|| c.term()), "b");
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("banana"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        gui::untracked(|| c.list().highlighted()).as_deref(),
        Some("banana"),
        "the disabled blueberry is skipped, and the list stops"
    );
    // Control + Enter is the form's, Enter chooses.
    control(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), None);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert!(!open.get_untracked(), "one value closes");
    assert_eq!(text_of(c.field()), "banana", "the text is put back to the value's");
    assert_eq!(gui::untracked(|| c.term()), "", "the term empties");
    assert!(
        !gui::untracked(|| c.list().has_highlight()),
        "closed, nothing is highlighted"
    );
    // Enter on a closed combobox chooses nothing.
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    // ArrowDown opens on the chosen one; the whole list shows.
    plain(&mut s, NamedKey::ArrowDown);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert_eq!(visible(&s, "option"), 5);
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("banana"));
    plain(&mut s, NamedKey::End);
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("grapes"));
    // Escape closes; the text goes back to the value's.
    typed(&mut s, "zz");
    plain(&mut s, NamedKey::Escape);
    assert!(!open.get_untracked());
    assert_eq!(text_of(c.field()), "banana");
}

#[test]
fn typing_in_a_closed_combobox_with_a_value_highlights_the_first_match_not_the_chosen() {
    // ComboboxInput.vue:68-81: typing opens the list and highlights the
    // first match, whatever is chosen.
    let value = Signal::new(Some("grapes".to_owned()));
    let open = Signal::new(false);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    // The platform's select-all, so the typing replaces the value's text.
    let all = Modifiers {
        command: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Modifiers::default()
    };
    s.shortcut('a', all);
    s.settle().expect("settled");
    typed(&mut s, "a");
    assert_eq!(text_of(c.field()), "a");
    assert!(open.get_untracked(), "typing opens");
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("apple"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("apple"));
}

#[test]
fn a_composing_input_method_keeps_the_combobox_keys() {
    let value = Signal::new(None);
    let open = Signal::new(true);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    s.settle().expect("settled");
    s.compose("ch");
    s.settle().expect("settled");
    let before = gui::untracked(|| c.list().highlighted());
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        gui::untracked(|| c.list().highlighted()),
        before,
        "the arrows are the input method's"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), None, "Enter is the input method's");
}

#[test]
fn a_combobox_of_several_toggles_and_stays_open_by_click() {
    let value: Signal<Vec<String>> = Signal::new(Vec::new());
    let open = Signal::new(true);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = COMBO_MANY.get().expect("the combobox");
    s.settle().expect("settled");
    let cherry = spot(&s, "option", 3);
    s.click(cherry.0, cherry.1);
    s.settle().expect("settled");
    let apple = spot(&s, "option", 0);
    s.click(apple.0, apple.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), ["cherry", "apple"]);
    assert!(open.get_untracked());
    assert!(focused(&mut s, c.field().focus), "focus back in the field");
    s.click(cherry.0, cherry.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), ["apple"]);
    // A press outside closes it.
    s.click(700.0, 500.0);
    s.settle().expect("settled");
    assert!(!open.get_untracked());
}

#[test]
fn combobox_a11y() {
    let value = Signal::new(Some("apple".to_owned()));
    let open = Signal::new(false);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    insta::assert_snapshot!("combobox_a11y_closed", a11y(&mut s));
    s.act("focus", || c.field().focus.focus());
    plain(&mut s, NamedKey::ArrowDown);
    s.settle().expect("settled");
    insta::assert_snapshot!("combobox_a11y_open", a11y(&mut s));
}

#[test]
fn a_combobox_trigger_controls_the_open_list_and_is_disabled_with_it() {
    let value: Signal<Option<String>> = Signal::new(None);
    let open = Signal::new(false);
    let held: Held<Combobox<String, Option<String>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let field = TextField::new("");
            let combobox: Combobox<String, Option<String>> =
                Combobox::new(Bound::new(value), Bound::new(open), field, ComboboxConfig::REKA, |_| {
                    String::new()
                });
            h.set(Some(combobox));
            move || {
                let open = combobox.is_open();
                let list = open.then(|| combobox.content(div().named("list").size(50.0)));
                div()
                    .child(combobox.input(text_input(&field).named("field").width(200.0).height(30.0), open))
                    .child(combobox.trigger(div().named("trigger").size(30.0), open, "Show popup"))
                    .child(list)
                    .into_element()
            }
        })
        .into_element()
    });
    let c = held.get().expect("the combobox");
    let trigger_line = |s: &mut Scenario| {
        a11y(s)
            .lines()
            .find(|l| l.contains("\"Show popup\""))
            .map(str::to_owned)
            .expect("the trigger")
    };
    assert!(!trigger_line(&mut s).contains("controls="), "closed: controls nothing");
    let t = spot(&s, "trigger", 0);
    s.click(t.0, t.1);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    let line = trigger_line(&mut s);
    assert!(
        line.contains("expanded=true") && line.contains("controls=ListBox"),
        "{line}"
    );
    s.click(t.0, t.1);
    s.settle().expect("settled");
    assert!(!open.get_untracked());
    s.act("disable", || c.set_availability(Availability::Disabled));
    s.settle().expect("settled");
    assert!(trigger_line(&mut s).contains(" disabled"), "disabled with the combobox");
    s.click(t.0, t.1);
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "a disabled combobox does not open");
    s.act("focus", || c.field().focus.focus());
    plain(&mut s, NamedKey::ArrowDown);
    assert!(!open.get_untracked(), "nor by its keys");
}

// ---------------------------------------------------------------- Autocomplete

#[test]
fn an_autocomplete_value_is_its_text() {
    let value = Signal::new(String::new());
    let open = Signal::new(false);
    let ac: Rc<Cell<Option<Autocomplete>>> = Rc::default();
    let a = Rc::clone(&ac);
    let mut s = scenario(move || {
        let a = Rc::clone(&a);
        component((), move |_| {
            let field = TextField::new("");
            let auto = Autocomplete::new(Bound::new(value), Bound::new(open), field);
            a.set(Some(auto));
            let c = auto.combobox();
            c.set_source(Rc::new(move || {
                let shown: Rc<dyn Options<String>> = Rc::new(Shown::under(&c.term()));
                shown
            }));
            move || {
                let open = c.is_open();
                let input = c
                    .input(text_input(&field).named("field").width(200.0).height(30.0), open)
                    .on_key_down(move |ev| c.key(ev, false));
                let list = open.then(|| {
                    let shown = Shown::under(&c.term());
                    deferred(
                        Positioning::Anchored {
                            side: Side::Bottom,
                            align: SideAlign::Start,
                            offset: 0.0,
                            match_width: false,
                            fit_height: false,
                        },
                        c.content(c.popup(div().named("list")))
                            .children(shown.0.iter().enumerate().map(|(i, (v, l, d))| {
                                c.option(
                                    div().named("option").height(30.0).child(text(*l)),
                                    i,
                                    &(*v).to_owned(),
                                    Availability::from_disabled(*d),
                                    false,
                                )
                            })),
                    )
                });
                div().child(input).child(list).into_element()
            }
        })
        .into_element()
    });
    let auto = ac.get().expect("the autocomplete");
    let c = auto.combobox();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "Ch");
    assert_eq!(value.get_untracked(), "Ch", "typing writes the value");
    assert!(open.get_untracked());
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("cherry"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), "cherry", "choosing writes the option's value");
    assert!(!open.get_untracked());
    assert_eq!(text_of(c.field()), "cherry", "closing keeps the text");
    // Opening filters by the text and highlights the option valued as it.
    plain(&mut s, NamedKey::ArrowDown);
    assert!(open.get_untracked());
    assert_eq!(gui::untracked(|| c.term()), "cherry");
    assert_eq!(gui::untracked(|| c.list().highlighted()).as_deref(), Some("cherry"));
    // A click on a row chooses it: its value is the text.
    plain(&mut s, NamedKey::Escape);
    s.act("clear", || c.cancel());
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), "", "cancel empties the text");
    typed(&mut s, "a");
    let at = spot(&s, "option", 1);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), "banana", "the clicked option's value");
    assert!(!open.get_untracked());
    // A caller's value is the field's text.
    s.act("write", || value.set("grape".to_owned()));
    s.settle().expect("settled");
    assert_eq!(text_of(c.field()), "grape");
    assert_eq!(
        gui::untracked(|| c.value()).as_deref(),
        Some("grape"),
        "the combobox's own value is the text"
    );
    insta::assert_snapshot!("autocomplete_a11y", a11y(&mut s));
}

// ---------------------------------------------------------------- TagsInput

fn tags_scenario(tags: Signal<Vec<String>>, config: TagsConfig) -> (Scenario, TagsInput<String>) {
    let held: Rc<Cell<Option<TagsInput<String>>>> = Rc::default();
    let h = Rc::clone(&held);
    let config = Rc::new(config);
    let s = scenario(move || {
        let (h, config) = (Rc::clone(&h), Rc::clone(&config));
        component((), move |_| {
            let field = TextField::new("");
            let input = TagsInput::strings(Bound::new(tags), field, (*config).clone(), Direction::Ltr.into());
            h.set(Some(input));
            move || {
                let items = input.tags().into_iter().enumerate().map(|(i, t)| {
                    input
                        .item(div().named("tag").child(text(t)), i)
                        .child(input.delete(div().named("delete").size(10.0), i))
                });
                div()
                    .children(items)
                    .child(
                        text_input(&field)
                            .named("field")
                            .width(200.0)
                            .height(30.0)
                            .on_key_down(move |ev| input.key(ev)),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let input = held.get().expect("the tags input");
    (s, input)
}

#[test]
fn a_tags_input_adds_by_one_rule() {
    let tags = Signal::new(vec!["apple".to_owned()]);
    let (mut s, input) = tags_scenario(tags, TagsConfig::default());
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "  banana  ");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(tags.get_untracked(), ["apple", "banana"], "trimmed");
    assert_eq!(text_of(input.field()), "");
    typed(&mut s, "   ");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(tags.get_untracked(), ["apple", "banana"], "a blank draft adds nothing");
    // A duplicate is refused, kept typed, and marks the input invalid until the next change.
    s.act("clear", || input.field().set_text(""));
    typed(&mut s, "apple");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(tags.get_untracked(), ["apple", "banana"]);
    assert_eq!(text_of(input.field()), "apple");
    assert!(gui::untracked(|| input.is_invalid()));
    typed(&mut s, "s");
    assert!(!gui::untracked(|| input.is_invalid()));
    // The delimiter adds what is before it, and is not typed.
    s.act("clear", || input.field().set_text(""));
    typed(&mut s, "cherry,");
    assert_eq!(tags.get_untracked(), ["apple", "banana", "cherry"]);
    assert_eq!(text_of(input.field()), "");
}

/// An arrow key moving the caret in a field: what watches the field's
/// value (a filter, a model, a draft) runs not at all, and its text is
/// copied nowhere.
fn an_arrow_is_quiet(s: &mut Scenario, what: &str) {
    let copies = gui::edit::text_copies();
    // Each frame's report takes what the runtime did since the last.
    gui::reactive::start_trace();
    s.press(NamedKey::ArrowLeft, Modifiers::default());
    let mut runs = 0;
    for k in 0..3 {
        runs += s.frame(&format!("arrow {k}")).expect("a frame").reactive.effect_runs;
    }
    gui::reactive::stop_trace();
    assert_eq!(runs, 0, "{what}: an arrow ran watches");
    assert_eq!(gui::edit::text_copies() - copies, 0, "{what}: an arrow copied its text");
}

#[test]
fn an_arrow_in_a_combobox_or_a_tags_input_runs_no_value_watch_and_copies_no_text() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "ba");
    an_arrow_is_quiet(&mut s, "a combobox");
    let (mut s, input) = tags_scenario(Signal::new(Vec::new()), TagsConfig::default());
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "ab");
    an_arrow_is_quiet(&mut s, "a tags input");
}

#[test]
fn a_tags_input_pastes_its_pieces_and_leaves_the_draft() {
    let tags = Signal::new(vec!["apple".to_owned()]);
    let config = TagsConfig {
        add_on_paste: true,
        ..TagsConfig::default()
    };
    let (mut s, input) = tags_scenario(tags, config);
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    // No delimiter in it: the pasted text is one tag (reka's `split` gives it whole).
    s.act("paste", || input.paste("  banana "));
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked(), ["apple", "banana"], "one piece, trimmed");
    // The paste alone is split; the draft is left as it was.
    typed(&mut s, "ab");
    s.act("paste", || input.paste("c, ,d"));
    s.settle().expect("settled");
    assert_eq!(
        tags.get_untracked(),
        ["apple", "banana", "c", "d"],
        "blank pieces dropped"
    );
    assert_eq!(text_of(input.field()), "ab", "the draft is not part of the paste");
    // What cannot be added (a duplicate) is pasted as text at the caret.
    s.act("clear", || input.field().set_text(""));
    s.act("paste", || input.paste("apple,e"));
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked(), ["apple", "banana", "c", "d", "e"]);
    assert_eq!(text_of(input.field()), "apple", "kept, not lost");
    assert!(gui::untracked(|| input.is_invalid()), "the duplicate marks it invalid");
    // An input method's commit holding the delimiter is typing, no paste.
    s.act("clear", || input.field().set_text(""));
    s.send(gui::window::Input::Ime(gui::ImeEvent::Commit("x,y".to_owned())));
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked().len(), 5, "a commit adds nothing");
    assert_eq!(text_of(input.field()), "x,y");
}

#[test]
fn a_tags_input_adds_on_tab_and_blur_up_to_its_limit() {
    let tags = Signal::new(Vec::new());
    let config = TagsConfig {
        add_on_tab: true,
        add_on_blur: true,
        max: 2,
        ..TagsConfig::default()
    };
    let (mut s, input) = tags_scenario(tags, config);
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, " a ");
    plain(&mut s, NamedKey::Tab);
    assert_eq!(tags.get_untracked(), ["a"], "Tab adds, trimmed");
    assert!(focused(&mut s, input.field().focus), "and focus stays");
    let empty_tab = gui::KeyEvent {
        key: gui::Key::Named(NamedKey::Tab),
        text: None,
        modifiers: Modifiers::default(),
    };
    assert!(!input.key(&empty_tab), "an empty draft: Tab is left to move focus");
    assert_eq!(tags.get_untracked(), ["a"]);
    typed(&mut s, "b");
    s.act("blur", || gui::focus::set_focus(None, false));
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked(), ["a", "b"], "blur adds");
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "c");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(tags.get_untracked(), ["a", "b"], "the limit refuses");
    assert_eq!(text_of(input.field()), "c", "the draft kept");
    assert!(!gui::untracked(|| input.is_invalid()), "a limit marks nothing invalid");
}

#[test]
fn focus_moving_into_the_list_the_field_controls_adds_no_draft() {
    let tags: Signal<Vec<String>> = Signal::new(Vec::new());
    // The tags input, a suggestion in its list, and a node outside.
    type Held = (TagsInput<String>, FocusHandle, FocusHandle);
    let held: Rc<Cell<Option<Held>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let field = TextField::new("");
            let config = TagsConfig {
                add_on_blur: true,
                ..TagsConfig::default()
            };
            let input = TagsInput::strings(Bound::new(tags), field, config, Direction::Ltr.into());
            let (option, outside) = (FocusHandle::new(), FocusHandle::new());
            h.set(Some((input, option, outside)));
            move || {
                div()
                    .child(
                        text_input(&field)
                            .named("field")
                            .width(200.0)
                            .height(30.0)
                            .controls(input.list_ref())
                            .on_key_down(move |ev| input.key(ev)),
                    )
                    .child(
                        input.list(
                            div()
                                .named("suggestions")
                                .child(div().named("suggestion").size(20.0).track_focus(option)),
                        ),
                    )
                    .child(div().named("outside").size(20.0).track_focus(outside))
                    .into_element()
            }
        })
        .into_element()
    });
    let (input, option, outside) = held.get().expect("the tags input");
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "ap");
    s.act("into the list", || option.focus());
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert!(tags.get_untracked().is_empty(), "into the list: the choice there wins");
    assert_eq!(text_of(input.field()), "ap", "the draft kept");
    s.act("back", || input.field().focus.focus());
    s.settle().expect("settled");
    s.act("out", || outside.focus());
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked(), ["ap"], "anywhere else: blur adds");
}

#[test]
fn a_removal_selects_the_next_usable_tag() {
    let tags = Signal::new(vec!["a".to_owned(), "b".to_owned(), "c".to_owned(), "d".to_owned()]);
    let (mut s, input) = tags_scenario(tags, TagsConfig::default());
    // The tag now at index 1 after a removal ("c") and the first ("a") cannot be used.
    s.act("disable", || {
        input.set_tag_availability(move |i| {
            let t = tags.get_untracked();
            match t.get(i).map(String::as_str) {
                Some("a" | "c") => primitives::Availability::Disabled,
                _ => primitives::Availability::Enabled,
            }
        });
    });
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    let selected = || gui::untracked(|| input.selected());
    plain(&mut s, NamedKey::Backspace);
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(selected(), Some(1), "\"b\", past the disabled \"c\"");
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(tags.get_untracked(), ["a", "c", "d"]);
    assert_eq!(selected(), Some(2), "\"c\" cannot be used: the next usable, \"d\"");
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(tags.get_untracked(), ["a", "c"]);
    assert_eq!(selected(), None, "nothing usable left");
}

#[test]
fn the_tags_keyboard_table() {
    let tags = Signal::new(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]);
    let (mut s, input) = tags_scenario(tags, TagsConfig::default());
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    let selected = || gui::untracked(|| input.selected());
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(selected(), Some(2), "Backspace selects the last");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(selected(), Some(1));
    plain(&mut s, NamedKey::Home);
    assert_eq!(selected(), Some(0), "Home selects the first");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(selected(), Some(0), "stops at the first");
    plain(&mut s, NamedKey::End);
    assert_eq!(selected(), Some(2));
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(selected(), None, "past the last: back to typing");
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::Delete);
    assert_eq!(tags.get_untracked(), ["a", "c"]);
    assert_eq!(selected(), Some(1), "the next is selected");
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(tags.get_untracked(), ["a"]);
    assert_eq!(selected(), Some(0), "the last removed: the one before");
    typed(&mut s, "x");
    assert_eq!(selected(), None, "typing lets the selected one go");
    // Not at the start: the keys are the field's.
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(tags.get_untracked(), ["a"]);
    assert_eq!(text_of(input.field()), "");
}

#[test]
fn a_tag_s_delete_button_is_named_for_its_action_and_removes_it() {
    let tags = Signal::new(vec!["apple".to_owned(), "banana".to_owned()]);
    let (mut s, input) = tags_scenario(tags, TagsConfig::default());
    s.act("focus", || input.field().focus.focus());
    plain(&mut s, NamedKey::Backspace);
    insta::assert_snapshot!("tags_input_a11y", a11y(&mut s));
    let at = spot(&s, "delete", 0);
    s.click(at.0, at.1);
    s.settle().expect("settled");
    assert_eq!(tags.get_untracked(), ["banana"]);
}

// ---------------------------------------------------------------- Tree

#[derive(Clone, Debug, PartialEq)]
struct Node3 {
    key: &'static str,
    children: Option<Vec<Node3>>,
    disabled: bool,
}

fn leaf(key: &'static str) -> Node3 {
    Node3 {
        key,
        children: None,
        disabled: false,
    }
}

fn data() -> TreeData<Node3, &'static str> {
    let roots = vec![
        Node3 {
            key: "docs",
            children: Some(vec![
                leaf("a.md"),
                Node3 {
                    disabled: true,
                    ..leaf("b.md")
                },
                leaf("c.md"),
            ]),
            disabled: false,
        },
        leaf("readme"),
    ];
    TreeData {
        roots: Rc::new(move || roots.clone()),
        children: Rc::new(|n: &Node3| n.children.clone()),
        key: Rc::new(|n: &Node3| n.key),
        label: Rc::new(|n: &Node3| n.key.to_owned()),
        availability: Rc::new(|n: &Node3| Availability::from_disabled(n.disabled)),
    }
}

type TestTree = Tree<Node3, &'static str, Vec<&'static str>>;

fn tree_scenario(
    value: Signal<Vec<&'static str>>,
    expanded: Signal<Vec<&'static str>>,
    behavior: primitives::SelectionBehavior,
) -> (Scenario, TestTree) {
    tree_scenario_with(
        value,
        expanded,
        TreeConfig {
            behavior,
            propagate: false,
        },
    )
}

fn tree_scenario_with(
    value: Signal<Vec<&'static str>>,
    expanded: Signal<Vec<&'static str>>,
    config: TreeConfig,
) -> (Scenario, TestTree) {
    let held: Rc<Cell<Option<TestTree>>> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let tree: TestTree = Tree::new(
                data(),
                Bound::new(value),
                Bound::new(expanded),
                config,
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            move || {
                let rows: Rc<Rows<Node3, &'static str>> = tree.rows();
                let built: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| tree.row(div().named("row").height(24.0).child(text(r.label.clone())), &rows, i))
                    .collect();
                tree.root(div().named("tree")).children(built).into_element()
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    (s, tree)
}

#[test]
fn a_tree_shows_new_data_without_being_made_again() {
    // reka's Tree `items` is reactive: roots and children read tracked.
    let roots = Signal::new(vec!["src", "readme"]);
    let files = Signal::new(vec!["a.rs"]);
    let value: Signal<Vec<&'static str>> = Signal::new(Vec::new());
    let expanded = Signal::new(vec!["src"]);
    let held: Held<TestTree> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let data = TreeData {
                roots: Rc::new(move || {
                    roots
                        .get()
                        .into_iter()
                        .map(|k| Node3 {
                            children: (k == "src").then(Vec::new),
                            ..leaf(k)
                        })
                        .collect()
                }),
                children: Rc::new(move |n: &Node3| {
                    n.children.as_ref().map(|_| files.get().into_iter().map(leaf).collect())
                }),
                key: Rc::new(|n: &Node3| n.key),
                label: Rc::new(|n: &Node3| n.key.to_owned()),
                availability: Rc::new(|_: &Node3| Availability::Enabled),
            };
            let tree: TestTree = Tree::new(
                data,
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig {
                    behavior: primitives::SelectionBehavior::Toggle,
                    propagate: true,
                },
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            move || {
                let rows: Rc<Rows<Node3, &'static str>> = tree.rows();
                let built: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| tree.row(div().named("row").height(24.0).child(text(r.label.clone())), &rows, i))
                    .collect();
                tree.root(div().named("tree")).children(built).into_element()
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    let keys = || gui::untracked(|| tree.rows().iter().map(|r| r.key).collect::<Vec<_>>());
    let state = |key: &str| {
        gui::untracked(|| {
            let rows = tree.rows();
            let row = rows.index_of(&key).and_then(|i| rows.get(i).cloned()).expect("the row");
            tree.state_of(&row)
        })
    };
    assert_eq!(keys(), ["src", "a.rs", "readme"]);
    assert_eq!(state("src"), gui::Toggled::False, "nothing chosen");
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::End);
    assert!(gui::untracked(|| tree.is_highlighted(&"readme")));
    // A root comes, a child comes: shown, the highlight kept on its key.
    s.act("roots", || roots.set(vec!["docs", "src", "readme"]));
    s.act("files", || files.set(vec!["a.rs", "b.rs"]));
    s.settle().expect("settled");
    assert_eq!(keys(), ["docs", "src", "a.rs", "b.rs", "readme"]);
    assert_eq!(visible(&s, "row"), 5);
    assert!(gui::untracked(|| tree.is_highlighted(&"readme")), "the highlight stays");
    plain(&mut s, NamedKey::ArrowUp);
    assert!(
        gui::untracked(|| tree.is_highlighted(&"b.rs")),
        "and moves in the new rows"
    );
    // The parent map follows: the new child chosen half-chooses its parent.
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), ["b.rs"]);
    assert_eq!(state("src"), gui::Toggled::Mixed);
    // The Tab stop stays on its key; away and back, Tab finds it.
    plain(&mut s, NamedKey::Tab);
    shift(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&"b.rs")), "the stop stays");
}

#[test]
fn the_tree_keyboard_table() {
    let value = Signal::new(Vec::new());
    let expanded = Signal::new(Vec::new());
    let (mut s, tree) = tree_scenario(value, expanded, primitives::SelectionBehavior::Toggle);
    plain(&mut s, NamedKey::Tab);
    let at = || {
        gui::untracked(|| {
            ["docs", "a.md", "b.md", "c.md", "readme"]
                .into_iter()
                .find(|k| tree.is_highlighted(k))
        })
    };
    assert_eq!(at(), Some("docs"), "Tab reaches the tree's one stop");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(expanded.get_untracked(), ["docs"], "Right opens");
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(at(), Some("a.md"), "Right on an open one: its first child");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(at(), Some("c.md"), "the disabled one is skipped");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(at(), Some("docs"), "Left on a leaf: its parent");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(expanded.get_untracked().is_empty(), "Left on an open one closes it");
    plain(&mut s, NamedKey::End);
    assert_eq!(at(), Some("readme"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), ["readme"]);
    plain(&mut s, NamedKey::Home);
    plain(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), ["readme", "docs"], "several toggle");
    typed(&mut s, "r");
    assert_eq!(at(), Some("readme"), "typeahead");
}

#[test]
fn a_propagating_tree_chooses_alike_by_key_and_by_click() {
    let value = Signal::new(Vec::new());
    let expanded = Signal::new(vec!["docs"]);
    let (mut s, tree) = tree_scenario_with(
        value,
        expanded,
        TreeConfig {
            behavior: primitives::SelectionBehavior::Toggle,
            propagate: true,
        },
    );
    let state = |key: &str| {
        gui::untracked(|| {
            let rows = tree.rows();
            let row = rows.index_of(&key).and_then(|i| rows.get(i).cloned()).expect("the row");
            tree.state_of(&row)
        })
    };
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Space);
    assert_eq!(
        value.get_untracked(),
        ["docs", "a.md", "b.md", "c.md"],
        "Space on a parent chooses its descendants"
    );
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(
        value.get_untracked(),
        ["b.md", "c.md"],
        "Enter lets a child go, and its parent with it"
    );
    assert_eq!(state("docs"), gui::Toggled::Mixed, "half chosen");
    let a = spot(&s, "row", 1);
    s.click(a.0, a.1);
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        ["b.md", "c.md", "a.md", "docs"],
        "a click fills the parent in, as the keys do"
    );
    assert_eq!(state("docs"), gui::Toggled::True);
}

#[test]
fn a_tree_s_shift_range_leaves_disabled_rows_out_and_a_click_toggles() {
    let value = Signal::new(Vec::new());
    let expanded = Signal::new(vec!["docs"]);
    let (mut s, tree) = tree_scenario(value, expanded, primitives::SelectionBehavior::Replace);
    let a = spot(&s, "row", 1);
    s.click(a.0, a.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), ["a.md"]);
    shift(&mut s, NamedKey::ArrowDown);
    assert_eq!(value.get_untracked(), ["a.md", "c.md"], "b.md cannot be chosen");
    // The highlight is the focus: the range's end has both.
    assert!(gui::untracked(|| tree.is_highlighted(&"c.md")));
    assert_eq!(
        focused_label(&mut s).as_deref(),
        Some("c.md"),
        "focus went with the range"
    );
    shift(&mut s, NamedKey::ArrowDown);
    assert_eq!(value.get_untracked(), ["a.md", "c.md", "readme"]);
    assert_eq!(focused_label(&mut s).as_deref(), Some("readme"));
    shift(&mut s, NamedKey::ArrowUp);
    assert_eq!(
        value.get_untracked(),
        ["a.md", "c.md"],
        "the range shrinks from its end"
    );
    assert_eq!(focused_label(&mut s).as_deref(), Some("c.md"));
    // No select-all in a tree (reka's has none): Control + A is the page's.
    select_all(&mut s);
    assert_eq!(value.get_untracked(), ["a.md", "c.md"]);
    // A click on a parent chooses it and closes it.
    let docs = spot(&s, "row", 0);
    s.click(docs.0, docs.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), ["docs"]);
    assert!(expanded.get_untracked().is_empty());
    insta::assert_snapshot!("tree_a11y", a11y(&mut s));
}

// ---------------------------------------------------------------- Command

const ITEMS: usize = 10_000;

fn palette() -> Arc<Vec<CommandSection>> {
    palette_of(ITEMS)
}

fn palette_of(items: usize) -> Arc<Vec<CommandSection>> {
    Arc::new(vec![
        CommandSection::group(
            Some("Letters"),
            [CommandItem::new("alpha"), CommandItem::disabled("beta")],
        ),
        CommandSection::Separator,
        CommandSection::group(
            Some("Numbers"),
            (0..items).map(|i| CommandItem::new(format!("number {i}"))),
        ),
        CommandSection::empty("No results found."),
    ])
}

fn command_scenario(picked: Rc<RefCell<Vec<String>>>) -> (Scenario, Command, TextField) {
    command_scenario_of(picked, palette())
}

fn command_scenario_of(
    picked: Rc<RefCell<Vec<String>>>,
    sections: Arc<Vec<CommandSection>>,
) -> (Scenario, Command, TextField) {
    let held: Rc<Cell<Option<(Command, TextField)>>> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || {
        let (h, picked, sections) = (Rc::clone(&h), Rc::clone(&picked), Arc::clone(&sections));
        component((), move |_| {
            let search = TextField::new("");
            let chosen = Signal::new(String::new());
            let cmd = Command::new(
                search,
                move || search.edit.with(|e| e.text().to_owned()),
                Command::listbox(),
                move |item: &CommandItem| chosen.with(|c| *c == item.value),
                move |item: CommandItem| {
                    chosen.set(item.value.clone());
                    picked.borrow_mut().push(item.value);
                    search.set_text("");
                },
            );
            h.set(Some((cmd, search)));
            let footer = Arc::new(Vec::new());
            let data = Arc::clone(&sections);
            let model = cmd.list_setup(move || (Arc::clone(&data), Arc::clone(&footer)), 30.0, None);
            move || {
                let flat = model.flat.get();
                let data = &*flat.0;
                model.state.fit_rows(Some(300.0));
                let options = options_of(&data.rows);
                let on = cmd.highlighted_in(data);
                let on = on.and_then(|i| data.item(i).map(|r| r.value.clone()));
                let body = virtual_list_in(
                    &model.state,
                    u64::try_from(data.rows.len()).unwrap_or(u64::MAX),
                    |i| data.key(usize::try_from(i).unwrap_or(usize::MAX)),
                    |i| {
                        let i = usize::try_from(i).unwrap_or(usize::MAX);
                        match data.rows.get(i) {
                            Some(CommandRow::Item { section, item, .. }) => {
                                let it = &sections[*section].items()[*item];
                                cmd.option(
                                    div().named("item").height(30.0).child(text(it.value.clone())),
                                    &model,
                                    it,
                                    sections[*section].heading(),
                                    on.as_deref() == Some(it.value.as_str()),
                                    options.get(i).copied().flatten(),
                                )
                                .into_element()
                            }
                            // A heading and a separator are no options.
                            Some(CommandRow::Separator { .. }) => {
                                div().height(30.0).role(gui::Role::Splitter).into_element()
                            }
                            Some(_) => div().height(30.0).role(gui::Role::GenericContainer).into_element(),
                            None => Element::Empty,
                        }
                    },
                    |v| v,
                )
                .named("list")
                .width(400.0);
                div()
                    .on_key_down(move |ev| cmd.key(ev))
                    .child(cmd.field(text_input(&search).named("search").width(400.0).height(30.0)))
                    .child(cmd.list_node(body, &model))
                    .into_element()
            }
        })
        .into_element()
    });
    let (cmd, search) = held.get().expect("the command");
    (s, cmd, search)
}

#[test]
fn a_command_walks_a_virtual_list_by_data_and_picks() {
    let picked: Rc<RefCell<Vec<String>>> = Rc::default();
    let (mut s, cmd, search) = command_scenario(Rc::clone(&picked));
    s.act("focus", || search.focus.focus());
    s.settle().expect("settled");
    assert_eq!(
        cmd.highlighted().as_deref(),
        Some("alpha"),
        "nothing highlighted: the first"
    );
    assert!(visible(&s, "item") < 60, "few rows built");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        cmd.highlighted().as_deref(),
        Some("number 0"),
        "the disabled one is skipped"
    );
    plain(&mut s, NamedKey::End);
    assert_eq!(cmd.highlighted(), Some(format!("number {}", ITEMS - 1)));
    assert!(s.locate(&format!("number {}", ITEMS - 1)).is_ok(), "brought into view");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        cmd.highlighted(),
        Some(format!("number {}", ITEMS - 1)),
        "stops at the end"
    );
    // A new search highlights its first item.
    typed(&mut s, "number 99");
    assert_eq!(cmd.highlighted().as_deref(), Some("number 99"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*picked.borrow(), ["number 99"]);
    assert_eq!(text_of(search), "", "picking empties the search");
    // Back to the whole list: the stale highlight does not come back.
    assert_eq!(cmd.highlighted().as_deref(), Some("alpha"));
    typed(&mut s, "zzz");
    assert_eq!(cmd.highlighted(), None);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(picked.borrow().len(), 1, "nothing to pick");
}

#[test]
fn a_composing_input_method_keeps_the_command_keys() {
    let picked: Rc<RefCell<Vec<String>>> = Rc::default();
    let (mut s, cmd, search) = command_scenario(Rc::clone(&picked));
    s.act("focus", || search.focus.focus());
    s.settle().expect("settled");
    s.compose("ch");
    s.settle().expect("settled");
    let before = cmd.highlighted();
    for k in [NamedKey::ArrowDown, NamedKey::End, NamedKey::Home] {
        plain(&mut s, k);
        assert_eq!(cmd.highlighted(), before, "{k:?} is the input method's");
    }
    plain(&mut s, NamedKey::Enter);
    assert!(picked.borrow().is_empty(), "Enter is the input method's");
}

#[test]
fn command_a11y() {
    let picked: Rc<RefCell<Vec<String>>> = Rc::default();
    let (mut s, _cmd, search) = command_scenario(picked);
    s.act("focus", || search.focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "number 12");
    plain(&mut s, NamedKey::ArrowDown);
    insta::assert_snapshot!("command_a11y", a11y(&mut s));
}

#[test]
fn a_listbox_key_reports_what_it_did() {
    let focus: Vec<FocusHandle> = FRUITS.iter().map(|_| FocusHandle::new()).collect();
    let items = fruit_items(&focus);
    let mut s = scenario(|| div().into_element());
    let mut keys = Vec::new();
    s.act("keys", || {
        let list: ListCore<String> = ListCore::new(ListConfig::LISTBOX);
        let value: Bound<Option<String>> = Bound::new(Signal::new(None));
        for k in [NamedKey::ArrowDown, NamedKey::Enter, NamedKey::Tab] {
            let ev = gui::KeyEvent {
                key: gui::Key::Named(k),
                text: None,
                modifiers: Modifiers::default(),
            };
            keys.push(list.key(&ev, &items[..], Some(value)));
        }
    });
    assert!(matches!(keys[0], ListKey::Highlighted(0)));
    assert!(matches!(keys[1], ListKey::Chose(0)));
    assert!(matches!(keys[2], ListKey::Ignored));
}

// ---------------------------------------------------------------- Review fixes

/// A part made in a scenario's setup, handed out to the test.
type Held<P> = Rc<Cell<Option<P>>>;

#[test]
fn choosing_the_chosen_option_keeps_a_select_s_value() {
    let value = Signal::new(Some("cherry".to_owned()));
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    // Opening focuses the chosen option; Enter on it keeps it.
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    assert_eq!(focused_option(&mut s).as_deref(), Some("cherry"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"), "Enter keeps it");
    assert!(!open.get_untracked(), "and closes");
    // A click on the chosen option keeps it.
    plain(&mut s, NamedKey::Space);
    s.settle().expect("settled");
    let cherry = spot(&s, "option", 3);
    s.click(cherry.0, cherry.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("cherry"), "a click keeps it");
    // Typing its letter on the closed trigger, the only match: kept.
    s.act("grapes", || value.set(Some("grapes".to_owned())));
    s.settle().expect("settled");
    typed(&mut s, "g");
    assert_eq!(value.get_untracked().as_deref(), Some("grapes"), "typeahead keeps it");
}

#[test]
fn a_caller_refuses_a_select_choice() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    s.act("veto", || {
        select.on_pick(|v: String| {
            if v == "banana" {
                primitives::listbox::Pick::Refuse
            } else {
                primitives::listbox::Pick::Allow
            }
        });
    });
    s.act("focus", || select.trigger_focus().focus());
    s.settle().expect("settled");
    typed(&mut s, "b");
    assert_eq!(value.get_untracked(), None, "typeahead on a closed trigger is refused");
    s.wait(RESET + Duration::from_millis(10));
    plain(&mut s, NamedKey::Enter);
    s.settle().expect("settled");
    let banana = spot(&s, "option", 1);
    s.click(banana.0, banana.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), None, "a click is refused");
    assert!(open.get_untracked(), "and the list stays open");
    let apple = spot(&s, "option", 0);
    s.click(apple.0, apple.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("apple"));
}

#[test]
fn an_empty_select_sends_its_fallback() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = select_scenario(value, open);
    let select = one();
    let data = || gui::untracked(|| select.form_data()).pairs("fruit");
    assert_eq!(data(), [("fruit".to_owned(), String::new())], "'' by default");
    s.act("fallback", || select.set_fallback("none"));
    assert_eq!(data(), [("fruit".to_owned(), "none".to_owned())]);
    s.act("apple", || value.set(Some("apple".to_owned())));
    s.settle().expect("settled");
    assert_eq!(data(), [("fruit".to_owned(), "apple".to_owned())]);
}

gui::engine_local! {
    [Fork] static LISTBOX: Cell<Option<primitives::listbox::Listbox<String, Option<String>>>> = Cell::new(None);
}

/// A button before the list, then a listbox of the fruits (with a filter
/// field when `filter`).
fn listbox_scenario(value: Signal<Option<String>>, filter: bool) -> Scenario {
    use primitives::listbox::Listbox;
    scenario(move || {
        component((), move |_| {
            let list: Listbox<String, Option<String>> = Listbox::new(Bound::new(value), ListConfig::LISTBOX);
            LISTBOX.set(Some(list));
            let field = TextField::new("");
            if filter {
                list.set_filter(field);
            }
            let focus: Vec<FocusHandle> = FRUITS.iter().map(|_| FocusHandle::new()).collect();
            move || {
                let term = field.edit.with(|e| e.text().to_owned());
                let search = Filter::default();
                let search = search.search(&term);
                let options = FRUITS
                    .iter()
                    .zip(&focus)
                    .filter(|((_, l, _), _)| search.contains(l))
                    .map(|((v, l, d), f)| {
                        list.item(
                            div().named("option").height(30.0).width(200.0).child(text(*l)),
                            (*v).to_owned(),
                            Rc::from(*l),
                            *f,
                            Availability::from_disabled(*d),
                        )
                    });
                let content = list.content(div().named("list").width(200.0).children(options));
                let root = div()
                    .child(div().named("before").size(20.0).focusable())
                    .when(filter, |d| {
                        d.child(list.filter(text_input(&field).named("filter").width(200.0).height(30.0)))
                    })
                    .child(content);
                div().child(list.root(root.named("root").width(300.0))).into_element()
            }
        })
        .into_element()
    })
}

fn the_listbox() -> primitives::listbox::Listbox<String, Option<String>> {
    LISTBOX.get().expect("the listbox")
}

fn listbox_highlight() -> Option<String> {
    let list = the_listbox();
    FRUITS
        .iter()
        .map(|(v, ..)| (*v).to_owned())
        .find(|v| gui::untracked(|| list.is_highlighted(v)))
}

#[test]
fn a_listbox_is_entered_by_tab_and_remembers_what_it_left() {
    let value = Signal::new(Some("cherry".to_owned()));
    let mut s = listbox_scenario(value, false);
    let before = s.read(gui::focus::focused);
    assert_eq!(
        listbox_highlight().as_deref(),
        Some("cherry"),
        "mounting highlights the chosen one"
    );
    assert_eq!(s.read(gui::focus::focused), before, "and moves no focus");
    // Tab from the button lands on the highlighted option, the one stop.
    let b = spot(&s, "before", 0);
    s.click(b.0, b.1);
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Tab);
    assert_eq!(listbox_highlight().as_deref(), Some("cherry"));
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(listbox_highlight().as_deref(), Some("grapes"));
    // Leaving lets the highlight go and remembers it.
    shift(&mut s, NamedKey::Tab);
    assert_eq!(listbox_highlight(), None, "focus left: nothing highlighted");
    // Coming back by Tab: the list itself is the stop, and entering it
    // highlights the option it left.
    plain(&mut s, NamedKey::Tab);
    assert_eq!(listbox_highlight().as_deref(), Some("grapes"), "the one it left");
    // A click chooses (toggle, reka's Listbox default).
    let apple = spot(&s, "option", 0);
    s.click(apple.0, apple.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked().as_deref(), Some("apple"));
    assert_eq!(listbox_highlight().as_deref(), Some("apple"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(
        value.get_untracked(),
        None,
        "one value, toggled: Enter on the chosen clears it"
    );
    // A caller's value highlights its option, focus with it.
    s.act("write", || value.set(Some("banana".to_owned())));
    s.settle().expect("settled");
    assert_eq!(listbox_highlight().as_deref(), Some("banana"));
    // The pointer leaving the root lets the highlight go.
    let o = spot(&s, "option", 0);
    s.hover(o.0, o.1);
    s.settle().expect("settled");
    s.hover(700.0, 500.0);
    s.settle().expect("settled");
    assert_eq!(listbox_highlight(), None, "the pointer left the root");
    insta::assert_snapshot!("listbox_a11y", a11y(&mut s));
}

#[test]
fn a_listbox_with_a_filter_keeps_focus_in_its_field() {
    let value = Signal::new(None);
    let mut s = listbox_scenario(value, true);
    let f = spot(&s, "filter", 0);
    s.click(f.0, f.1);
    s.settle().expect("settled");
    let field = s.read(gui::focus::focused);
    typed(&mut s, "b");
    assert_eq!(
        listbox_highlight().as_deref(),
        Some("banana"),
        "typing highlights the first match"
    );
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        listbox_highlight().as_deref(),
        Some("banana"),
        "the disabled one is skipped, the end stops"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert_eq!(s.read(gui::focus::focused), field, "focus stays in the field");
    s.act("write", || value.set(Some("cherry".to_owned())));
    s.settle().expect("settled");
    assert_eq!(
        s.read(gui::focus::focused),
        field,
        "a caller's value moves no focus with a filter"
    );
}

gui::engine_local! {
    /// How many option values [`million`]'s data gave.
    [Fork] static VALUES_READ: Cell<usize> = Cell::new(0);
}

/// The data of a virtual list of `N` options: the last and every seventh
/// disabled (999 999 by hand, 999 998 by the sevens).
fn million() -> Rc<dyn Options<usize>> {
    const N: usize = 1_000_000;
    Rc::new(primitives::listbox::options_fn(
        N,
        |i| Availability::from_disabled(i == N - 1 || i % 7 == 6),
        |i| Cow::Owned(format!("option {i}")),
        |i| {
            VALUES_READ.set(VALUES_READ.get() + 1);
            Some(i)
        },
    ))
}

/// A button, then a virtual listbox over [`million`] holding `value`.
fn virtual_listbox(
    value: Signal<Option<usize>>,
    hover: bool,
) -> (Scenario, primitives::listbox::Listbox<usize, Option<usize>>) {
    use primitives::listbox::Listbox;
    let held: Held<Listbox<usize, Option<usize>>> = Rc::default();
    let h = Rc::clone(&held);
    let s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let list: Listbox<usize, Option<usize>> = Listbox::new(Bound::new(value), ListConfig::LISTBOX.hover(hover));
            h.set(Some(list));
            let state = gui::list::ListState::new(30.0);
            list.set_virtual(state, Rc::new(million));
            move || {
                let options = million();
                let n = u64::try_from(options.len()).unwrap_or(u64::MAX);
                let body = virtual_list_in(
                    &state,
                    n,
                    |i| i,
                    |i| {
                        let i = usize::try_from(i).unwrap_or(usize::MAX);
                        list.virtual_option(
                            div().named("option").height(30.0).child(text(format!("option {i}"))),
                            &*options,
                            i,
                        )
                        .into_element()
                    },
                    |v| v,
                )
                .height(300.0)
                .width(300.0);
                div()
                    .child(div().named("before").size(20.0).focusable())
                    .child(list.virtual_content(body))
                    .into_element()
            }
        })
        .into_element()
    });
    (s, held.get().expect("the listbox"))
}

#[test]
fn a_virtual_listbox_of_a_million_options_walks_its_data() {
    const N: usize = 1_000_000;
    let value: Signal<Option<usize>> = Signal::new(None);
    let (mut s, list) = virtual_listbox(value, false);
    let hl = || gui::untracked(|| list.core().highlighted());
    let o = spot(&s, "option", 2);
    s.click(o.0, o.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), Some(2), "a press chooses what it pressed");
    assert_eq!(hl(), Some(2), "and highlights it, not the first");
    assert!(visible(&s, "option") < 60, "a few rows built");
    let last = N - 3;
    plain(&mut s, NamedKey::End);
    assert_eq!(hl(), Some(last), "the last usable option, never built before");
    assert!(s.locate(&format!("option {last}")).is_ok(), "brought into view");
    plain(&mut s, NamedKey::PageUp);
    assert_eq!(hl(), Some(0));
    plain(&mut s, NamedKey::End);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), Some(last));
}

#[test]
fn hovering_a_virtual_option_keeps_the_highlight_read_in_o_1() {
    let value: Signal<Option<usize>> = Signal::new(None);
    let (mut s, list) = virtual_listbox(value, true);
    let hl = || gui::untracked(|| list.core().highlighted());
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::End);
    // Far from where the highlight was found (the end), so a highlight that
    // lost its index would be looked for over the whole data.
    for row in [999_990, 999_992, 999_993] {
        let o = s.locate(&format!("option {row}")).expect("in view");
        VALUES_READ.set(0);
        s.hover(o.0, o.1);
        s.settle().expect("settled");
        assert_eq!(hl(), Some(row), "the pointer highlights the row it is on");
        assert!(
            VALUES_READ.get() < 1000,
            "the highlight keeps the row's index: {} values read",
            VALUES_READ.get()
        );
    }
}

#[test]
fn a_virtual_listbox_keeps_the_listbox_guarantees_over_its_data() {
    let value: Signal<Option<usize>> = Signal::new(Some(500_000));
    let (mut s, list) = virtual_listbox(value, false);
    let hl = || gui::untracked(|| list.core().highlighted());
    // Mounting highlights the chosen option, never built, and brings it
    // into the list's own view; focus does not move.
    assert_eq!(hl(), Some(500_000), "the mount highlight reads the data");
    assert!(s.locate("option 500000").is_ok(), "brought into view");
    assert_eq!(s.read(gui::focus::focused), None, "no focus moved");
    // A caller's value: highlighted, brought into view, focused.
    s.act("write", || value.set(Some(900_000)));
    s.settle().expect("settled");
    assert_eq!(hl(), Some(900_000));
    assert!(s.locate("option 900000").is_ok(), "brought into view");
    let content = s.read(gui::focus::focused).expect("focus came to the list");
    // Leaving lets the highlight go and remembers it; Tab back restores it.
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(hl(), Some(900_001));
    shift(&mut s, NamedKey::Tab);
    assert_ne!(s.read(gui::focus::focused), Some(content), "focus left");
    assert_eq!(hl(), None, "focus left: nothing highlighted");
    plain(&mut s, NamedKey::Tab);
    assert_eq!(s.read(gui::focus::focused), Some(content));
    assert_eq!(hl(), Some(900_001), "Tab back: the option focus left");
    // A caller emptying the value: the first usable option, at the top.
    s.act("clear", || value.set(None));
    s.settle().expect("settled");
    assert_eq!(hl(), Some(0), "nothing chosen: the first usable one");
    assert!(s.locate("option 0").is_ok(), "brought into view");
}

#[test]
fn a_combobox_closes_as_focus_leaves_it() {
    let value = Signal::new(None);
    let open = Signal::new(false);
    let mut s = scenario(move || combobox_ui(value, open));
    let c = combo();
    s.act("focus", || c.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "a");
    assert!(open.get_untracked());
    // A press in the list or the field keeps it open.
    let f = spot(&s, "field", 0);
    s.click(f.0, f.1);
    s.settle().expect("settled");
    assert!(open.get_untracked(), "the root is inside");
    // Focus moving out closes it.
    plain(&mut s, NamedKey::Tab);
    s.settle().expect("settled");
    assert!(!open.get_untracked(), "focus left the root and the list");
}

#[test]
fn a_combobox_popup_holding_focus_gives_it_to_the_trigger_as_it_goes() {
    let value: Signal<Option<String>> = Signal::new(None);
    let open = Signal::new(false);
    let held: Held<Combobox<String, Option<String>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let field = TextField::new("");
            let combobox: Combobox<String, Option<String>> =
                Combobox::new(Bound::new(value), Bound::new(open), field, ComboboxConfig::REKA, |_| {
                    String::new()
                });
            h.set(Some(combobox));
            combobox.set_source(Rc::new(move || {
                let shown: Rc<dyn Options<String>> = Rc::new(Shown::under(&combobox.term()));
                shown
            }));
            move || {
                let open = combobox.is_open();
                // The field is inside the list, a button opens it (shadcn's).
                let list = open.then(|| {
                    let shown = Shown::under(&combobox.term());
                    deferred(
                        Positioning::Anchored {
                            side: Side::Bottom,
                            align: SideAlign::Start,
                            offset: 4.0,
                            match_width: false,
                            fit_height: false,
                        },
                        combobox
                            .popup(div().named("popup"))
                            .child(
                                combobox
                                    .input(text_input(&field).named("field").width(200.0).height(30.0), open)
                                    .on_key_down(move |ev| combobox.key(ev, false)),
                            )
                            .child(combobox.content(div()).children(shown.0.iter().enumerate().map(
                                |(i, (v, l, d))| {
                                    combobox.option(
                                        div().named("option").height(30.0).child(text(*l)),
                                        i,
                                        &(*v).to_owned(),
                                        Availability::from_disabled(*d),
                                        false,
                                    )
                                },
                            ))),
                    )
                });
                combobox
                    .button(div().named("button").width(100.0).height(30.0), open)
                    .child(list)
                    .into_element()
            }
        })
        .into_element()
    });
    let c = held.get().expect("the combobox");
    let b = spot(&s, "button", 0);
    s.click(b.0, b.1);
    s.settle().expect("settled");
    assert!(open.get_untracked());
    assert!(focused(&mut s, c.field().focus), "opening focuses the field");
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked().as_deref(), Some("banana"));
    assert!(!open.get_untracked());
    assert!(
        focused(&mut s, c.trigger_focus()),
        "the popup went with focus: the trigger has it"
    );
}

/// A tree of `roots` roots of `kids` leaves each, every root open, and a
/// count of the times its children were asked.
fn wide_tree(roots: usize, kids: usize, asked: Rc<Cell<usize>>) -> TreeData<(usize, usize), (usize, usize)> {
    TreeData {
        roots: Rc::new(move || (0..roots).map(|r| (r, usize::MAX)).collect()),
        children: Rc::new(move |n: &(usize, usize)| {
            // The live reads: a cold check computes every computed again.
            if !gui::component::is_cold_render() {
                asked.set(asked.get() + 1);
            }
            (n.1 == usize::MAX).then(|| (0..kids).map(|k| (n.0, k)).collect())
        }),
        key: Rc::new(|n: &(usize, usize)| *n),
        label: Rc::new(|n: &(usize, usize)| format!("{}.{}", n.0, n.1)),
        availability: Rc::new(|_: &(usize, usize)| Availability::Enabled),
    }
}

#[test]
fn a_tree_of_a_million_rows_reads_its_rows_once() {
    type Key = (usize, usize);
    let asked = Rc::new(Cell::new(0_usize));
    let value: Signal<Vec<Key>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Key>> = Signal::new((0..1000).map(|r| (r, usize::MAX)).collect());
    let held: Held<Tree<Key, Key, Vec<Key>>> = Rc::default();
    let (h, a) = (Rc::clone(&held), Rc::clone(&asked));
    let mut s = scenario(move || {
        let (h, a) = (Rc::clone(&h), Rc::clone(&a));
        component((), move |_| {
            let tree: Tree<Key, Key, Vec<Key>> = Tree::new(
                wide_tree(1000, 999, Rc::clone(&a)),
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            let state = gui::list::ListState::new(24.0);
            tree.attach_list(&state);
            move || {
                let rows = tree.rows();
                tree.virtual_list(&state, &rows, |r, _| {
                    div().named("row").height(24.0).child(text(r.label.clone()))
                })
                .height(480.0)
                .width(300.0)
                .into_element()
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    assert_eq!(gui::untracked(|| tree.rows().len()), 1_000_000);
    assert!(visible(&s, "row") < 60, "a few rows built");
    plain(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&(0, usize::MAX))));
    let walked = asked.get();
    plain(&mut s, NamedKey::ArrowDown);
    plain(&mut s, NamedKey::End);
    assert_eq!(asked.get(), walked, "keys read the rows, never the data");
    assert!(gui::untracked(|| tree.is_highlighted(&(999, 998))), "End: the last row");
    assert!(s.locate("999.998").is_ok(), "brought into view");
    let row = gui::untracked(|| tree.rows().index_of(&(999, 998))).expect("the row");
    assert_eq!(row, 999_999);
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), [(999, 998)]);
    assert_eq!(asked.get(), walked, "choosing reads no data");
}

#[test]
fn a_tree_of_any_depth_lays_out() {
    // A chain 200 000 deep, open all the way: no recursion to overflow.
    const DEPTH: usize = 200_000;
    let data = TreeData {
        roots: Rc::new(|| vec![0_usize]),
        children: Rc::new(|n: &usize| (*n + 1 < DEPTH).then(|| vec![*n + 1])),
        key: Rc::new(|n: &usize| *n),
        label: Rc::new(|n: &usize| n.to_string()),
        availability: Rc::new(|_: &usize| Availability::Enabled),
    };
    let expanded: Signal<Vec<usize>> = Signal::new((0..DEPTH).collect());
    let value: Signal<Vec<usize>> = Signal::new(Vec::new());
    let mut s = scenario(|| div().into_element());
    s.act("tree", || {
        let tree: Tree<usize, usize, Vec<usize>> = Tree::new(
            data,
            Bound::new(value),
            Bound::new(expanded),
            TreeConfig {
                behavior: primitives::SelectionBehavior::Toggle,
                propagate: true,
            },
            Direction::Ltr.into(),
        );
        let rows = gui::untracked(|| tree.rows());
        assert_eq!(rows.len(), DEPTH);
        assert_eq!(rows.get(DEPTH - 1).map(|r| r.level), Some(DEPTH));
        value.set(vec![DEPTH - 1]);
        let mixed = rows.get(0).map(|r| gui::untracked(|| tree.state_of(r)));
        assert_eq!(mixed, Some(gui::Toggled::Mixed), "the root is half chosen");
    });
}

#[test]
fn a_command_of_a_million_items_filters_off_the_ui_thread() {
    const N: usize = 1_000_000;
    let picked: Rc<RefCell<Vec<String>>> = Rc::default();
    let (mut s, cmd, search) = command_scenario_of(Rc::clone(&picked), palette_of(N));
    s.act("focus", || search.focus.focus());
    s.settle().expect("settled");
    assert!(visible(&s, "item") < 60, "a few rows built");
    plain(&mut s, NamedKey::End);
    assert_eq!(cmd.highlighted(), Some(format!("number {}", N - 1)));
    typed(&mut s, "number 99999");
    // The rows come from their thread; until then the list shows what it did.
    let typed_at = std::time::Instant::now();
    assert_ne!(
        cmd.highlighted().as_deref(),
        Some("number 99999"),
        "not filtered in the event"
    );
    // The worker runs in real time, not the scenario's clock.
    while cmd.highlighted().as_deref() != Some("number 99999") && typed_at.elapsed() < Duration::from_secs(60) {
        std::thread::sleep(Duration::from_millis(5));
        s.settle().expect("settled");
    }
    eprintln!("a million items filtered off the UI thread in {:?}", typed_at.elapsed());
    assert_eq!(cmd.highlighted().as_deref(), Some("number 99999"));
    plain(&mut s, NamedKey::End);
    assert_eq!(cmd.highlighted().as_deref(), Some("number 999999"));
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*picked.borrow(), ["number 999999"]);
}

#[test]
fn a_tags_input_that_cannot_be_used_adds_nothing() {
    let tags = Signal::new(Vec::new());
    let (mut s, input) = tags_scenario(
        tags,
        TagsConfig {
            delimiter: primitives::tags_input::Delimiter::Pattern(regex::Regex::new("[;|]").expect("a pattern")),
            add_on_blur: true,
            ..TagsConfig::default()
        },
    );
    s.act("focus", || input.field().focus.focus());
    s.settle().expect("settled");
    typed(&mut s, "a;");
    assert_eq!(tags.get_untracked(), ["a"], "a pattern delimits");
    s.act("unavailable", || input.set_availability(Availability::Unavailable));
    typed(&mut s, "b");
    plain(&mut s, NamedKey::Enter);
    typed(&mut s, "|");
    s.act("blur", || gui::focus::set_focus(None, false));
    s.settle().expect("settled");
    assert_eq!(
        tags.get_untracked(),
        ["a"],
        "neither Enter, the delimiter nor blur adds"
    );
}

#[test]
fn a_virtual_tree_scrolled_through_keeps_focus_for_the_rows_it_shows() {
    type Key = (usize, usize);
    let value: Signal<Vec<Key>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Key>> = Signal::new((0..100).map(|r| (r, usize::MAX)).collect());
    let roots = Signal::new(100_usize);
    let held: Held<Tree<Key, Key, Vec<Key>>> = Rc::default();
    let list: Rc<Cell<Option<gui::list::ListState>>> = Rc::default();
    let (h, l) = (Rc::clone(&held), Rc::clone(&list));
    let mut s = scenario(move || {
        let (h, l) = (Rc::clone(&h), Rc::clone(&l));
        component((), move |_| {
            let mut data = wide_tree(100, 99, Rc::new(Cell::new(0)));
            data.roots = Rc::new(move || (0..roots.get()).map(|r| (r, usize::MAX)).collect());
            let tree: Tree<Key, Key, Vec<Key>> = Tree::new(
                data,
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            let state = gui::list::ListState::new(24.0);
            tree.attach_list(&state);
            l.set(Some(state));
            move || {
                let rows = tree.rows();
                tree.virtual_list(&state, &rows, |r, _| {
                    div().named("row").height(24.0).child(text(r.label.clone()))
                })
                .height(480.0)
                .width(300.0)
                .into_element()
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    let state = list.take().expect("the list");
    assert_eq!(gui::untracked(|| tree.rows().len()), 10_000);
    // Focus the first row: it is the stop, and stays kept wherever the
    // list scrolls.
    plain(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&(0, usize::MAX))));
    let shown = visible(&s, "row");
    let mut most = 0;
    for step in 0..40_u64 {
        s.act("scroll", || state.scroll_to_row(step * 250));
        s.settle().expect("settled");
        s.settle().expect("settled");
        most = most.max(tree.kept_focus());
    }
    assert!(
        most <= visible(&s, "row").max(shown) + 2,
        "focus kept for {most} rows, {shown} shown"
    );
    // The focused row scrolled away is kept: the keys still reach it.
    plain(&mut s, NamedKey::ArrowDown);
    assert!(gui::untracked(|| tree.is_highlighted(&(0, 0))));
    // Nodes leaving the data let go of theirs.
    s.act("scroll home", || state.scroll_to_row(0));
    s.settle().expect("settled");
    s.act("fewer roots", || roots.set(1));
    s.settle().expect("settled");
    s.settle().expect("settled");
    assert!(tree.kept_focus() <= 101, "kept {}", tree.kept_focus());
}

/// The parent's half of a forwarded-callback test: a timer the callback
/// starts, and the child it hides. Made in the parent's setup.
fn parent_timer(log: &Rc<RefCell<Vec<String>>>, shown: Signal<bool>) -> impl Fn() + Clone + 'static {
    let log = Rc::clone(log);
    move || {
        let log = Rc::clone(&log);
        gui::clock::after(Duration::from_millis(100), move || {
            log.borrow_mut().push("parent timer".to_owned())
        });
        shown.set(false);
    }
}

/// A child, 100 x 30 at the top left, whose click runs `act`: the element
/// and its handler are the child's, so whatever runs as the child's goes
/// when it unmounts.
fn clicking_child(act: impl Fn() + 'static) -> Element {
    let act = Rc::new(act);
    component((), move |_| {
        let act = Rc::clone(&act);
        move || {
            let act = Rc::clone(&act);
            div().width(100.0).height(30.0).on_click(move |_| act()).into_element()
        }
    })
    .into_element()
}

#[test]
fn a_commands_pick_runs_as_the_root_that_gave_it_and_outlives_the_row_that_picked() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let l = Rc::clone(&l);
        component((), move |_| {
            let shown = Signal::new(true);
            let search = TextField::new("");
            let started = parent_timer(&l, shown);
            // The selection prop, given in the root's setup.
            let cmd = Command::new(
                search,
                move || search.edit.with(|e| e.text().to_owned()),
                Command::listbox(),
                |_: &CommandItem| false,
                move |_: CommandItem| started(),
            );
            let sections = Arc::new(vec![CommandSection::group(None, [CommandItem::new("Apple")])]);
            let list = cmd.list_setup(move || (Arc::clone(&sections), Arc::default()), 30.0, None);
            move || {
                let list = list.clone();
                div()
                    .width(400.0)
                    .height(100.0)
                    .child(if shown.get() {
                        // The row is a child's: its click picks.
                        clicking_child(move || cmd.pick(&list, &CommandItem::new("Apple")))
                    } else {
                        div().into_element()
                    })
                    .into_element()
            }
        })
        .into_element()
    });
    s.click(20.0, 15.0);
    s.settle().expect("settled");
    s.wait(Duration::from_millis(200));
    s.settle().expect("settled");
    assert_eq!(*log.borrow(), ["parent timer"]);
}

#[test]
fn a_tags_inputs_on_add_runs_as_the_root_that_gave_it_and_outlives_the_part_that_added() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let l = Rc::clone(&l);
        component((), move |_| {
            let shown = Signal::new(true);
            let started = parent_timer(&l, shown);
            let tags = Signal::new(Vec::<String>::new());
            let input = TagsInput::strings(
                Bound::new(tags),
                TextField::new(""),
                TagsConfig::default(),
                Direction::Ltr.into(),
            );
            // The notification prop, given in the root's setup.
            input.on_add(move |_: String| started());
            move || {
                div()
                    .width(400.0)
                    .height(100.0)
                    .child(if shown.get() {
                        // The part that adds is a child's.
                        clicking_child(move || input.paste("rust"))
                    } else {
                        div().into_element()
                    })
                    .into_element()
            }
        })
        .into_element()
    });
    s.click(20.0, 15.0);
    s.settle().expect("settled");
    s.wait(Duration::from_millis(200));
    s.settle().expect("settled");
    assert_eq!(*log.borrow(), ["parent timer"]);
}

/// The list part replaced in one update (keyed anew): the new part gives
/// its options before the old one leaves, and the old one's leaving takes
/// back only its own, so the combobox still has options.
#[test]
fn a_combobox_list_part_replaced_in_one_update_keeps_its_options() {
    let value: Signal<Option<String>> = Signal::new(None);
    let open = Signal::new(false);
    let which = Signal::new(false);
    let held: Held<Combobox<String, Option<String>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let field = TextField::new("");
            let combobox: Combobox<String, Option<String>> = Combobox::new(
                Bound::new(value),
                Bound::new(open),
                field,
                ComboboxConfig::REKA,
                |v: &Option<String>| v.clone().unwrap_or_default(),
            );
            h.set(Some(combobox));
            // One source both parts give: the old part's leaving must not
            // take for its own what the new part holds.
            let source: Rc<dyn Fn() -> Rc<dyn Options<String>>> = Rc::new(move || {
                let shown: Rc<dyn Options<String>> = Rc::new(Shown::under(&combobox.term()));
                shown
            });
            let part = move || {
                let source = Rc::clone(&source);
                component((), move |_| {
                    combobox.set_source(Rc::clone(&source));
                    || div().into_element()
                })
                .into_element()
            };
            move || {
                div()
                    .child(combobox.input(text_input(&field).named("field").width(200.0).height(30.0), false))
                    .child(
                        div()
                            .key(gui::key_of(if which.get() { "second" } else { "first" }))
                            .child(part()),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let c = held.get().expect("the combobox");
    assert!(c.options().is_some());
    s.act("replace the list part", || which.set(true));
    s.settle().expect("settled");
    assert!(c.options().is_some(), "the new part's options are the combobox's");
}

/// A virtual list replaced in one update (keyed anew), over the same list
/// state and data: the new one holds them from its mount, and the old
/// one's leaving lets go only of what it still holds, so the keys still
/// walk the whole data.
#[test]
fn a_virtual_listbox_list_replaced_in_one_update_keeps_its_data() {
    use primitives::listbox::Listbox;
    const N: usize = 1_000_000;
    let value: Signal<Option<usize>> = Signal::new(None);
    let which = Signal::new(false);
    let held: Held<Listbox<usize, Option<usize>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let list: Listbox<usize, Option<usize>> = Listbox::new(Bound::new(value), ListConfig::LISTBOX);
            h.set(Some(list));
            // One list state and one source both lists give.
            let state = gui::list::ListState::new(30.0);
            let source: Rc<dyn Fn() -> Rc<dyn Options<usize>>> = Rc::new(million);
            let part = move || {
                let source = Rc::clone(&source);
                component((), move |_| {
                    list.set_virtual(state, Rc::clone(&source));
                    move || {
                        let options = million();
                        let n = u64::try_from(options.len()).unwrap_or(u64::MAX);
                        let body = virtual_list_in(
                            &state,
                            n,
                            |i| i,
                            |i| {
                                let i = usize::try_from(i).unwrap_or(usize::MAX);
                                list.virtual_option(
                                    div().named("option").height(30.0).child(text(format!("option {i}"))),
                                    &*options,
                                    i,
                                )
                                .into_element()
                            },
                            |v| v,
                        )
                        .height(300.0)
                        .width(300.0);
                        list.virtual_content(body).into_element()
                    }
                })
                .into_element()
            };
            move || {
                div()
                    .child(
                        div()
                            .key(gui::key_of(if which.get() { "second" } else { "first" }))
                            .child(part()),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let list = held.get().expect("the listbox");
    s.act("replace the list", || which.set(true));
    s.settle().expect("settled");
    let o = spot(&s, "option", 2);
    s.click(o.0, o.1);
    s.settle().expect("settled");
    plain(&mut s, NamedKey::End);
    assert_eq!(
        gui::untracked(|| list.core().highlighted()),
        Some(N - 3),
        "the last usable option of the new list's data"
    );
}

/// A list state a virtual listbox used, reused by a plain virtual list once
/// the listbox left: the listbox let go of what it gave the state as it
/// unmounted, so the plain list never calls into a listbox that is gone.
#[test]
fn a_list_state_reused_after_its_listbox_left_calls_nothing_of_it() {
    use primitives::listbox::Listbox;
    let value: Signal<Option<usize>> = Signal::new(None);
    let boxed = Signal::new(true);
    let state = gui::list::ListState::new(30.0);
    let st = state;
    let mut s = scenario(move || {
        let state = st;
        if boxed.get() {
            component((), move |_| {
                let list: Listbox<usize, Option<usize>> = Listbox::new(Bound::new(value), ListConfig::LISTBOX);
                list.set_virtual(state, Rc::new(million));
                move || {
                    let options = million();
                    let body = virtual_list_in(
                        &state,
                        1000,
                        |i| i,
                        |i| {
                            let i = usize::try_from(i).unwrap_or(usize::MAX);
                            list.virtual_option(div().named("option").height(30.0), &*options, i)
                                .into_element()
                        },
                        |v| v,
                    )
                    .height(300.0)
                    .width(300.0);
                    list.virtual_content(body).into_element()
                }
            })
            .into_element()
        } else {
            virtual_list_in(
                &state,
                1000,
                |i| i,
                |_| div().named("row").height(30.0).into_element(),
                |v| v,
            )
            .height(300.0)
            .width(300.0)
            .into_element()
        }
    });
    let o = spot(&s, "option", 2);
    s.click(o.0, o.1);
    s.settle().expect("settled");
    s.act("the listbox leaves", || boxed.set(false));
    s.settle().expect("settled");
    assert!(visible(&s, "row") > 0, "the plain list shows its rows");
}

/// An option the listbox highlights, replaced in one update by one of the
/// same value (keyed anew): the old one's leaving is told by the option,
/// and the option of that value laid out after it holds the highlight, so
/// it stays; the option going for good lets the highlight go.
#[test]
fn an_option_replaced_by_one_of_the_same_value_keeps_the_highlight() {
    use primitives::listbox::Listbox;
    let value: Signal<Option<String>> = Signal::new(None);
    let which = Signal::new(Some(false));
    let held: Held<Listbox<String, Option<String>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let list: Listbox<String, Option<String>> = Listbox::new(Bound::new(value), ListConfig::LISTBOX);
            h.set(Some(list));
            let other = FocusHandle::new();
            move || {
                let apple = which.get().map(|second| {
                    div()
                        .key(gui::key_of(if second { "second" } else { "first" }))
                        .child(component((), move |_| {
                            let focus = FocusHandle::new();
                            gui::on_unmounted(move || list.option_unmounted(&"apple".to_owned()));
                            move || {
                                list.item(
                                    div().named("option").height(30.0).width(200.0).child(text("Apple")),
                                    "apple".to_owned(),
                                    Rc::from("Apple"),
                                    focus,
                                    Availability::Enabled,
                                )
                                .into_element()
                            }
                        }))
                        .into_element()
                });
                let pear = list.item(
                    div().named("option").height(30.0).width(200.0).child(text("Pear")),
                    "pear".to_owned(),
                    Rc::from("Pear"),
                    other,
                    Availability::Enabled,
                );
                let content = list.content(div().named("list").width(200.0).child(apple).child(pear));
                div().child(list.root(content)).into_element()
            }
        })
        .into_element()
    });
    let list = held.get().expect("the listbox");
    let o = spot(&s, "option", 0);
    s.click(o.0, o.1);
    s.settle().expect("settled");
    let apple = || gui::untracked(|| list.is_highlighted(&"apple".to_owned()));
    assert!(apple(), "the pressed option is highlighted");
    s.act("replace it by one of the same value", || which.set(Some(true)));
    s.settle().expect("settled");
    assert!(apple(), "the highlight stays on the value an option still holds");
    s.act("it goes", || which.set(None));
    s.settle().expect("settled");
    assert!(!apple(), "no option holds it: the highlight goes");
}

#[test]
fn tree_rows_that_come_and_go_leave_nothing_behind() {
    type Key = (usize, usize);
    let first = Signal::new(0_usize);
    let value: Signal<Vec<Key>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Key>> = Signal::new(Vec::new());
    let held: Held<Tree<Key, Key, Vec<Key>>> = Rc::default();
    let h = Rc::clone(&held);
    let mut s = scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let mut data = wide_tree(0, 0, Rc::new(Cell::new(0)));
            data.roots = Rc::new(move || {
                let from = first.get();
                (from..from + 3).map(|r| (r, usize::MAX)).collect()
            });
            let tree: Tree<Key, Key, Vec<Key>> = Tree::new(
                data,
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            move || {
                let rows = tree.rows();
                let built: Vec<_> = rows
                    .iter()
                    .enumerate()
                    .map(|(i, r)| tree.row(div().named("row").height(24.0).child(text(r.label.clone())), &rows, i))
                    .collect();
                tree.root(div().named("tree")).children(built).into_element()
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    let mut most = 0;
    for n in 1..=1000 {
        s.act("rows come and go", || first.set(n));
        s.settle().expect("settled");
        most = most.max(tree.kept_focus());
    }
    assert!(most <= 3, "kept a focus for {most} rows, 3 shown");
    // The rows there are now are focused through their own keys.
    plain(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&(1000, usize::MAX))));
    plain(&mut s, NamedKey::ArrowDown);
    assert!(gui::untracked(|| tree.is_highlighted(&(1001, usize::MAX))));
}

/// A tree laid on a virtual list, then on plain rows: the list's part left
/// with it, so the keys move focus through the plain rows and never ask the
/// old list to scroll.
#[test]
fn a_tree_whose_virtual_list_left_keeps_its_keys_off_that_list() {
    type Key = (usize, usize);
    let value: Signal<Vec<Key>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Key>> = Signal::new(Vec::new());
    let virtual_rows = Signal::new(true);
    let held: Held<Tree<Key, Key, Vec<Key>>> = Rc::default();
    let list: Rc<Cell<Option<gui::list::ListState>>> = Rc::default();
    let (h, l) = (Rc::clone(&held), Rc::clone(&list));
    let mut s = scenario(move || {
        let (h, l) = (Rc::clone(&h), Rc::clone(&l));
        component((), move |_| {
            let tree: Tree<Key, Key, Vec<Key>> = Tree::new(
                wide_tree(100, 0, Rc::new(Cell::new(0))),
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            let l = Rc::clone(&l);
            move || {
                if virtual_rows.get() {
                    let l = Rc::clone(&l);
                    component((), move |_| {
                        let state = gui::list::ListState::new(24.0);
                        tree.attach_list(&state);
                        l.set(Some(state));
                        move || {
                            let rows = tree.rows();
                            tree.virtual_list(&state, &rows, |r, _| {
                                div().named("row").height(24.0).child(text(r.label.clone()))
                            })
                            .height(240.0)
                            .width(300.0)
                            .into_element()
                        }
                    })
                    .into_element()
                } else {
                    let rows = tree.rows();
                    let built: Vec<_> = (0..rows.len())
                        .filter_map(|i| rows.get(i).map(|r| (i, r)))
                        .map(|(i, r)| tree.row(div().named("row").height(4.0).child(text(r.label.clone())), &rows, i))
                        .collect();
                    tree.root(div().named("tree")).children(built).into_element()
                }
            }
        })
        .into_element()
    });
    let tree = held.get().expect("the tree");
    let old = list.take().expect("the virtual list");
    s.act("plain rows", || virtual_rows.set(false));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&(0, usize::MAX))));
    plain(&mut s, NamedKey::End);
    assert!(
        gui::untracked(|| tree.is_highlighted(&(99, usize::MAX))),
        "End reached the last row"
    );
    // The old list went with its component: the tree asking it to scroll
    // would have failed loudly (a list used after its owner ended).
    assert!(!old.is_alive(), "the old list ended with its component");
}

/// A key whose `Hash` writes the same bytes for every value: two of them
/// are equal only by their number.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Twin(u8);

impl std::hash::Hash for Twin {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        0_u8.hash(h);
    }
}

fn twin_tree(roots: Signal<Vec<u8>>, held: &Held<Tree<Twin, Twin, Vec<Twin>>>) -> Scenario {
    let h = Rc::clone(held);
    let value: Signal<Vec<Twin>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Twin>> = Signal::new(Vec::new());
    scenario(move || {
        let h = Rc::clone(&h);
        component((), move |_| {
            let data = TreeData {
                roots: Rc::new(move || roots.get().into_iter().map(Twin).collect()),
                children: Rc::new(|_: &Twin| None),
                key: Rc::new(Twin::clone),
                label: Rc::new(|n: &Twin| format!("row {}", n.0)),
                availability: Rc::new(|_: &Twin| Availability::Enabled),
            };
            let tree: Tree<Twin, Twin, Vec<Twin>> = Tree::new(
                data,
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            h.set(Some(tree));
            move || {
                let rows = tree.rows();
                let built: Vec<_> = (0..rows.len())
                    .filter_map(|i| rows.get(i).map(|r| (i, r)))
                    .map(|(i, r)| tree.row(div().named("row").height(24.0).child(text(r.label.clone())), &rows, i))
                    .collect();
                tree.root(div().named("tree")).children(built).into_element()
            }
        })
        .into_element()
    })
}

/// Two rows of a virtual tree whose keys hash alike, each holding a
/// component of the same type: each row's wrapper is keyed by its own
/// identity, so each component mounts and renders its row.
#[test]
fn virtual_rows_whose_keys_hash_alike_each_render_their_component() {
    let value: Signal<Vec<Twin>> = Signal::new(Vec::new());
    let expanded: Signal<Vec<Twin>> = Signal::new(Vec::new());
    let mut s = scenario(move || {
        component((), move |_| {
            let data = TreeData {
                roots: Rc::new(|| vec![Twin(1), Twin(2)]),
                children: Rc::new(|_: &Twin| None),
                key: Rc::new(Twin::clone),
                label: Rc::new(|n: &Twin| format!("row {}", n.0)),
                availability: Rc::new(|_: &Twin| Availability::Enabled),
            };
            let tree: Tree<Twin, Twin, Vec<Twin>> = Tree::new(
                data,
                Bound::new(value),
                Bound::new(expanded),
                TreeConfig::default(),
                Direction::Ltr.into(),
            );
            let state = gui::list::ListState::new(24.0);
            tree.attach_list(&state);
            move || {
                let rows = tree.rows();
                tree.virtual_list(&state, &rows, |r, _| {
                    let label = r.label.clone();
                    div().named("row").height(24.0).child(component((), move |_| {
                        move || text(format!("inner {label}")).into_element()
                    }))
                })
                .height(240.0)
                .width(300.0)
                .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    assert!(s.locate("inner row 1").is_ok(), "the first row's component rendered");
    assert!(s.locate("inner row 2").is_ok(), "the second row's component rendered");
}

/// Two rows whose keys hash alike but are not equal are focused through
/// handles of their own: the keys move focus from one to the other.
#[test]
fn rows_whose_keys_hash_alike_are_focused_apart() {
    let roots = Signal::new(vec![1_u8, 2]);
    let held: Held<Tree<Twin, Twin, Vec<Twin>>> = Rc::default();
    let mut s = twin_tree(roots, &held);
    let tree = held.get().expect("the tree");
    plain(&mut s, NamedKey::Tab);
    assert!(gui::untracked(|| tree.is_highlighted(&Twin(1))));
    let first = s.read(gui::focus::focused);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(
        gui::untracked(|| tree.is_highlighted(&Twin(2))),
        "the second row holds the highlight"
    );
    assert_ne!(s.read(gui::focus::focused), first, "focus moved to another handle");
}

/// A row's key leaves the data and comes back: the handle saved from the
/// row that left focuses nothing, not the row that came in its place.
#[test]
fn a_row_added_again_is_not_focused_through_the_handle_of_the_one_that_left() {
    let roots = Signal::new(vec![1_u8, 2, 3]);
    let held: Held<Tree<Twin, Twin, Vec<Twin>>> = Rc::default();
    let mut s = twin_tree(roots, &held);
    let tree = held.get().expect("the tree");
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(gui::untracked(|| tree.is_highlighted(&Twin(2))));
    let saved = s.read(gui::focus::focused).expect("a row focused");
    plain(&mut s, NamedKey::ArrowDown);
    s.act("row 2 leaves", || roots.set(vec![1, 3]));
    s.settle().expect("settled");
    s.act("row 2 comes back", || roots.set(vec![1, 2, 3]));
    s.settle().expect("settled");
    s.act("focus the saved handle", || gui::focus::set_focus(Some(saved), true));
    s.settle().expect("settled");
    assert_ne!(
        s.read(gui::focus::focused),
        Some(saved),
        "the old handle reaches nothing, not the row that came back"
    );
}
