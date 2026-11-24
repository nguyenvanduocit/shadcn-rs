//! What a source costs on the heap (`gui::value::source`): the one mount
//! of a signal costs what the closure it replaces does, and a node hidden
//! by a source costs its parent's resolves nothing past what a node hidden
//! by a fixed value does. Counted on the test's own thread (ADR-0023).

use gui::app::Scenario;
use gui::layout::{LayoutTree, UiState};
use gui::prelude::*;
use gui::reactive::Signal;
use gui::{Color, Element, div, text};

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

/// How the children are hidden.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hidden {
    Closure,
    Source,
}

/// A parent resolved again and again (its width follows `tick`) over two
/// children hidden by `shown`, through a source or a closure.
fn hiding(tick: Signal<f32>, shown: Signal<bool>, by: Hidden) -> impl Fn() -> Element + 'static {
    move || {
        let display = |on: bool| if on { Display::Block } else { Display::None };
        let child = |name: &'static str| {
            let c = div().named(name).size(10.0).child(text(name));
            match by {
                Hidden::Source => c.display(shown.map(move |on| display(*on))),
                Hidden::Closure => c.display(move || display(shown.get())),
            }
        };
        div()
            .width(move || 100.0 + tick.get())
            .height(40.0)
            .child(child("one"))
            .child(child("two"))
            .into_element()
    }
}

/// Frames resolving the parent of nodes hidden by a source allocate no
/// more than those of nodes hidden by a closure (each resolve builds the
/// hidden nodes again, and mounts their setters, either way): what the
/// hidden nodes' registrations hold (a signal's hub, the parent's mark and
/// its list) is kept across the parent's resolves, not made anew. And the
/// nodes come back when the source says so.
#[test]
fn a_parent_of_nodes_hidden_by_a_source_resolves_without_churn() {
    gui::testing::both_passes(churn);
}

#[expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
fn churn(pass: gui::testing::Pass) {
    let per_frames = |by: Hidden| {
        let (tick, shown) = (Signal::new(0.0_f32), Signal::new(false));
        let mut s = Scenario::measuring(hiding(tick, shown, by), 300, 100, 1.0).expect("a scenario");
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
        let heap = allocations(|| {
            for k in 5..=24 {
                step(&mut s, k);
            }
        });
        assert!(s.locate("one").is_err(), "hidden");
        s.act("show", || shown.set(true));
        s.settle().expect("a frame");
        assert!(s.locate("one").is_ok() && s.locate("two").is_ok(), "both come back");
        heap
    };
    let (closure, source) = (per_frames(Hidden::Closure), per_frames(Hidden::Source));
    pass.measured(|| {
        assert!(
            source <= closure,
            "20 resolves of the parent: hidden by a source {source} allocations, by a closure {closure}"
        );
    });
}
