//! Sources a bound setter reads without a tracker of its own (review
//! §12.5, level 2; `gui::value::source`): an interaction's flag, a signal,
//! a group. A mount of one holds no tracker (a signal's hub is one for
//! every mount that reads the signal; a group's is the group's), and a
//! write runs exactly the mounts that read what it wrote.

use gui::app::Scenario;
use gui::interact::Flag;
use gui::layout::{Frame, LayoutTree, UiState};
use gui::prelude::*;
use gui::reactive::{Signal, graph};
use gui::value::Group;
use gui::{Color, Div, Element, Interaction, component, div, text};

const REST: Color = Color::WHITE;
const LIT: Color = Color::BLACK;
/// Nodes per tree.
const N: usize = 16;

fn lay_out(tree: &mut LayoutTree, root: &Element) {
    let mut ts = gui::text::TextSystem::new();
    tree.layout(
        root,
        400.0,
        2000.0,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut ts,
    );
}

fn node<'a>(f: &'a Frame, name: &str) -> Option<&'a gui::layout::FrameNode> {
    f.nodes.iter().find(|n| n.name == Some(name))
}

/// `N` nodes made by `each` under a root.
fn tree(each: impl Fn() -> Div) -> Element {
    div().children((0..N).map(|_| each())).into_element()
}

/// The trackers (render nodes) and subscriptions a tree of `bound` holds
/// once laid out, past what the same tree with fixed values holds; and
/// whether dropping it left the graph as it was.
fn held(bound: &Element, fixed: &Element) -> (i64, i64, bool) {
    let count = |root: &Element| {
        let before = graph();
        let mut t = LayoutTree::new();
        lay_out(&mut t, root);
        let after = graph();
        drop(t);
        let gone = graph();
        let d = |a: usize, b: usize| {
            i64::try_from(a)
                .unwrap_or(i64::MAX)
                .saturating_sub(i64::try_from(b).unwrap_or(i64::MAX))
        };
        (
            d(after.renders, before.renders),
            d(after.subscriptions, before.subscriptions),
            gone.renders == before.renders && gone.subscriptions == before.subscriptions,
        )
    };
    let (b, f) = (count(bound), count(fixed));
    (b.0.saturating_sub(f.0), b.1.saturating_sub(f.1), b.2 && f.2)
}

/// A mount of a source holds no tracker of its own: none for a flag, one
/// hub for every mount of one signal, none past the group's own for a
/// group. A closure holds one per mount; it is the yardstick.
#[test]
fn a_source_mount_holds_no_tracker() {
    // Hover: a closure, then the source.
    let fixed = tree(|| div().interaction(Interaction::new()).size(10.0).background(REST));
    let closure = tree(|| {
        let i = Interaction::new();
        div()
            .interaction(i)
            .size(10.0)
            .background(move || if i.hovered() { LIT } else { REST })
    });
    let source = tree(|| {
        let i = Interaction::new();
        div().interaction(i).size(10.0).background(i.hover(REST, LIT))
    });
    let n = i64::try_from(N).unwrap_or(i64::MAX);
    assert_eq!(
        held(&closure, &fixed),
        (n, n, true),
        "a closure: a tracker and a read each"
    );
    assert_eq!(held(&source, &fixed), (0, 0, true), "the hover source: neither");

    // A signal.
    let w = Signal::new(10.0_f32);
    let fixed = tree(|| div().height(10.0).width(10.0));
    let closure = tree(move || div().height(10.0).width(move || w.get()));
    let source = tree(move || div().height(10.0).width(w));
    let mapped = tree(move || div().height(10.0).width(w.map(|w| w * 2.0)));
    // What the engine makes once per thread (a debug build's check of a
    // map) made before counting.
    held(&mapped, &fixed);
    assert_eq!(
        held(&closure, &fixed),
        (n, n, true),
        "a closure: a tracker and a read each"
    );
    assert_eq!(
        held(&source, &fixed),
        (2, 2, true),
        "the first mount's tracker and the others' hub, gone with the tree"
    );
    assert_eq!(held(&mapped, &fixed), (2, 2, true), "a map of it: the same");

    // A group of two signals, made before the tree (its hub with it).
    let (a, b) = (Signal::new(1.0_f32), Signal::new(0.5_f32));
    let group = Group::new(move || a.get() * b.get());
    let fixed = tree(|| div().size(10.0).opacity(0.5));
    let g = group.clone();
    let closure = tree(move || {
        let g = g.clone();
        div().size(10.0).opacity(move || g.with(|v| *v))
    });
    let source = tree(move || div().size(10.0).opacity(group.map(|v| *v)));
    assert_eq!(
        held(&closure, &fixed),
        (n, 2 * n, true),
        "a closure: a tracker each, reading both signals"
    );
    // The group's hub reads its two signals at the first mount, and keeps
    // them as long as the group lives (it outlives the tree).
    let (renders, reads, _) = held(&source, &fixed);
    assert_eq!(renders, 0, "the group's source: no tracker past the group's own");
    assert!(reads <= 2, "the group's own reads at most: {reads}");
}

