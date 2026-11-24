//! What a source costs on the heap (`gui::value::source`): the one mount
//! of a signal, and a node's flag sources, cost what the closures they
//! replace do, and a node hidden by a source costs its parent's resolves
//! the same each time, nothing churned. Counted on the test's own thread
//! (ADR-0023).

use gui::app::Scenario;
use gui::interact::Flag;
use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;
use gui::reactive::Signal;
use gui::{Color, Element, Interaction, div, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

fn allocations(f: impl FnOnce()) -> u64 {
    let ((), heap) = gui::heap::measure(f);
    // Not counted (no counting allocator): a count no assertion meets.
    heap.map_or(u64::MAX, |h| h.allocations)
}

fn lay_out(tree: &mut LayoutTree, root: &Element) {
    let mut ts = gui::text::TextSystem::new();
    tree.layout(
        root,
        400.0,
        300.0,
        &UiState::default(),
        &mut gui::Motion::default(),
        &mut ts,
    );
}

/// One node lit by `sel`: through a closure, or through `sel.map`.
fn one(sel: Signal<bool>, source: bool) -> Element {
    let lit = |on: bool| if on { Color::BLACK } else { Color::WHITE };
    let swatch = div().size(10.0);
    let swatch = if source {
        swatch.background(sel.map(move |on| lit(*on)))
    } else {
        swatch.background(move || lit(sel.get()))
    };
    div().child(swatch).into_element()
}

/// The one mount of a signal source allocates no more than the closure
/// it replaces: mounted, and run again by a write.
#[test]
fn a_signal_source_mounted_once_costs_what_a_closure_does() {
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    let sel = Signal::new(false);
    let (closure, source) = (one(sel, false), one(sel, true));
    // Warm: the arena's slots, the layout's buffers, the engine's own
    // state, made once.
    for r in [&closure, &source, &closure, &source] {
        let mut t = LayoutTree::new();
        lay_out(&mut t, r);
        sel.set(!sel.get_untracked());
        lay_out(&mut t, r);
    }
    let cost = |root: &Element| {
        let mut t = LayoutTree::new();
        let mount = allocations(|| lay_out(&mut t, root));
        sel.set(!sel.get_untracked());
        let write = allocations(|| lay_out(&mut t, root));
        (mount, write)
    };
    let (c, s) = (cost(&closure), cost(&source));
    assert!(s.0 <= c.0, "mounted: the source {} against the closure {}", s.0, c.0);
    assert!(s.1 <= c.1, "written: the source {} against the closure {}", s.1, c.1);
}

/// Ten button-like nodes, each with six setters following its own flags:
/// through sources, or through the closures they replace.
fn buttons(source: bool) -> Element {
    const REST: Color = Color::WHITE;
    const LIT: Color = Color::BLACK;
    div()
        .children((0..10).map(|_| {
            let i = Interaction::new();
            let d = div().interaction(i).size(20.0);
            if source {
                d.background(i.hover(REST, LIT))
                    .border_color(i.when(Flag::FocusVisible, REST, LIT))
                    .ring(i.when(Flag::FocusVisible, None, Some(Ring::new(3.0, LIT))))
                    .opacity(i.when(Flag::Disabled, 1.0, 0.5))
                    .cursor(i.when(Flag::Disabled, None, Some(Cursor::NotAllowed)))
                    .text_color(i.when(Flag::Pressed, REST, LIT))
            } else {
                let on = move |f: fn(Interaction) -> bool| f(i);
                d.background(move || if on(Interaction::hovered) { LIT } else { REST })
                    .border_color(move || if on(Interaction::focus_visible) { LIT } else { REST })
                    .ring(move || on(Interaction::focus_visible).then(|| Ring::new(3.0, LIT)))
                    .opacity(move || if on(Interaction::disabled) { 0.5 } else { 1.0 })
                    .cursor(move || on(Interaction::disabled).then_some(Cursor::NotAllowed))
                    .text_color(move || if on(Interaction::pressed) { LIT } else { REST })
            }
        }))
        .into_element()
}

/// A node's flag sources allocate no more when mounted than the closures
/// they replace.
#[test]
fn flag_sources_mount_for_no_more_than_closures() {
    assert!(gui::heap::thread().is_some(), "the counting allocator is installed");
    // Warm.
    for source in [false, true, false, true] {
        let mut t = LayoutTree::new();
        lay_out(&mut t, &buttons(source));
    }
    // The least of three: a table's amortized growth falls on one now and
    // then, whichever it is.
    let cost = |source: bool| {
        (0..3)
            .map(|_| {
                let root = buttons(source);
                let mut t = LayoutTree::new();
                allocations(|| lay_out(&mut t, &root))
            })
            .min()
            .unwrap_or(u64::MAX)
    };
    let (closure, source) = (cost(false), cost(true));
    assert!(
        source <= closure,
        "ten nodes of six setters mounted: sources {source} allocations, closures {closure}"
    );
}

/// How the children are hidden.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hidden {
    Fixed,
    Source,
}

