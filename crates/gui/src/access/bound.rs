//! Bound attributes (K4b, ADR-0026's accessibility binding phase): every
//! attribute setter takes a value or a closure; a closure runs where its
//! node resolves, under a tracker of its own, and a change tells the
//! screen reader of that node alone, rendering and mounting nothing.

use std::cell::Cell;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use accesskit::{AriaCurrent, AutoComplete, HasPopup, Live, Node, NodeId, Role, TreeId, TreeUpdate};
use accesskit_consumer::{NodeRef as Held, Tree, TreeChangeHandler};

use super::NodeRef;
use crate::app::Scenario;
use crate::color::Color;
use crate::component::{StructureStats, each, structure_stats};
use crate::element::{Div, Element, ValueRange, div};
use crate::prelude::*;
use crate::reactive::Signal;

fn update(s: &mut Scenario) -> TreeUpdate {
    s.settle().unwrap();
    s.pipeline_mut().access_update(1.0, "page")
}

fn full(s: &mut Scenario) -> TreeUpdate {
    s.pipeline_mut().access_tree(1.0, "page")
}

fn by_id(u: &TreeUpdate) -> HashMap<NodeId, Node> {
    u.nodes.iter().cloned().collect()
}

struct Quiet;

impl TreeChangeHandler for Quiet {
    fn node_added(&mut self, _: &Held<'_>) {}
    fn node_updated(&mut self, _: &Held<'_>, _: &Held<'_>) {}
    fn focus_moved(&mut self, _: Option<&Held<'_>>, _: Option<&Held<'_>>) {}
    fn node_removed(&mut self, _: &Held<'_>) {}
}

/// Every node a screen reader holds, as AccessKit's consumer keeps it.
fn held(tree: &Tree) -> HashMap<NodeId, Node> {
    let state = tree.state();
    let mut out = HashMap::new();
    let mut stack = vec![NodeId(0)];
    while let Some(id) = stack.pop() {
        let node = state.node_by_tree_local_id(id, TreeId::ROOT).unwrap();
        stack.extend(node.data().children().iter().copied());
        out.insert(id, node.data().clone());
    }
    out
}

fn since(before: StructureStats) -> StructureStats {
    let now = structure_stats();
    StructureStats {
        commits: now.commits.saturating_sub(before.commits),
        mounts: now.mounts.saturating_sub(before.mounts),
        unmounts: now.unmounts.saturating_sub(before.unmounts),
        moves: now.moves.saturating_sub(before.moves),
    }
}

/// A window for a test that counts runs, mounts or allocations: a cold
/// check (`GUI_COLD_CHECK`) runs every binding again in a tree of its own,
/// so it is off here; parity with a cold build is its own test
/// ([`every_bound_attribute_matches_a_cold_build`]).
fn counting(render: impl Fn() -> Element + 'static, height: u32) -> Scenario {
    let mut s = Scenario::measuring(render, 200, height, 1.0).unwrap();
    s.check_cold(false);
    s
}

fn bump(c: &Cell<u32>) {
    c.set(c.get().saturating_add(1));
}

const ROWS: usize = 1000;

/// A listbox of [`ROWS`] options in an `each`, option `i` selected as
/// `flags[i]` says, bound; `renders` counts the root's renders, `views`
/// the rows' views run.
fn listbox(
    flags: &Rc<Vec<Signal<bool>>>,
    renders: &Rc<Cell<u32>>,
    views: &Rc<Cell<u32>>,
) -> impl Fn() -> Element + use<> {
    let (flags, renders, views) = (Rc::clone(flags), Rc::clone(renders), Rc::clone(views));
    move || {
        bump(&renders);
        let (flags, views) = (Rc::clone(&flags), Rc::clone(&views));
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .role(Role::ListBox)
            .child(each(
                || (0..ROWS).collect::<Vec<_>>(),
                |i| *i,
                move |row| {
                    bump(&views);
                    let flag = flags.get(row.item_untracked()).copied().unwrap();
                    div().height(1.0).role(Role::ListBoxOption).aria_selected(flag)
                },
            ))
            .into_element()
    }
}

fn flags() -> Rc<Vec<Signal<bool>>> {
    Rc::new((0..ROWS).map(|_| Signal::new(false)).collect())
}

/// The plan's acceptance case: `aria_selected` bound to a signal, the
/// signal written: nothing renders or mounts, one binding runs, nothing is
/// laid out or painted again, and the screen reader is sent that node
/// alone.
#[test]
fn a_bound_aria_selected_tells_its_node_alone_and_mounts_nothing() {
    let (flags, renders, views) = (flags(), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let mut s = counting(listbox(&flags, &renders, &views), 1100);
    let mut tree = Tree::new(update(&mut s), true);
    assert_eq!(views.get(), u32::try_from(ROWS).unwrap());
    let (rendered, viewed) = (renders.get(), views.get());
    let row = flags.get(500).copied().unwrap();
    for (k, on) in [true, false, true, true, false].into_iter().enumerate() {
        let before = structure_stats();
        s.take_reports();
        s.act("select", || row.set(on));
        let u = update(&mut s);
        let d = since(before);
        assert_eq!((d.mounts, d.unmounts, d.moves), (0, 0, 0), "flip {k}: nothing mounts");
        let now = (renders.get(), views.get());
        assert_eq!(now, (rendered, viewed), "flip {k}: nothing renders");
        let reports = s.take_reports();
        if k == 3 {
            // Written its own value again: nothing runs, nothing is sent.
            assert!(u.nodes.is_empty(), "{:?}", u.nodes);
            continue;
        }
        assert_eq!(u.nodes.len(), 1, "flip {k}: that node alone: {:?}", u.nodes);
        let (_, n) = u.nodes.first().unwrap();
        assert_eq!((n.role(), n.is_selected()), (Role::ListBoxOption, Some(on)));
        let last = reports.last().unwrap();
        assert_eq!(last.bindings, 1, "flip {k}: its binding alone runs");
        assert_eq!(last.visits.resolved, 1, "flip {k}: its node alone resolves");
        assert!(last.rendered.is_empty(), "flip {k}: {:?}", last.rendered);
        assert_eq!(last.layout.calls, 0, "flip {k}: nothing lays out");
        assert_eq!(last.paint.recorded, 0, "flip {k}: nothing paints");
        tree.update_and_process_changes(u, &mut Quiet);
        assert_eq!(held(&tree), by_id(&full(&mut s)));
    }
}

/// K4b's budget: one write flips [`ROWS`] bound `aria_selected`s at once:
/// each node is sent once, and what the frame allocates is per node
/// changed (its behaviour made anew, its AccessKit node), none of it the
/// bindings' own.
#[test]
fn a_thousand_bound_flags_flipped_at_once_send_a_thousand_nodes() {
    let (flags, renders, views) = (flags(), Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let mut s = counting(listbox(&flags, &renders, &views), 1100);
    update(&mut s);
    // Warm: the first flip grows what a frame keeps.
    for on in [true, false] {
        s.act("all", || flags.iter().for_each(|f| f.set(on)));
        update(&mut s);
    }
    let rendered = renders.get();
    s.take_reports();
    let before = structure_stats();
    s.act("all", || flags.iter().for_each(|f| f.set(true)));
    let ((), frame) = crate::heap::allocations(|| s.settle().unwrap());
    let (u, access) = crate::heap::allocations(|| s.pipeline_mut().access_update(1.0, "page"));
    let report = s.take_reports().pop().unwrap();
    assert_eq!(u.nodes.len(), ROWS, "each flipped node, once");
    assert!(u.nodes.iter().all(|(_, n)| n.is_selected() == Some(true)));
    assert_eq!(since(before).mounts, 0);
    assert_eq!(renders.get(), rendered);
    assert_eq!(report.bindings, u64::try_from(ROWS).unwrap());
    assert_eq!(report.visits.resolved, u64::try_from(ROWS).unwrap());
    assert_eq!(report.layout.calls, 0);
    assert_eq!(report.paint.recorded, 0);
    eprintln!(
        "K4b flip of {ROWS} bound aria_selected: sent {} nodes, bindings run {}, nodes resolved {}, \
         frame allocations {frame}, accessibility update allocations {access}",
        u.nodes.len(),
        report.bindings,
        report.visits.resolved,
    );
    // Per node changed: its behaviour (one allocation) and what the frame
    // and the window's log keep of it; none per binding that did not run.
    assert!(frame <= 4 * u64::try_from(ROWS).unwrap(), "frame allocations {frame}");
}

/// A bound attribute that does not change costs nothing: its node resolved
/// for another reason (its colour, bound) allocates what the same node with
/// the attribute fixed allocates, and runs no binding of it.
#[test]
fn an_unchanged_bound_attribute_allocates_nothing_when_its_node_resolves() {
    fn rows(lit: Signal<bool>, bound: bool) -> impl Fn() -> Element {
        move || {
            let flag = Signal::new(false);
            div()
                .display(Display::Flex)
                .flex_direction(FlexDirection::Column)
                .role(Role::ListBox)
                .children((0..ROWS).map(|i| {
                    let d = div()
                        .key(u64::try_from(i).unwrap())
                        .height(1.0)
                        .role(Role::ListBoxOption)
                        .background(move || if lit.get() { Color::WHITE } else { Color::BLACK });
                    if bound {
                        d.aria_selected(flag)
                    } else {
                        d.aria_selected(false)
                    }
                }))
                .into_element()
        }
    }
    let spent = |bound: bool| {
        let lit = Signal::new(false);
        let mut s = counting(rows(lit, bound), 1100);
        update(&mut s);
        for on in [true, false] {
            s.act("lit", || lit.set(on));
            update(&mut s);
        }
        s.take_reports();
        s.act("lit", || lit.set(true));
        let ((), spent) = crate::heap::allocations(|| {
            update(&mut s);
        });
        let report = s.take_reports().pop().unwrap();
        let rows = u64::try_from(ROWS).unwrap();
        assert_eq!(report.visits.resolved, rows, "every row resolves");
        assert_eq!(report.bindings, rows, "only the colours run");
        spent
    };
    let (fixed, bound) = (spent(false), spent(true));
    eprintln!("K4b {ROWS} rows resolved for their colour: allocations fixed {fixed}, bound {bound}");
    assert_eq!(bound, fixed, "a bound attribute that did not change allocates nothing");
}

/// Every attribute's value at phase `p`.
struct At {
    p: u32,
}

impl At {
    fn even(&self) -> bool {
        self.p.is_multiple_of(2)
    }

    /// The phase plus `plus`, as a count.
    fn n(&self, plus: usize) -> usize {
        usize::try_from(self.p).unwrap().saturating_add(plus)
    }

    /// `even` at an even phase, else `odd`.
    fn pick<T>(&self, even: T, odd: T) -> T {
        if self.even() { even } else { odd }
    }
}

/// A node with every attribute set as phase `p` gives it: each from a
/// closure reading `p` when `bound`, else as a value read now. `refs` are
/// the two nodes its active descendant points to.
fn every_attribute(p: Signal<u32>, bound: bool, refs: (NodeRef, NodeRef)) -> Element {
    macro_rules! set {
        ($d:expr, $m:ident, $f:expr) => {
            if bound {
                $d.$m(move || ($f)(&At { p: p.get() }))
            } else {
                $d.$m(($f)(&At { p: p.get_untracked() }))
            }
        };
    }
    let d: Div = div().width(100.0).height(40.0).focusable();
    let d = set!(d, role, |a: &At| a.pick(Role::Slider, Role::SpinButton));
    let d = set!(d, aria_label, |a: &At| format!("knob {}", a.p));
    let d = set!(d, aria_checked, |a: &At| a.even());
    let d = set!(d, aria_selected, |a: &At| !a.even());
    let d = set!(d, aria_expanded, |a: &At| a.p.is_multiple_of(3));
    let d = set!(d, aria_row_count, |a: &At| a.n(5));
    let d = set!(d, aria_row_index, |a: &At| a.n(0));
    let d = set!(d, aria_orientation, |a: &At| a.pick(Axis::Horizontal, Axis::Vertical));
    let d = set!(d, aria_value_range, |a: &At| ValueRange {
        now: Some(f64::from(a.p)),
        min: 0.0,
        max: 10.0,
        text: (a.p == 2).then(|| "two".to_owned()),
    });
    let d = set!(d, aria_level, |a: &At| a.n(1));
    let d = set!(d, aria_current, |a: &At| a.pick(AriaCurrent::Page, AriaCurrent::Step));
    let d = set!(d, aria_value, |a: &At| a.p.to_string());
    let d = set!(d, aria_role_description, |a: &At| format!("dial {}", a.p));
    let d = set!(d, aria_description, |a: &At| format!("about {}", a.p));
    let d = set!(d, aria_live, |a: &At| a.pick(Live::Polite, Live::Assertive));
    let d = set!(d, aria_multiselectable, |a: &At| a.p & 1 == 1);
    let d = set!(d, aria_required, |a: &At| a.p & 2 == 2);
    let d = set!(d, aria_invalid, |a: &At| a.p == 1);
    let d = set!(d, aria_readonly, |a: &At| a.p == 3);
    let d = set!(d, aria_has_popup, |a: &At| a.pick(HasPopup::Menu, HasPopup::Listbox));
    let (list, both) = (AutoComplete::List, AutoComplete::Both);
    let d = set!(d, aria_auto_complete, move |a: &At| a.pick(list, both));
    let d = set!(d, unavailable, |a: &At| a.p == 2);
    let d = set!(d, disabled, |a: &At| a.p == 3);
    let d = set!(d, pass_through_all, |a: &At| a.p == 1);
    let d = set!(d, tab_children_reversed, |a: &At| !a.even());
    let (one, two) = refs;
    let d = set!(d, active_descendant, move |a: &At| match a.p % 3 {
        0 => Some(one),
        1 => Some(two),
        _ => None,
    });
    let d = if bound {
        d.aria_pos_in_set(move || At { p: p.get() }.n(1), 4_usize)
    } else {
        d.aria_pos_in_set(At { p: p.get_untracked() }.n(1), 4_usize)
    };
    let inert = div().width(10.0).height(10.0).role(Role::Button).aria_label("inert");
    let inert = set!(inert, inert, |a: &At| a.even());
    let dormant = div().width(10.0).height(10.0).role(Role::Button).aria_label("dormant");
    let dormant = set!(dormant, dormant, |a: &At| a.p % 4 == 3);
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(
            d.child(div().width(10.0).height(10.0).role(Role::ListBoxOption).node_ref(one))
                .child(div().width(10.0).height(10.0).role(Role::ListBoxOption).node_ref(two)),
        )
        .child(inert)
        .child(dormant)
        .into_element()
}

/// What placing gave each node of the last frame to act on, beside what a
/// screen reader is told.
fn behaviours(s: &Scenario) -> Vec<(bool, bool, bool, bool, crate::element::Semantics)> {
    let f = s.last();
    (0..f.nodes.len())
        .filter_map(|i| f.behaviour(i))
        .map(|b| (b.disabled, b.inert, b.pass_through, b.tab_reversed, b.semantics.clone()))
        .collect()
}

/// Every attribute bound to one signal, written through four phases and
/// back: after each write the screen reader holds what a window built with
/// those values fixed holds, and the frame acts as that window's does;
/// the root never renders again. With `cold`, each frame is checked
/// against one built cold.
fn every_attribute_follows_its_binding(cold: bool) {
    let refs = (NodeRef::new(), NodeRef::new());
    let p = Signal::new(0_u32);
    let renders = Rc::new(Cell::new(0));
    let counted = Rc::clone(&renders);
    let mut s = Scenario::measuring(
        move || {
            // A cold check renders a root of its own.
            if !crate::component::is_cold_render() {
                bump(&counted);
            }
            every_attribute(p, true, refs)
        },
        200,
        200,
        1.0,
    )
    .unwrap();
    s.check_cold(cold);
    let mut tree = Tree::new(update(&mut s), true);
    for next in [1_u32, 2, 3, 0, 3, 1, 2] {
        let before = structure_stats();
        s.act("phase", || p.set(next));
        let u = update(&mut s);
        assert_eq!(since(before).mounts, 0);
        tree.update_and_process_changes(u, &mut Quiet);
        let fixed = Signal::new(next);
        let mut want = Scenario::measuring(move || every_attribute(fixed, false, refs), 200, 200, 1.0).unwrap();
        want.settle().unwrap();
        assert_eq!(held(&tree), by_id(&full(&mut want)), "phase {next}: what is told");
        assert_eq!(behaviours(&s), behaviours(&want), "phase {next}: how it acts");
    }
    assert_eq!(renders.get(), 1, "the root rendered once");
}

#[test]
fn every_attribute_bound_to_a_signal_follows_it() {
    every_attribute_follows_its_binding(false);
}

/// Cold-check parity: every frame of the flips equals one built cold.
#[test]
fn every_bound_attribute_matches_a_cold_build() {
    every_attribute_follows_its_binding(true);
}

/// A check builds its own tree: the bindings it runs run on its own
/// mounts, and leave the live node's as they were (ADR-0035): the live
/// frame after it sends and runs nothing, and the next write runs the
/// live binding once and tells that node alone.
#[test]
fn a_binding_read_inside_a_check_leaves_the_live_node_as_it_was() {
    let on = Signal::new(false);
    let runs = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&runs);
    let mut s = counting(
        move || {
            let counted = Rc::clone(&counted);
            div()
                .child(div().height(10.0).role(Role::Tab).aria_selected(move || {
                    bump(&counted);
                    on.get()
                }))
                .into_element()
        },
        100,
    );
    let mut tree = Tree::new(update(&mut s), true);
    assert_eq!(runs.get(), 1);
    s.check_now();
    assert_eq!(runs.get(), 2, "the check ran the binding on its own mount");
    s.take_reports();
    s.act("nothing", || {});
    let u = update(&mut s);
    assert!(u.nodes.is_empty(), "the check marked nothing live: {:?}", u.nodes);
    assert_eq!(runs.get(), 2, "the live mount did not run again");
    s.act("select", || on.set(true));
    let u = update(&mut s);
    assert_eq!(runs.get(), 3, "the live mount runs once");
    assert_eq!(u.nodes.len(), 1, "{:?}", u.nodes);
    assert_eq!(u.nodes.first().unwrap().1.is_selected(), Some(true));
    tree.update_and_process_changes(u, &mut Quiet);
    assert_eq!(held(&tree), by_id(&full(&mut s)));
    // And a check now agrees with the live node's new value.
    s.check_now();
}

/// The last setter of an attribute owns it: a value after a closure drops
/// the closure (a write to what it read runs nothing), a closure after a
/// value is bound.
#[test]
fn the_last_setter_of_an_attribute_owns_it() {
    let on = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(div().height(10.0).role(Role::Tab).aria_selected(on).aria_selected(true))
                .child(
                    div()
                        .height(10.0)
                        .role(Role::Tab)
                        .aria_expanded(false)
                        .aria_expanded(on),
                )
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .unwrap();
    let first = update(&mut s);
    let tabs = |u: &TreeUpdate| {
        let mut t: Vec<_> = u
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::Tab)
            .map(|(_, n)| (n.is_selected(), n.is_expanded()))
            .collect();
        t.sort_by_key(|(s, _)| s.is_some());
        t
    };
    assert_eq!(tabs(&first), [(None, Some(false)), (Some(true), None)]);
    s.take_reports();
    s.act("on", || on.set(true));
    let u = update(&mut s);
    assert_eq!(tabs(&u), [(None, Some(true))], "the bound one alone changes");
    assert_eq!(s.take_reports().last().unwrap().bindings, 1);
}

/// A bound attribute whose closure panics in a turn keeps what it gave
/// (the rest of the frame is made anew), a cold check reuses that, and it
/// runs again once what it read changes.
#[test]
fn a_bound_attribute_that_panics_keeps_what_it_gave() {
    let n = Signal::new(0_u32);
    let m = Signal::new(0_u32);
    let mut s = Scenario::measuring(
        move || {
            div()
                .child(div().height(10.0).role(Role::Button).aria_label(move || {
                    let v = n.get();
                    assert!(v != 1, "n is not 1");
                    format!("n {v}")
                }))
                .child(
                    div()
                        .height(10.0)
                        .role(Role::Button)
                        .aria_label(move || format!("m {}", m.get())),
                )
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    update(&mut s);
    let labels = |s: &mut Scenario| {
        let mut l: Vec<String> = full(s)
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == Role::Button)
            .filter_map(|(_, n)| n.label().map(str::to_owned))
            .collect();
        l.sort();
        l
    };
    s.act("both", || {
        n.set(1);
        m.set(1);
    });
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    assert_eq!(labels(&mut s), ["m 1", "n 0"]);
    // A frame checked cold reuses what the held binding gave.
    s.act("m", || m.set(2));
    s.settle().unwrap();
    assert_eq!(labels(&mut s), ["m 2", "n 0"]);
    s.act("n", || n.set(2));
    s.settle().unwrap();
    assert_eq!(labels(&mut s), ["m 2", "n 2"]);
}

/// `disabled` bound reaches the subtree: a child of a node disabled by
/// its binding is disabled and loses its Tab stop, and comes back with it.
#[test]
fn a_bound_disabled_reaches_the_subtree() {
    let off = Signal::new(false);
    let mut s = Scenario::measuring(
        move || {
            div()
                .disabled(off)
                .child(
                    div()
                        .height(10.0)
                        .role(Role::Button)
                        .aria_label("child")
                        .on_click(|_| {}),
                )
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    let child = |s: &mut Scenario| {
        full(s)
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("child"))
            .map(|(_, n)| n.is_disabled())
            .unwrap()
    };
    update(&mut s);
    assert!(!child(&mut s));
    for on in [true, false, true] {
        s.act("off", || off.set(on));
        update(&mut s);
        assert_eq!(child(&mut s), on);
    }
}

/// Whether the last frame shows a node named `name`.
fn shows(s: &Scenario, name: &str) -> bool {
    s.last().nodes.iter().any(|n| n.name == Some(name))
}

/// Review r1 (Opus I1, Codex 1): a node its own bound `disabled` hides
/// (its display reads its interaction's disabled flag) comes back when
/// the binding gives `false` again, as the same node with `disabled` fixed
/// does when its render gives `false`. Hiding hands over what every
/// binding of the node read, not only its style's.
#[test]
fn r9_hidden_by_own_bound_disabled_comes_back() {
    for bound in [false, true] {
        let off = Signal::new(false);
        let mut s = counting(
            move || {
                let i = Interaction::new();
                let d = div().named("probe").height(20.0).interaction(i);
                let d = if bound { d.disabled(off) } else { d.disabled(off.get()) };
                div()
                    .child(d.display(move || if i.disabled() { Display::None } else { Display::Flex }))
                    .into_element()
            },
            100,
        );
        s.settle().unwrap();
        assert!(shows(&s, "probe"));
        for v in [true, false, true, false] {
            s.act("disable", || off.set(v));
            s.settle().unwrap();
            assert_eq!(shows(&s, "probe"), !v, "bound={bound} disabled={v}");
        }
    }
}

/// Review r4 (Codex, K4b): siblings each hidden from their first frame by
/// their own bound `disabled` come back each on its own flag, and every
/// frame matches a cold one.
#[test]
fn r4_initially_hidden_siblings_wake_independently() {
    let flags = [Signal::new(true), Signal::new(true)];
    let handles = [Interaction::new(), Interaction::new()];
    let mut s = Scenario::measuring(
        move || {
            div()
                .children(flags.into_iter().zip(handles).zip(["A", "B"]).map(|((off, i), name)| {
                    div()
                        .named(name)
                        .height(20.0)
                        .disabled(off)
                        .interaction(i)
                        .display(move || if i.disabled() { Display::None } else { Display::Flex })
                }))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .unwrap();
    s.check_cold(true);
    for (a, b) in [(true, true), (false, true), (true, false), (false, false)] {
        s.act("flags", || {
            flags[0].set(a);
            flags[1].set(b);
        });
        s.settle().unwrap();
        assert_eq!(shows(&s, "A"), !a, "a={a} b={b}");
        assert_eq!(shows(&s, "B"), !b, "a={a} b={b}");
    }
}

/// A node hidden by its own style binding, whose content is bound, comes
/// back showing what its content binding gives now, whatever changed while
/// it was hidden: every kind of binding a node holds (style, attribute,
/// text) is handed over the same way.
#[test]
fn a_node_hidden_through_each_binding_kind_comes_back_current() {
    let (gone, label, words) = (Signal::new(false), Signal::new(1_u32), Signal::new(1_u32));
    let mut s = counting(
        move || {
            div()
                .child(
                    crate::element::text_bound(move || format!("w{}", words.get()))
                        .named("probe")
                        .aria_label(move || format!("l{}", label.get()))
                        .display(move || if gone.get() { Display::None } else { Display::Flex }),
                )
                .into_element()
        },
        100,
    );
    s.check_cold(true);
    s.settle().unwrap();
    s.act("hide", || gone.set(true));
    s.settle().unwrap();
    assert!(!shows(&s, "probe"));
    s.act("change while hidden", || {
        label.set(2);
        words.set(2);
    });
    s.settle().unwrap();
    s.act("show", || gone.set(false));
    let u = update(&mut s);
    assert!(shows(&s, "probe"));
    let (_, n) = full(&mut s)
        .nodes
        .into_iter()
        .find(|(_, n)| n.role() == Role::Label)
        .unwrap();
    assert_eq!((n.label(), n.value()), (Some("l2"), Some("w2")), "{:?}", u.nodes);
}

/// Review r1 (Opus I2): a render that gives a node another bound
/// attribute in the place of one it had does not hand the new one what
/// the old one gave: the new one panicking on its first run gives nothing.
#[test]
fn r5_a_panicking_new_binding_does_not_inherit_another_attributes_value() {
    let changed = Signal::new(false);
    let mut s = counting(
        move || {
            let d = div().height(10.0).role(Role::Tab);
            let d = if changed.get() {
                d.aria_expanded(|| -> bool { panic!("no expanded value") })
            } else {
                d.aria_selected(|| true)
            };
            div().child(d).into_element()
        },
        100,
    );
    update(&mut s);
    s.act("replace", || changed.set(true));
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    let tab = |s: &mut Scenario| {
        full(s)
            .nodes
            .into_iter()
            .find(|(_, n)| n.role() == Role::Tab)
            .map(|(_, n)| (n.is_selected(), n.is_expanded()))
            .unwrap()
    };
    assert_eq!(tab(&mut s), (None, None));
}

/// Review r1 (Codex 3): the same, the new attribute with a fixed value
/// set before its closure: a first run that panics leaves the fixed one.
#[test]
fn a_panicking_new_binding_leaves_its_own_fixed_value() {
    let changed = Signal::new(false);
    let mut s = counting(
        move || {
            let d = div().height(10.0).role(Role::Tab);
            let d = if changed.get() {
                d.aria_expanded(false)
                    .aria_expanded(|| -> bool { panic!("no expanded value") })
            } else {
                d.aria_selected(|| true)
            };
            div().child(d).into_element()
        },
        100,
    );
    update(&mut s);
    s.act("replace", || changed.set(true));
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    let (_, n) = full(&mut s)
        .nodes
        .into_iter()
        .find(|(_, n)| n.role() == Role::Tab)
        .unwrap();
    assert_eq!((n.is_selected(), n.is_expanded()), (None, Some(false)));
}

/// Review r1 (Opus I3, Codex 2): a virtual list's row whose role is
/// bound to a row role is the row, as one whose role is fixed is: it
/// carries its index, and no wrapper around it is told as a row.
#[test]
fn r8_virtual_row_with_bound_role_is_the_row() {
    for bound in [false, true] {
        let state = crate::list::ListState::new(24.0);
        let mut s = counting(
            move || {
                let row = move |_| {
                    let d = div().height(24.0).aria_label("real row");
                    let d = if bound { d.role(|| Role::Row) } else { d.role(Role::Row) };
                    d.into_element()
                };
                crate::list::virtual_list(&state, 20, |i| i, row)
                    .height(200.0)
                    .role(Role::Table)
                    .into_element()
            },
            300,
        );
        let u = update(&mut s);
        let rows = u.nodes.iter().filter(|(_, n)| n.role() == Role::Row).count();
        let (_, row) = u.nodes.iter().find(|(_, n)| n.label() == Some("real row")).unwrap();
        assert_eq!(row.row_index(), Some(0), "bound={bound}");
        let real = u.nodes.iter().filter(|(_, n)| n.label() == Some("real row")).count();
        let table = u.nodes.iter().find(|(_, n)| n.role() == Role::Table).unwrap();
        assert_eq!(table.1.row_count(), Some(20), "bound={bound}");
        assert_eq!(rows, real, "bound={bound}: no wrapper is a row of its own");
    }
}

/// The same row whose bound role moves off a row's and back: the index
/// follows the role as it resolves, and the list never renders again.
#[test]
fn a_virtual_rows_index_follows_its_bound_role() {
    let heading = Signal::new(false);
    let state = crate::list::ListState::new(24.0);
    let mut s = counting(
        move || {
            let row = move |_| {
                div()
                    .height(24.0)
                    .aria_label("row")
                    .role(move || if heading.get() { Role::Heading } else { Role::Row })
                    .into_element()
            };
            crate::list::virtual_list(&state, 3, |i| i, row)
                .height(100.0)
                .role(Role::Table)
                .into_element()
        },
        300,
    );
    s.check_cold(true);
    let indices = |s: &mut Scenario| {
        let mut v: Vec<_> = full(s)
            .nodes
            .into_iter()
            .filter(|(_, n)| n.label() == Some("row"))
            .map(|(_, n)| (n.role(), n.row_index()))
            .collect();
        v.sort_by_key(|(_, i)| *i);
        v
    };
    let rows = [(Role::Row, Some(0)), (Role::Row, Some(1)), (Role::Row, Some(2))];
    update(&mut s);
    assert_eq!(indices(&mut s), rows);
    s.act("heading", || heading.set(true));
    update(&mut s);
    assert_eq!(indices(&mut s), [(Role::Heading, None); 3]);
    s.act("row", || heading.set(false));
    update(&mut s);
    assert_eq!(indices(&mut s), rows);
}

/// Review r1 nit: a set place whose position closure panics on its first
/// run tells no place, not 0.
#[test]
fn a_set_place_is_told_once_both_parts_have_given() {
    let mut s = counting(
        move || {
            div()
                .child(
                    div()
                        .height(10.0)
                        .role(Role::ListBoxOption)
                        .aria_pos_in_set(|| -> usize { panic!("no place") }, 4_usize),
                )
                .into_element()
        },
        100,
    );
    let r = catch_unwind(AssertUnwindSafe(|| s.settle()));
    assert!(r.is_err(), "the panic reaches the caller");
    let (_, n) = full(&mut s)
        .nodes
        .into_iter()
        .find(|(_, n)| n.role() == Role::ListBoxOption)
        .unwrap();
    assert_eq!((n.position_in_set(), n.size_of_set()), (None, None));
}

/// Review r1 (Codex 4): `aria_hidden` and the relations take a closure
/// too. A relation's closure gives the whole list, in order; fixed calls
/// add up.
#[test]
fn aria_hidden_and_relations_follow_their_bindings() {
    let (a, b) = (NodeRef::new(), NodeRef::new());
    let (hide, swap) = (Signal::new(false), Signal::new(false));
    let mut s = counting(
        move || {
            div()
                .child(div().height(10.0).role(Role::Label).aria_label("a").node_ref(a))
                .child(div().height(10.0).role(Role::Label).aria_label("b").node_ref(b))
                .child(
                    div()
                        .height(10.0)
                        .role(Role::Button)
                        .aria_label("probe")
                        .described_by(a)
                        .described_by(b)
                        .labelled_by(move || if swap.get() { vec![b, a] } else { vec![a] })
                        .controls(move || b)
                        .aria_hidden(hide),
                )
                .into_element()
        },
        100,
    );
    s.check_cold(true);
    let ids = |s: &mut Scenario| {
        let u = full(s);
        let id = |label: &str| {
            u.nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map(|(id, _)| *id)
        };
        let probe = u.nodes.iter().find(|(_, n)| n.label() == Some("probe")).map(|(_, n)| {
            (
                n.labelled_by().to_vec(),
                n.described_by().to_vec(),
                n.controls().to_vec(),
            )
        });
        (id("a").unwrap(), id("b").unwrap(), probe)
    };
    update(&mut s);
    let (ia, ib, probe) = ids(&mut s);
    assert_eq!(probe, Some((vec![ia], vec![ia, ib], vec![ib])));
    s.act("swap", || swap.set(true));
    update(&mut s);
    assert_eq!(ids(&mut s).2, Some((vec![ib, ia], vec![ia, ib], vec![ib])));
    s.act("hide", || hide.set(true));
    update(&mut s);
    assert_eq!(ids(&mut s).2, None, "hidden from a screen reader");
    s.act("show", || hide.set(false));
    update(&mut s);
    assert!(ids(&mut s).2.is_some());
}

/// Every public attribute setter of a node, each of which takes a value
/// or a closure. [`every_attribute_setter_is_listed`] keeps this in step
/// with `element.rs`; [`every_attribute_setter_takes_a_closure`] gives
/// each one a closure.
const SETTERS: &[&str] = &[
    "tab_children_reversed",
    "disabled",
    "unavailable",
    "role",
    "aria_label",
    "aria_checked",
    "aria_selected",
    "aria_expanded",
    "aria_row_count",
    "aria_row_index",
    "aria_pos_in_set",
    "aria_orientation",
    "aria_value_range",
    "aria_level",
    "aria_current",
    "aria_value",
    "aria_role_description",
    "aria_description",
    "aria_live",
    "aria_multiselectable",
    "aria_required",
    "aria_invalid",
    "aria_readonly",
    "aria_has_popup",
    "aria_auto_complete",
    "aria_hidden",
    "pass_through_all",
    "inert",
    "dormant",
    "labelled_by",
    "described_by",
    "controls",
    "owns",
    "active_descendant",
];

/// Compiles only while every setter in [`SETTERS`] takes a closure.
#[test]
fn every_attribute_setter_takes_a_closure() {
    let r = NodeRef::new();
    let d: Div = div()
        .tab_children_reversed(|| true)
        .disabled(|| true)
        .unavailable(|| true)
        .role(|| Role::Button)
        .aria_label(|| "x".to_owned())
        .aria_checked(|| true)
        .aria_selected(|| true)
        .aria_expanded(|| true)
        .aria_row_count(|| 1_usize)
        .aria_row_index(|| 1_usize)
        .aria_pos_in_set(|| 1_usize, || 2_usize)
        .aria_orientation(|| Axis::Vertical)
        .aria_value_range(ValueRange::default)
        .aria_level(|| 1_usize)
        .aria_current(|| AriaCurrent::Page)
        .aria_value(|| "v".to_owned())
        .aria_role_description(|| "r".to_owned())
        .aria_description(|| "d".to_owned())
        .aria_live(|| Live::Polite)
        .aria_multiselectable(|| true)
        .aria_required(|| true)
        .aria_invalid(|| true)
        .aria_readonly(|| true)
        .aria_has_popup(|| HasPopup::Menu)
        .aria_auto_complete(|| AutoComplete::List)
        .aria_hidden(|| true)
        .pass_through_all(|| true)
        .inert(|| true)
        .dormant(|| true)
        .labelled_by(move || r)
        .described_by(move || vec![r])
        .controls(move || r)
        .owns(move || vec![r])
        .active_descendant(move || Some(r));
    let (_, bound) = d.common.attrs_to_resolve();
    let names: Vec<&str> = bound.iter().map(|op| op.name).collect();
    for setter in SETTERS {
        assert!(names.iter().any(|n| n.starts_with(setter)), "{setter} kept no closure");
    }
}

/// The source of `gui`, file by file, relative to `src`.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, root: &std::path::Path, out: &mut Vec<(String, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, root, out);
            } else if p.extension().is_some_and(|x| x == "rs")
                && let (Ok(rel), Ok(text)) = (p.strip_prefix(root), std::fs::read_to_string(&p))
            {
                out.push((rel.to_string_lossy().replace('\\', "/"), text));
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

/// Lint: every `pub fn` of `element.rs` that writes an attribute (through
/// `attr` or `relation`) is in [`SETTERS`], so it is given a closure above.
#[test]
fn every_attribute_setter_is_listed() {
    let element = include_str!("../element.rs");
    let mut found = Vec::new();
    for part in element.split("    pub fn ").skip(1) {
        let name: String = part.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let body = part.split("\n    }\n").next().unwrap_or_default();
        if body.contains("self.attr(") || body.contains(".attr(\"") || body.contains("self.relation(") {
            found.push(name);
        }
    }
    let mut listed: Vec<String> = SETTERS.iter().map(|s| (*s).to_owned()).collect();
    found.sort();
    listed.sort();
    assert_eq!(found, listed, "SETTERS is out of step with element.rs");
}

/// Lint (review r1, Opus I3): a node's fixed attributes are read only by
/// its resolve, which writes its bound ones over them; code that decides
/// by its role asks in a way that follows a binding. Anything else would
/// miss what a binding gives.
#[test]
fn only_the_resolve_reads_a_nodes_fixed_attributes() {
    const RESOLVE: &[&str] = &["element.rs", "layout/patch.rs", "access/bound.rs"];
    let allowed: &[(&str, &[&str])] = &[("attrs_to_resolve(", RESOLVE)];
    for (file, text) in sources() {
        for (call, files) in allowed {
            assert!(
                !text.contains(call) || files.contains(&file.as_str()),
                "{file} calls {call}: only the node's resolve reads its fixed attributes"
            );
        }
    }
}

// ---- Review round 2 (round 3's repros) ----

/// The one node labelled `label` in the whole tree.
fn node_labelled(s: &mut Scenario, label: &str) -> Node {
    full(s)
        .nodes
        .into_iter()
        .find(|(_, n)| n.label() == Some(label))
        .map(|(_, n)| n)
        .unwrap()
}

/// Review r2 (Opus hunt 1): an attribute closure reads its node's
/// interaction flags as a style binding does: bound and written before it
/// runs, so the first frame reads them as they are and a cold build agrees
/// (no early read of an unbound handle bumps live state).
#[test]
fn r3a_an_attribute_reads_its_nodes_flags_bound() {
    let mut s = counting(
        move || {
            let i = Interaction::new();
            div()
                .child(
                    div()
                        .height(10.0)
                        .interaction(i)
                        .disabled(true)
                        .role(Role::Button)
                        .aria_label(move || format!("d{}", i.disabled())),
                )
                .into_element()
        },
        100,
    );
    s.check_cold(true);
    update(&mut s);
    let n = full(&mut s);
    assert!(n.nodes.iter().any(|(_, n)| n.label() == Some("dtrue")), "{:?}", n.nodes);
}

/// The same, a flag written by focus: Tab focuses the node, and its bound
/// `aria_selected` reads it.
#[test]
fn r3a_an_attribute_follows_its_nodes_focus() {
    let mut s = counting(
        move || {
            let i = Interaction::new();
            div()
                .child(
                    div()
                        .height(10.0)
                        .interaction(i)
                        .focusable()
                        .role(Role::Tab)
                        .aria_label("tab")
                        .aria_selected(move || i.focused()),
                )
                .into_element()
        },
        100,
    );
    s.check_cold(true);
    update(&mut s);
    assert_eq!(node_labelled(&mut s, "tab").is_selected(), Some(false));
    s.press(crate::events::NamedKey::Tab, crate::events::Modifiers::default());
    update(&mut s);
    assert_eq!(node_labelled(&mut s, "tab").is_selected(), Some(true));
}

/// Control: the same read from a style binding.
#[test]
fn r3a_a_style_binding_reads_its_nodes_flags_bound() {
    let mut s = counting(
        move || {
            let i = Interaction::new();
            div()
                .child(
                    div()
                        .height(10.0)
                        .interaction(i)
                        .disabled(true)
                        .role(Role::Button)
                        .aria_label("b")
                        .background(move || if i.disabled() { Color::WHITE } else { Color::BLACK }),
                )
                .into_element()
        },
        100,
    );
    s.check_cold(true);
    update(&mut s);
}

/// Review r2 (Opus OOS 4, Codex hunt 1): a bound attribute whose first run
/// panicked runs again only when what it read changes, not when its node
/// resolves for another binding's sake.
#[test]
fn r3b_a_first_run_panic_does_not_rerun_on_an_unrelated_resolve() {
    let runs = Rc::new(Cell::new(0_u32));
    let (open, fail) = (Signal::new(false), Signal::new(0_u32));
    let counted = Rc::clone(&runs);
    let mut s = counting(
        move || {
            let counted = Rc::clone(&counted);
            div()
                .child(
                    div()
                        .height(10.0)
                        .role(Role::Tab)
                        .aria_label("tab")
                        .aria_expanded(open)
                        .aria_selected(move || -> bool {
                            bump(&counted);
                            let _ = fail.get();
                            panic!("no selected value")
                        }),
                )
                .into_element()
        },
        100,
    );
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    assert_eq!(runs.get(), 1);
    s.act("open", || open.set(true));
    s.settle().unwrap();
    assert_eq!(runs.get(), 1, "an unrelated attribute's change does not run it again");
    assert_eq!(node_labelled(&mut s, "tab").is_expanded(), Some(true));
    s.act("what it read", || fail.set(1));
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    assert_eq!(runs.get(), 2, "what it read changing runs it again");
}

/// The same for a bound text: one mechanism for every binding kind.
#[test]
fn r3b_a_text_whose_first_run_panicked_does_not_rerun_on_an_unrelated_resolve() {
    let runs = Rc::new(Cell::new(0_u32));
    let lit = Signal::new(false);
    let counted = Rc::clone(&runs);
    let mut s = counting(
        move || {
            let counted = Rc::clone(&counted);
            div()
                .child(
                    crate::element::text_bound(move || -> String {
                        bump(&counted);
                        panic!("no text")
                    })
                    .background(move || if lit.get() { Color::WHITE } else { Color::BLACK }),
                )
                .into_element()
        },
        100,
    );
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    assert_eq!(runs.get(), 1);
    s.act("lit", || lit.set(true));
    s.settle().unwrap();
    assert_eq!(runs.get(), 1, "its node's colour changing does not run it again");
}

/// One row of a table, built by `row`.
fn one_row_table(row: impl Fn() -> Div + 'static) -> Scenario {
    let state = crate::list::ListState::new(24.0);
    let row = Rc::new(row);
    counting(
        move || {
            let row = Rc::clone(&row);
            crate::list::virtual_list(
                &state,
                1,
                |i| i,
                move |_| row().height(24.0).aria_label("r").into_element(),
            )
            .height(100.0)
            .role(Role::Table)
            .into_element()
        },
        200,
    )
}

/// Review r2 (kept Codex C): the row index follows the role the node
/// resolved to, held over a panic, not a second run of the role's closure.
#[test]
fn r3c_a_virtual_rows_index_follows_its_held_role() {
    let row = Signal::new(false);
    let failed = Rc::new(Cell::new(false));
    let mut s = one_row_table(move || {
        let failed = Rc::clone(&failed);
        div().role(move || {
            if row.get() {
                assert!(failed.replace(true), "fails once");
                Role::Row
            } else {
                Role::Heading
            }
        })
    });
    update(&mut s);
    s.act("row", || row.set(true));
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    let n = node_labelled(&mut s, "r");
    assert_eq!((n.role(), n.row_index()), (Role::Heading, None));
}

/// Review r2 (fresh Codex 2): a fixed row role under a bound one whose
/// first run panics is a row, with its index.
#[test]
fn r3c_a_virtual_row_whose_bound_role_panics_keeps_its_fixed_role_and_index() {
    let mut s = one_row_table(|| div().role(Role::Row).role(|| -> Role { panic!("no role") }));
    assert!(catch_unwind(AssertUnwindSafe(|| s.settle())).is_err());
    let n = node_labelled(&mut s, "r");
    assert_eq!((n.role(), n.row_index()), (Role::Row, Some(0)));
}

/// Review r2 (Opus 2): the list's index never replaces one the row set.
#[test]
fn r3c_a_virtual_row_keeps_its_own_index() {
    for bound in [false, true] {
        let mut s = one_row_table(move || {
            let d = div().aria_row_index(7_usize);
            if bound {
                d.role(|| Role::Heading)
            } else {
                d.role(Role::Heading)
            }
        });
        update(&mut s);
        assert_eq!(node_labelled(&mut s, "r").row_index(), Some(7), "bound={bound}");
        let mut s = one_row_table(move || {
            let d = div().aria_row_index(7_usize);
            if bound { d.role(|| Role::Row) } else { d.role(Role::Row) }
        });
        update(&mut s);
        assert_eq!(node_labelled(&mut s, "r").row_index(), Some(7), "bound={bound}");
    }
}

/// Review r2 (D): a relation's targets are built in call order: a fixed
/// call appends, a closure replaces what came before with what it gives,
/// and a fixed call after it appends to that.
#[test]
fn r3d_a_relations_targets_are_built_in_call_order() {
    let (a, b, c) = (NodeRef::new(), NodeRef::new(), NodeRef::new());
    let mut s = counting(
        move || {
            let target = |n: NodeRef, l: &'static str| div().height(5.0).role(Role::Label).aria_label(l).node_ref(n);
            div()
                .child(target(a, "a"))
                .child(target(b, "b"))
                .child(target(c, "c"))
                .child(
                    div()
                        .height(5.0)
                        .role(Role::Button)
                        .aria_label("probe")
                        .labelled_by(a)
                        .labelled_by(move || vec![b])
                        .labelled_by(c)
                        .described_by(a)
                        .described_by(move || b)
                        .described_by(c)
                        .controls(a)
                        .controls(move || vec![b])
                        .controls(c)
                        .owns(a)
                        .owns(move || b)
                        .owns(c),
                )
                .into_element()
        },
        100,
    );
    update(&mut s);
    let u = full(&mut s);
    let id = |l: &str| {
        u.nodes
            .iter()
            .find(|(_, n)| n.label() == Some(l))
            .map(|(id, _)| *id)
            .unwrap()
    };
    let want = vec![id("b"), id("c")];
    let p = u
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("probe"))
        .map(|(_, n)| n)
        .unwrap();
    assert_eq!(p.labelled_by(), want.as_slice());
    assert_eq!(p.described_by(), want.as_slice());
    assert_eq!(p.controls(), want.as_slice());
    assert_eq!(p.owns(), want.as_slice());
}