/// A write runs the mounts that read what it wrote, and no other: not
/// the mounts of another signal, not another source on the same node.
#[test]
fn a_write_runs_exactly_the_mounts_that_read_it() {
    let (w, h) = (Signal::new(10.0_f32), Signal::new(20.0_f32));
    let root = div()
        .children((0..N).map(|_| div().width(w).height(5.0)))
        .children((0..N).map(|_| div().width(5.0).height(h.map(|h| h + 1.0))))
        .children((0..N).map(|_| div().width(w.map(|w| w / 2.0)).height(h)))
        .into_element();
    let mut t = LayoutTree::new();
    lay_out(&mut t, &root);
    let _mounted = gui::component::take_bindings();
    let n = u64::try_from(N).unwrap_or(u64::MAX);

    w.set(30.0);
    lay_out(&mut t, &root);
    assert_eq!(
        gui::component::take_bindings(),
        2 * n,
        "the width mounts alone run, of both kinds of node"
    );
    let widths: Vec<f32> = t.frame().nodes.iter().map(|n| n.bounds.width).collect();
    assert!(
        widths.contains(&30.0) && widths.contains(&15.0),
        "and write: {widths:?}"
    );

    h.set(40.0);
    lay_out(&mut t, &root);
    assert_eq!(gui::component::take_bindings(), 2 * n, "then the height mounts alone");

    w.set(30.0);
    lay_out(&mut t, &root);
    assert_eq!(gui::component::take_bindings(), 0, "an equal value runs nothing");
}

/// A group's change runs its mounts, and a closure reading it, and
/// nothing of another group; the group reads its signals once for all.
#[test]
fn a_group_change_runs_its_mounts_alone() {
    let (a, b) = (Signal::new(0.5_f32), Signal::new(10.0_f32));
    let reads = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let counted = std::rc::Rc::clone(&reads);
    let opacity = Group::new(move || {
        counted.set(counted.get().saturating_add(1));
        a.get()
    });
    let rotate = Group::new(move || b.get());
    let o = opacity.clone();
    let root = div()
        .children((0..N).map(|_| div().size(10.0).opacity(opacity.map(|v| *v))))
        .children((0..N).map(|_| div().size(10.0).rotate(rotate.map(|v| *v))))
        // Both groups on one node: a change of one runs that one's mount.
        .children((0..N).map(|_| div().size(10.0).opacity(opacity.map(|v| *v)).rotate(rotate.map(|v| *v))))
        .child(div().size(10.0).translate_x(move || o.with(|v| *v * 100.0)))
        .into_element();
    let mut t = LayoutTree::new();
    lay_out(&mut t, &root);
    let _mounted = gui::component::take_bindings();
    reads.set(0);
    let n = u64::try_from(N).unwrap_or(u64::MAX);

    a.set(0.25);
    lay_out(&mut t, &root);
    assert_eq!(
        gui::component::take_bindings(),
        2 * n + 1,
        "the opacity group's mounts and the closure reading it"
    );
    assert_eq!(
        reads.get(),
        2,
        "the group read once for its mounts, once by the closure"
    );
    let o = t
        .frame()
        .nodes
        .iter()
        .filter(|n| (n.look.opacity - 0.25).abs() < 1e-6)
        .count();
    assert_eq!(o, 2 * N, "each mount wrote the new value");

    b.set(45.0);
    lay_out(&mut t, &root);
    assert_eq!(
        gui::component::take_bindings(),
        2 * n,
        "the rotate group's mounts alone"
    );
}