/// A parent resolved again and again (its width follows `tick`) over `n`
/// children hidden by `shown`, through a source, or by a fixed value.
fn hiding(tick: Signal<f32>, shown: Signal<bool>, by: Hidden, n: usize) -> impl Fn() -> Element + 'static {
    move || {
        let display = |on: bool| if on { Display::Block } else { Display::None };
        let child = |k: usize| {
            let c = div()
                .named(if k == 0 { "first" } else { "other" })
                .size(10.0)
                .child(text("child"));
            match by {
                Hidden::Source => c.display(shown.map(move |on| display(*on))),
                Hidden::Fixed => c.display(Display::None),
            }
        };
        div()
            .width(move || 100.0 + tick.get())
            .height(40.0)
            .children((0..n).map(child))
            .into_element()
    }
}

/// What each of 20 resolves of the parent allocates.
#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn resolves(pass: gui::testing::Pass, by: Hidden, n: usize) -> Vec<u64> {
    let (tick, shown) = (Signal::new(0.0_f32), Signal::new(false));
    let mut s = Scenario::measuring(hiding(tick, shown, by, n), 300, 100, 1.0).expect("a scenario");
    pass.scenario(&mut s);
    s.settle().expect("a frame");
    let step = |s: &mut Scenario, k: u16| {
        s.act("tick", || tick.set(f32::from(k)));
        s.frame("tick").expect("a frame");
    };
    // Warm.
    for k in 1..=4 {
        step(&mut s, k);
    }
    let each: Vec<u64> = (5..=24).map(|k| allocations(|| step(&mut s, k))).collect();
    if by == Hidden::Source {
        assert!(s.locate("child").is_err(), "hidden");
        s.act("show", || shown.set(true));
        s.settle().expect("a frame");
        assert!(s.locate("child").is_ok(), "they come back");
    }
    each
}

/// Each resolve of the parent of nodes hidden by a source allocates the
/// same, and past what the same resolve allocates over nodes hidden by a
/// fixed value, only what building each hidden node's setter allocates:
/// nothing each resolve for the parent (its mark, its list, a signal's
/// hub, kept across resolves), and nothing more for a node than for one.
#[test]
fn a_parent_of_nodes_hidden_by_a_source_resolves_without_churn() {
    gui::testing::both_passes(churn);
}

fn churn(pass: gui::testing::Pass) {
    // What each resolve allocates past one over nodes hidden by a fixed
    // value (the engine's own growth falls on the same resolves either way).
    let past = |n: usize| -> Vec<i64> {
        let (fixed, source) = (resolves(pass, Hidden::Fixed, n), resolves(pass, Hidden::Source, n));
        source
            .iter()
            .zip(&fixed)
            .map(|(&s, &f)| {
                i64::try_from(s)
                    .unwrap_or(i64::MAX)
                    .saturating_sub(i64::try_from(f).unwrap_or(0))
            })
            .collect()
    };
    let (two, four) = (past(2), past(4));
    pass.measured(|| {
        let per_node = two.first().copied().unwrap_or(0).saturating_div(2);
        let (of_two, of_four) = (per_node.saturating_mul(2), per_node.saturating_mul(4));
        assert!(
            two.iter().all(|&a| a == of_two) && four.iter().all(|&a| a == of_four),
            "the same each resolve, for each node, nothing for the parent: two {two:?}, four {four:?}"
        );
    });
}