/// Rows lit while hovered through a source, and row 7's ring shown while
/// focus shows: hovering runs the hover mounts of the row the pointer left
/// and the row it came to, nothing else, and holds no tracker.
#[test]
fn hovering_runs_the_hover_mounts_alone() {
    let rows = || {
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .children((0..20).map(|r| {
                let i = Interaction::new();
                let row = div()
                    .interaction(i)
                    .height(20.0)
                    .background(i.hover(REST, LIT))
                    .child(text(format!("row {r}")));
                if r == 7 {
                    row.ring(i.when(Flag::FocusVisible, None, Some(Ring::new(3.0, LIT))))
                } else {
                    row
                }
            }))
            .into_element()
    };
    let mut s = Scenario::measuring(rows, 300, 600, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let trackers = graph().renders;
    for (step, row) in ["row 7", "row 8", "row 7", "row 9"].into_iter().enumerate() {
        let (x, y) = s.locate(row).expect("the row");
        s.hover(x, y);
        let r = s.frame(&format!("hover {step}")).expect("a frame");
        assert!(r.rendered.is_empty(), "no component renders: {:?}", r.rendered);
        if step > 0 {
            assert_eq!(r.visits.resolved, 2, "the row left and the row entered: {}", r.visits);
            assert_eq!(r.bindings, 2, "their hover mounts alone run");
        }
        let lit = s.last().nodes.iter().filter(|n| n.look.background == Some(LIT)).count();
        assert_eq!(lit, 1, "one row lit at {row}");
    }
    assert_eq!(graph().renders, trackers, "hovering made no tracker");
}

/// A node hidden by a source (`display: none`) comes back when what hid
/// it changes: its registration wakes its parent, which builds it again.
#[test]
fn a_node_hidden_by_a_source_comes_back() {
    let shown = Signal::new(false);
    let root = div()
        .child(
            div()
                .named("hidden")
                .size(10.0)
                .display(shown.map(|on| if *on { Display::Block } else { Display::None })),
        )
        .into_element();
    let mut t = LayoutTree::new();
    lay_out(&mut t, &root);
    assert!(node(t.frame(), "hidden").is_none(), "hidden at first");
    shown.set(true);
    lay_out(&mut t, &root);
    assert!(node(t.frame(), "hidden").is_some(), "shown once its signal says so");
    shown.set(false);
    lay_out(&mut t, &root);
    assert!(node(t.frame(), "hidden").is_none(), "and hidden again");
}

/// A source of a handle whose node is built after it (a later sibling's)
/// reads it off, then registers with the node's flags once it is bound:
/// hovering that node lights it.
#[test]
fn a_source_of_a_handle_bound_later_follows_it() {
    let later = Interaction::new();
    let ui = move || {
        div()
            .child(div().named("reader").size(20.0).background(later.hover(REST, LIT)))
            .child(div().interaction(later).size(40.0).child(text("target")))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let look = |s: &Scenario| {
        s.last()
            .nodes
            .iter()
            .find(|n| n.name == Some("reader"))
            .and_then(|n| n.look.background)
    };
    assert_eq!(look(&s), Some(REST));
    let (x, y) = s.locate("target").expect("the target");
    s.hover(x, y);
    s.settle().expect("a frame");
    assert_eq!(look(&s), Some(LIT), "the reader follows the node bound after it");
}

/// The background of the node named `name`.
fn look(s: &Scenario, name: &str) -> Option<Color> {
    node(s.last(), name).and_then(|n| n.look.background)
}

/// A reader of a handle whose node goes, the handle given to a node made
/// in its place (a row replaced), follows the replacement: the reader is
/// in a part that does not render again, and runs only when the handle is
/// bound and when the node it names is hovered.
#[test]
fn a_source_follows_its_handle_to_a_replacement_node() {
    let h = Interaction::new();
    let second = Signal::new(false);
    let ui = move || {
        div()
            .child(div().named("reader").size(20.0).background(h.hover(REST, LIT)))
            .child(component((), move |_| {
                move || {
                    // Another place: another node, the same handle.
                    let target = div().interaction(h).size(40.0).child(text("target"));
                    if second.get() {
                        div().child(div().child(target)).into_element()
                    } else {
                        div().child(target).into_element()
                    }
                }
            }))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let (x, y) = s.locate("target").expect("the target");
    s.hover(x, y);
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(LIT), "lit by the first node");
    s.leave();
    s.settle().expect("a frame");
    s.act("replace", || second.set(true));
    let r = s.frame("replace").expect("a frame");
    assert_eq!(r.bindings, 1, "the reader runs once, for the new binding");
    assert_eq!(look(&s, "reader"), Some(REST));
    let (x, y) = s.locate("target").expect("the replacement");
    s.hover(x, y);
    let r = s.frame("hover the replacement").expect("a frame");
    assert_eq!(look(&s, "reader"), Some(LIT), "lit by the replacement");
    assert_eq!(r.bindings, 1, "the reader alone runs");
}

/// Readers of many handles not bound yet (more than any table of them
/// would keep): binding one runs its reader alone, which then follows its
/// node's hover.
#[test]
fn binding_one_of_many_unbound_handles_runs_its_readers_alone() {
    const HANDLES: usize = 70;
    let handles: Vec<Interaction> = (0..HANDLES).map(|_| Interaction::new()).collect();
    let shown = Signal::new(false);
    let first = handles.first().copied().unwrap_or_default();
    let ui = {
        move || {
            div()
                .display(Display::Flex)
                .flex_wrap(true)
                .children(handles.iter().enumerate().map(|(k, h)| {
                    div()
                        .named(if k == 0 { "first" } else { "other" })
                        .size(4.0)
                        .background(h.hover(REST, LIT))
                }))
                .child(component((), move |_| {
                    move || {
                        if shown.get() {
                            div().interaction(first).size(40.0).child(text("target")).into_element()
                        } else {
                            div().into_element()
                        }
                    }
                }))
                .into_element()
        }
    };
    let mut s = Scenario::measuring(ui, 400, 400, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    s.act("bind", || shown.set(true));
    let r = s.frame("bind").expect("a frame");
    assert_eq!(r.bindings, 1, "the reader of the handle bound runs alone");
    let (x, y) = s.locate("target").expect("the target");
    s.hover(x, y);
    let r = s.frame("hover").expect("a frame");
    assert_eq!(r.bindings, 1, "hover runs that reader alone");
    assert_eq!(look(&s, "first"), Some(LIT), "and it follows its node");
}

/// A node read by another, removed by a show toggle and built again: the
/// reader reads it off while it is gone, then follows the new node.
#[test]
fn a_source_follows_a_node_rebuilt_by_a_show_toggle() {
    let h = Interaction::new();
    let shown = Signal::new(true);
    let ui = move || {
        div()
            .child(div().named("reader").size(20.0).background(h.hover(REST, LIT)))
            .child(component((), move |_| {
                move || {
                    if shown.get() {
                        div().interaction(h).size(40.0).child(text("target")).into_element()
                    } else {
                        div().size(40.0).into_element()
                    }
                }
            }))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let (x, y) = s.locate("target").expect("the target");
    s.hover(x, y);
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(LIT));
    s.act("hide", || shown.set(false));
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(REST), "off while the node is gone");
    s.act("show", || shown.set(true));
    s.settle().expect("a frame");
    s.leave();
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(REST));
    s.hover(x, y);
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(LIT), "lit by the node built again");
}

/// A mount given another flag of the same handle (hover, then focus
/// shown) waits for that flag from then on: hover runs it no more, focus
/// shown lights it.
#[test]
fn a_source_rebound_to_another_flag_follows_that_flag() {
    let h = Interaction::new();
    let ring = Signal::new(false);
    let ui = move || {
        div()
            .child(component((), move |_| {
                move || {
                    let bg = if ring.get() {
                        h.when(Flag::FocusVisible, REST, LIT)
                    } else {
                        h.hover(REST, LIT)
                    };
                    div().named("reader").size(20.0).background(bg).into_element()
                }
            }))
            .child(div().interaction(h).focusable().size(40.0).child(text("target")))
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    s.act("ring", || ring.set(true));
    s.settle().expect("a frame");
    let (x, y) = s.locate("target").expect("the target");
    s.hover(x, y);
    let r = s.frame("hover").expect("a frame");
    assert_eq!(r.bindings, 0, "hover wakes it no more");
    assert_eq!(look(&s, "reader"), Some(REST));
    s.press(gui::NamedKey::Tab, gui::Modifiers::default());
    s.settle().expect("a frame");
    assert_eq!(look(&s, "reader"), Some(LIT), "focus shown lights it");
}

/// A tooltip shown while its trigger is hovered, by a source of the
/// trigger's handle: hidden, its registration wakes its parent, which
/// builds it when the trigger is hovered, and hides it when it is left.
#[test]
fn a_node_hidden_by_another_nodes_hover_shows_with_it() {
    let trigger = Interaction::new();
    let ui = move || {
        div()
            .child(div().interaction(trigger).size(40.0).child(text("trigger")))
            .child(
                div()
                    .named("tip")
                    .size(20.0)
                    .display(trigger.hover(Display::None, Display::Block)),
            )
            .into_element()
    };
    let mut s = Scenario::measuring(ui, 200, 200, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    assert!(node(s.last(), "tip").is_none(), "hidden at first");
    let (x, y) = s.locate("trigger").expect("the trigger");
    s.hover(x, y);
    s.settle().expect("a frame");
    assert!(node(s.last(), "tip").is_some(), "shown while the trigger is hovered");
    s.leave();
    s.settle().expect("a frame");
    assert!(node(s.last(), "tip").is_none(), "hidden once it is left");
    s.hover(x, y);
    s.settle().expect("a frame");
    assert!(node(s.last(), "tip").is_some(), "and shown again");
}

/// A map given to a source reads only what it is given: one that reads a
/// signal is reported, naming where it was given.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "tests/sources.rs")]
fn a_map_reading_a_signal_is_reported() {
    let (w, extra) = (Signal::new(10.0_f32), Signal::new(1.0_f32));
    let root = div()
        .child(div().height(10.0).width(w.map(move |w| w + extra.get())))
        .into_element();
    let mut t = LayoutTree::new();
    lay_out(&mut t, &root);
}
