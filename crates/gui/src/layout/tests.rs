use super::binding::Tracked;
use super::*;
use crate::element::{IntoElement, Side, SideAlign, Text, deferred, div, key_of, span, text};
use crate::focus::FocusHandle;
use crate::placement::EDGE_MARGIN;
use crate::prelude::*;
use crate::styled::Styled;

fn frame(root: impl IntoElement) -> Frame {
    let mut ts = TextSystem::new();
    layout(
        &root.into_element(),
        400.0,
        300.0,
        &UiState::default(),
        &mut Motion::default(),
        &mut ts,
    )
}

fn find<'a>(f: &'a Frame, name: &str) -> &'a FrameNode {
    f.nodes.iter().find(|n| n.name == Some(name)).expect("node")
}

fn changed(index: u32, x: f32, repainted: bool) -> Changed {
    Changed {
        index,
        before: Bounds {
            x,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        repainted,
    }
}

/// A log of `n` entries, node `i % 4` each, its box before at `i`.
fn logged(n: u16) -> Changes {
    let mut c = Changes::default();
    c.rebuilt(0);
    c.log
        .extend((0..n).map(|i| changed(u32::from(i % 4), f32::from(i), false)));
    c
}

/// Room for no more than 2 * 0 + 64 entries: a trim of 100 keeps the
/// last 32 and counts 68 dropped.
#[test]
fn a_trim_drops_the_older_half_and_counts_it() {
    let mut c = logged(100);
    let mut at_start = ReadAt::default();
    assert_eq!(c.since(&mut at_start), Since::Rebuilt, "never read: the whole frame");
    let (mut at_dropped, mut before_dropped, mut at_end) = (at_start, at_start, at_start);
    at_dropped.at = 68;
    before_dropped.at = 67;
    at_end.at = 100;
    c.trim(0);
    assert_eq!((c.log.len(), c.end()), (32, 100));
    let Since::Logged(log) = c.since(&mut at_dropped) else {
        panic!("a reader at the first entry kept reads on");
    };
    assert_eq!(log, &c.log[..]);
    assert_eq!(log.first().map(|e| e.before.x), Some(68.0));
    assert_eq!(
        c.since(&mut before_dropped),
        Since::FellBehind,
        "one entry it had not read was dropped"
    );
    assert_eq!(c.since(&mut at_end), Since::Logged(&[]));
    // Each moved to the end.
    assert_eq!(c.since(&mut before_dropped), Since::Logged(&[]));
    assert_eq!(at_dropped.at, 100);
    // The count keeps growing past what the log holds.
    c.log.push(changed(1, 0.0, true));
    assert_eq!(c.end(), 101);
    assert!(matches!(c.since(&mut at_end), Since::Logged([e]) if e.repainted));
}

#[test]
fn a_rebuild_resets_what_was_dropped_and_every_reader() {
    let mut c = logged(100);
    let mut read = ReadAt::default();
    let _ = c.since(&mut read);
    c.trim(0);
    let epoch = c.epoch;
    c.rebuilt(0);
    assert_ne!(c.epoch, epoch);
    assert_eq!((c.log.len(), c.dropped, c.end()), (0, 0, 0));
    assert_eq!(c.since(&mut read), Since::Rebuilt, "indices it held mean nothing");
    c.log.push(changed(0, 0.0, false));
    assert_eq!(c.since(&mut read), Since::Logged(&[changed(0, 0.0, false)]));
}

/// The reader of every change (the frame report's diff) never falls
/// behind: what
/// a trim drops before it read it is kept for it, a node's first box
/// before and whether any entry repainted it.
#[test]
fn what_a_trim_drops_unread_is_kept_for_the_reader_of_every_change() {
    let mut c = logged(10);
    let mut got = Vec::new();
    assert!(!c.unread(&mut got), "its first read compares the whole frame");
    assert!(c.unread(&mut got) && got.is_empty());
    c.log
        .extend((10..100_u16).map(|i| changed(u32::from(i % 4), f32::from(i), i == 50)));
    c.trim(0);
    assert!(c.unread(&mut got));
    // The 58 dropped since it read (10..68), one per node, then the log.
    let (kept, log) = got.split_at(4);
    assert_eq!(
        kept,
        [
            changed(0, 12.0, false),
            changed(1, 13.0, false),
            changed(2, 10.0, true),
            changed(3, 11.0, false)
        ]
    );
    assert_eq!(log, &c.log[..]);
    // What it read is not kept again.
    c.log.extend((100..200_u16).map(|i| changed(5, f32::from(i), false)));
    c.trim(0);
    assert!(c.unread(&mut got));
    assert_eq!(got.first(), Some(&changed(5, 100.0, false)));
    assert_eq!(got.len(), 1 + 32);
    // A rebuild from node 3 on keeps what was written before it.
    c.log.push(changed(1, 7.0, true));
    c.log.push(changed(3, 8.0, true));
    c.rebuilt(3);
    assert!(c.unread(&mut got));
    assert_eq!(got, [changed(1, 7.0, true)]);
}

/// Pass 0 writes nodes in place; pass 1 rebuilds from a node before
/// them, and pass 2 from one before that (the kept nodes merged): what
/// is kept is the frame as painted, each node's first box before, and
/// the one repainted marked so.
#[test]
fn a_rebuild_keeps_the_nodes_as_painted_not_as_earlier_passes_wrote_them() {
    let mut f = frame(div().children((0..8).map(|_| div().size(10.0))));
    let mut got = Vec::new();
    // Painted: the frame report read it.
    let _ = f.changes.unread(&mut got);
    f.before = None;
    let painted = f.nodes.clone();
    let len = f.nodes.len();
    assert!(len >= 8);
    let at = |x: f32| Bounds {
        x,
        y: 0.0,
        width: 10.0,
        height: 10.0,
    };
    // Pass 0: node 7 moved twice (its first box before counts), node 5
    // repainted, node 3 moved.
    for (index, x, repainted) in [
        (7_usize, 50.0, false),
        (7, 60.0, false),
        (5, -1.0, true),
        (3, 40.0, false),
    ] {
        let n = &mut f.nodes[index];
        f.changes.log.push(Changed {
            index: u32::try_from(index).expect("small"),
            before: n.bounds,
            repainted,
        });
        if x >= 0.0 {
            n.bounds = at(x);
        }
    }
    let written = f.nodes.clone();
    // Pass 1 rebuilds from 4, writing the nodes as they are now.
    LayoutTree::rebuild_from(&mut f, 4);
    f.nodes.extend_from_slice(&written[4..]);
    f.changes.rebuilt(4);
    // Pass 2 rebuilds from 2.
    LayoutTree::rebuild_from(&mut f, 2);
    f.nodes.extend_from_slice(&written[2..]);
    f.changes.rebuilt(2);
    let before = f.before.take().expect("kept");
    assert_eq!(before.at, 2);
    assert_eq!(
        before.nodes.iter().map(|n| n.bounds).collect::<Vec<_>>(),
        painted[2..].iter().map(|n| n.bounds).collect::<Vec<_>>(),
        "as painted"
    );
    assert_eq!(before.repainted, [5 - 2]);
    // Nothing written in place stays to read: the kept nodes hold it.
    assert!(f.changes.unread(&mut got) && got.is_empty());
    let d = before.diff(&f);
    let changed: Vec<(Bounds, Bounds, bool)> = d
        .changes
        .iter()
        .filter_map(|c| match c.change {
            crate::inspect::Change::Updated {
                before,
                after,
                repainted,
            } => Some((before, after, repainted)),
            _ => None,
        })
        .collect();
    assert_eq!(d.changes.len(), 3, "{d}");
    assert_eq!(order(&f, &d), [3, 5, 7], "in tree order: {d}");
    assert!(changed.contains(&(painted[7].bounds, at(60.0), false)), "{d}");
    assert!(changed.contains(&(painted[3].bounds, at(40.0), false)), "{d}");
    assert!(changed.contains(&(painted[5].bounds, painted[5].bounds, true)), "{d}");
}

/// A frame of eight boxes the frame report has read (painted), as
/// painted: what the tests of a rebuild's undo start from.
fn painted_frame() -> (Frame, Vec<FrameNode>) {
    let mut f = frame(div().children((0..8).map(|_| div().size(10.0))));
    let _ = f.changes.unread(&mut Vec::new());
    f.before = None;
    let painted = f.nodes.clone();
    assert!(painted.len() >= 9);
    (f, painted)
}

/// A pass writes node `index` in place: its box to `x` (none when
/// `x` is negative), logged with its box before.
fn write_at(f: &mut Frame, index: usize, x: f32, repainted: bool) {
    let n = &mut f.nodes[index];
    f.changes.log.push(Changed {
        index: u32::try_from(index).expect("small"),
        before: n.bounds,
        repainted,
    });
    if x >= 0.0 {
        n.bounds.x = x;
    }
}

/// A pass rebuilds the frame from `start`, writing its nodes as `now`.
fn rebuild_as(f: &mut Frame, start: usize, now: &[FrameNode]) {
    LayoutTree::rebuild_from(f, start);
    f.nodes.extend_from_slice(&now[start..]);
    f.changes.rebuilt(start);
}

/// The order of the changed nodes in `d`, by index in `f`: each
/// change is of a node still in `f` (one that went has no index; see
/// the removal test for those).
fn order(f: &Frame, d: &crate::inspect::LayoutDiff) -> Vec<usize> {
    d.changes
        .iter()
        .map(|c| {
            f.nodes
                .iter()
                .position(|n| n.path == c.path)
                .unwrap_or_else(|| panic!("{:?} is not in the frame: {d}", c.path))
        })
        .collect()
}

/// A write the frame report read before the rebuild is part of the
/// frame as painted: the rebuild undoes only what came after it.
#[test]
fn a_rebuild_does_not_undo_what_the_report_already_read() {
    let (mut f, painted) = painted_frame();
    write_at(&mut f, 5, 50.0, false);
    // Painted again: the report read the move.
    let mut got = Vec::new();
    assert!(f.changes.unread(&mut got) && got.len() == 1);
    write_at(&mut f, 6, 60.0, true);
    let now = f.nodes.clone();
    rebuild_as(&mut f, 4, &now);
    let before = f.before.take().expect("kept");
    assert_eq!(before.nodes[5 - 4].bounds.x, 50.0, "read: as painted, moved");
    assert_eq!(before.nodes[6 - 4].bounds, painted[6].bounds, "unread: undone");
    assert_eq!(before.repainted, [6 - 4]);
    let d = before.diff(&f);
    assert_eq!(order(&f, &d), [6], "{d}");
}

/// A trim folds the writes the report had not read; a rebuild in the
/// same frame undoes them from what it folded.
#[test]
fn a_rebuild_after_a_trim_undoes_what_the_trim_folded() {
    let (mut f, painted) = painted_frame();
    write_at(&mut f, 5, 50.0, false);
    write_at(&mut f, 7, -1.0, true);
    write_at(&mut f, 5, 55.0, false);
    // Enough writes (each leaving node 1 where it was) that a trim
    // drops those above.
    for _ in 0..(2 * f.nodes.len() + 64) {
        write_at(&mut f, 1, -1.0, false);
    }
    f.changes.trim(f.nodes.len());
    assert!(f.changes.dropped >= 3, "the trim dropped them");
    assert!(f.changes.log.iter().all(|c| c.index == 1));
    let now = f.nodes.clone();
    rebuild_as(&mut f, 4, &now);
    let before = f.before.take().expect("kept");
    assert_eq!(before.nodes[5 - 4].bounds, painted[5].bounds, "its first box before");
    assert_eq!(before.repainted, [7 - 4]);
    let d = before.diff(&f);
    assert_eq!(order(&f, &d), [5, 7], "in tree order: {d}");
}

/// Between two rebuilds, from A then from S before it, passes write in
/// place: the second undoes the writes into [S, A), and ignores those
/// into nodes from A on, which the first kept as painted.
#[test]
fn a_second_rebuild_undoes_only_writes_into_what_it_newly_keeps() {
    let (mut f, painted) = painted_frame();
    let (s, a) = (2, 5);
    let now = f.nodes.clone();
    rebuild_as(&mut f, a, &now);
    write_at(&mut f, 3, 30.0, false);
    write_at(&mut f, 3, 35.0, false);
    write_at(&mut f, 4, -1.0, true);
    write_at(&mut f, 6, 60.0, true);
    write_at(&mut f, 7, 70.0, false);
    let now = f.nodes.clone();
    rebuild_as(&mut f, s, &now);
    let before = f.before.take().expect("kept");
    assert_eq!(before.at, s);
    assert_eq!(
        before.nodes.iter().map(|n| n.bounds).collect::<Vec<_>>(),
        painted[s..].iter().map(|n| n.bounds).collect::<Vec<_>>(),
        "as painted"
    );
    assert_eq!(before.repainted, [4 - s], "node 6's write ignored");
    let d = before.diff(&f);
    assert_eq!(order(&f, &d), [3, 4, 6, 7], "in tree order: {d}");
}

/// A rebuild that drops a marked node and reorders keyed ones: every
/// change, removals included, in tree order then gone, each once; the
/// marked node that went shows only as removed.
#[test]
fn a_rebuild_reports_a_removal_beside_a_keyed_reorder_once_each() {
    use crate::inspect::Change;
    let mut f = frame(div().children((0..8).map(|k| div().key(key_of(&format!("c{k}"))).size(10.0))));
    let _ = f.changes.unread(&mut Vec::new());
    f.before = None;
    // Child `k` of the root lies at node `k + 1`.
    let child = |k: usize| k + 1;
    for k in 0..8 {
        let key = key_of(&format!("c{k}"));
        assert_eq!(f.nodes[child(k)].path.0.last(), Some(&PathSeg::Key(key)));
    }
    // Painted, then c2 and c5 repainted in place.
    write_at(&mut f, child(2), -1.0, true);
    write_at(&mut f, child(5), -1.0, true);
    let painted = f.nodes.clone();
    // Rebuilt without c5, c1 and c3 swapped: each takes the box of
    // the place it lands in.
    let landed = [0, child(0), child(3), child(2), child(1), child(4), child(6), child(7)];
    let now: Vec<FrameNode> = landed
        .iter()
        .enumerate()
        .map(|(at, &k)| FrameNode {
            bounds: painted[at].bounds,
            ..painted[k].clone()
        })
        .collect();
    rebuild_as(&mut f, 1, &now);
    let before = f.before.take().expect("kept");
    assert_eq!(before.repainted, [child(2) - 1, child(5) - 1]);
    let d = before.diff(&f);
    let got: Vec<(NodePath, Change)> = d.changes.iter().map(|c| (c.path.clone(), c.change.clone())).collect();
    let path = |k: usize| painted[child(k)].path.clone();
    let bounds = |at: usize| painted[at].bounds;
    let moved = |k: usize, to: usize| Change::Updated {
        before: bounds(child(k)),
        after: bounds(to),
        repainted: false,
    };
    let expected = vec![
        (path(3), moved(3, child(1))),
        (
            path(2),
            Change::Updated {
                before: bounds(child(2)),
                after: bounds(child(2)),
                repainted: true,
            },
        ),
        (path(1), moved(1, child(3))),
        (path(6), moved(6, child(5))),
        (path(7), moved(7, child(6))),
        (
            path(5),
            Change::Removed {
                before: bounds(child(5)),
            },
        ),
    ];
    assert_eq!(got, expected, "{d}");
}

/// A binding that panics leaves no tracker subscribed: what held its
/// trackers drops while the thread unwinds, and disposes them then.
#[test]
fn trackers_dropped_while_unwinding_leave_the_graph() {
    let marks: Marks = Rc::default();
    let read = crate::reactive::Signal::new(1_u32);
    let before = crate::reactive::graph();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut tracked: Tracked<dyn Fn() -> u32, u32> = Tracked::new(&marks, 0);
        let f: Rc<dyn Fn() -> u32> = Rc::new(move || read.get());
        let _ = tracked.get(&f, || f());
        let mut d = div().width(move || f32::from(u16::try_from(read.get()).unwrap_or(0)));
        let (style, bindings) = d.style_and_bindings();
        let ops = Rc::clone(bindings.and_then(|b| b.ops()).expect("bound"));
        let mut sb = binding::StyleBinding::new(&marks, 0);
        let _ = sb.get(&ops, style);
        std::panic::panic_any("a binding panicked");
    }));
    assert!(unwound.is_err());
    let after = crate::reactive::graph();
    assert_eq!(after.renders, before.renders, "both trackers left the graph");
    assert_eq!(after.subscriptions, before.subscriptions, "and their reads with them");
    read.set(2);
    assert!(marks.borrow().is_empty(), "a write marks no node");
}

/// A parent resolved again over children hidden by a source keeps the
/// mark its hidden children's registrations wake, and their list: the
/// same mark, the same buffer, each time.
#[test]
fn a_parent_keeps_the_mark_and_list_of_its_hidden_sources() {
    let (tick, shown) = (
        crate::reactive::Signal::new(0.0_f32),
        crate::reactive::Signal::new(false),
    );
    let hidden = |name: &'static str| {
        div()
            .named(name)
            .size(10.0)
            .display(shown.map(|on| if *on { Display::Block } else { Display::None }))
    };
    let root = div()
        .width(move || 100.0 + tick.get())
        .child(hidden("one"))
        .child(hidden("two"))
        .into_element();
    let mut tree = LayoutTree::new();
    let kept = |tree: &mut LayoutTree, k: f32| {
        tick.set(k);
        let mut ts = TextSystem::new();
        tree.layout(
            &root,
            400.0,
            300.0,
            &UiState::default(),
            &mut Motion::default(),
            &mut ts,
        );
        tree.extras.iter().flatten().find_map(|e| {
            e.hidden_mark.as_ref().map(|m| {
                (
                    Rc::as_ptr(m).cast::<()>(),
                    e.hidden_waits.as_ptr(),
                    e.hidden_waits.len(),
                )
            })
        })
    };
    let first = kept(&mut tree, 1.0);
    assert!(
        first.is_some_and(|(_, _, n)| n == 2),
        "two registrations wake the parent: {first:?}"
    );
    for k in 2..6_u8 {
        assert_eq!(
            kept(&mut tree, f32::from(k)),
            first,
            "resolve {k}: the same mark and list"
        );
    }
}

/// Lay `root` out on `tree`, live; the widths of the nodes named `names`.
fn widths<const N: usize>(tree: &mut LayoutTree, root: &Element, names: [&str; N]) -> [f32; N] {
    let mut ts = TextSystem::new();
    let f = tree.layout(root, 400.0, 300.0, &UiState::default(), &mut Motion::default(), &mut ts);
    names.map(|n| find(f, n).bounds.width)
}

/// Two nodes, one bound setter: one width closure cloned onto both.
fn twins(w: crate::reactive::Signal<f32>) -> Element {
    let bound = div().height(10.0).width(move || w.get());
    div()
        .child(bound.clone().named("a"))
        .child(bound.named("b"))
        .into_element()
}

/// One bound setter cloned onto two nodes: each node keeps what the
/// closure gave there, so each gets its first write and every change
/// after, whichever runs first.
#[test]
fn one_binding_on_two_nodes_writes_each_its_first_and_later_values() {
    let w = crate::reactive::Signal::new(40.0_f32);
    let root = twins(w);
    let mut tree = LayoutTree::new();
    assert_eq!(widths(&mut tree, &root, ["a", "b"]), [40.0, 40.0], "both first writes");
    for v in [60.0, 80.0, 60.0] {
        w.set(v);
        assert_eq!(widths(&mut tree, &root, ["a", "b"]), [v, v], "both take {v}");
    }
}

/// A cold build (the cold check, a fork: a tree of its own) runs the
/// same bound setters the live tree mounted, and leaves what the live
/// nodes last gave as it was: the live tree still sees the change.
#[test]
fn a_cold_build_of_the_same_bindings_leaves_the_live_ones_as_they_were() {
    let w = crate::reactive::Signal::new(40.0_f32);
    let root = twins(w);
    let mut live = LayoutTree::new();
    assert_eq!(widths(&mut live, &root, ["a", "b"]), [40.0, 40.0]);
    w.set(70.0);
    let mut ts = TextSystem::new();
    let cold = layout(
        &root,
        400.0,
        300.0,
        &UiState::default(),
        &mut Motion::default(),
        &mut ts,
    );
    assert_eq!(find(&cold, "a").bounds.width, 70.0, "the cold build runs them");
    assert_eq!(
        widths(&mut live, &root, ["a", "b"]),
        [70.0, 70.0],
        "the live nodes write the change they had not seen"
    );
}

/// A bound setter's or bound text's closure that panics in a cold build
/// (its tree dropped while the panic unwinds) leaves no tracker in the
/// graph and no read subscribed.
#[test]
fn a_binding_that_panics_leaves_no_tracker_subscribed() {
    on_its_own_thread(a_binding_that_panics_leaves_no_tracker_subscribed_here);
}

fn a_binding_that_panics_leaves_no_tracker_subscribed_here() {
    let read = crate::reactive::Signal::new(1_u32);
    let style = move || -> Element {
        div()
            .width(move || {
                if read.get() > 0 {
                    std::panic::panic_any("a binding panicked");
                }
                10.0
            })
            .into_element()
    };
    let content = move || -> Element {
        crate::element::text_bound(move || {
            if read.get() > 0 {
                std::panic::panic_any("a binding panicked");
            }
            String::new()
        })
        .into_element()
    };
    for root in [style(), content()] {
        let before = crate::reactive::graph();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| frame(root)));
        assert!(unwound.is_err(), "the closure panicked");
        let after = crate::reactive::graph();
        assert_eq!(after.renders, before.renders, "its trackers left the graph");
        assert_eq!(after.subscriptions, before.subscriptions, "and their reads with them");
    }
}

/// A retained node whose only bound setter is replaced by a fixed value
/// drops its trackers: a write to what the old closure read asks for no
/// resolve.
#[test]
fn a_node_whose_last_binding_went_reads_nothing() {
    let w = crate::reactive::Signal::new(40.0_f32);
    let mut tree = LayoutTree::new();
    let bound = div()
        .child(div().named("a").height(10.0).width(move || w.get()))
        .into_element();
    assert_eq!(widths(&mut tree, &bound, ["a"]), [40.0]);
    let fixed = div().child(div().named("a").height(10.0).width(50.0)).into_element();
    assert_eq!(widths(&mut tree, &fixed, ["a"]), [50.0]);
    w.set(60.0);
    assert!(tree.marks.borrow().is_empty(), "the old closure's read marks nothing");
}

/// A closure that gives a constant, as a component built again gives it:
/// the same closure type each time, another value.
fn constant(x: f32) -> impl Fn() -> f32 {
    move || x
}

/// Built again with another closure of the same setter, no signal
/// written: the node's mount takes the new closure and runs it.
#[test]
fn a_binding_built_again_runs_its_new_closure() {
    let mut tree = LayoutTree::new();
    let built = |x| {
        div()
            .child(div().named("a").height(10.0).width(constant(x)))
            .into_element()
    };
    assert_eq!(widths(&mut tree, &built(10.0), ["a"]), [10.0]);
    assert_eq!(widths(&mut tree, &built(20.0), ["a"]), [20.0]);
}

/// A pass over a retained tree that panics in one node's binding leaves
/// the tree as a new one: tried again on the same tree, it holds the
/// trackers and reads a fresh tree holds, no more, and shows the values
/// as they are now.
#[test]
fn a_pass_that_panics_leaves_the_tree_as_a_fresh_one() {
    on_its_own_thread(|| {
        let w = crate::reactive::Signal::new(10.0_f32);
        let boom = crate::reactive::Signal::new(false);
        let root = div()
            .children(["a", "b", "c"].into_iter().enumerate().map(move |(i, n)| {
                div().named(n).height(10.0).width(move || {
                    if i == 1 && boom.get() {
                        std::panic::panic_any("a binding panicked");
                    }
                    w.get() + i as f32
                })
            }))
            .into_element();
        let at_start = crate::reactive::graph();
        let held = |g: crate::reactive::Graph| (g.renders - at_start.renders, g.subscriptions - at_start.subscriptions);
        let (mut motion, mut ts) = (Motion::default(), TextSystem::new());
        let names = |f: &Frame| ["a", "b", "c"].map(|n| find(f, n).bounds.width);
        let fresh = {
            let mut t = LayoutTree::new();
            assert_eq!(names(lay(&mut t, &root, &mut motion, &mut ts)), [10.0, 11.0, 12.0]);
            held(crate::reactive::graph())
        };
        let mut tree = LayoutTree::new();
        assert_eq!(names(lay(&mut tree, &root, &mut motion, &mut ts)), [10.0, 11.0, 12.0]);
        w.set(15.0);
        boom.set(true);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lay(&mut tree, &root, &mut motion, &mut ts);
        }));
        assert!(unwound.is_err(), "the pass panicked");
        boom.set(false);
        w.set(20.0);
        assert_eq!(names(lay(&mut tree, &root, &mut motion, &mut ts)), [20.0, 21.0, 22.0]);
        assert_eq!(
            held(crate::reactive::graph()),
            fresh,
            "trackers and reads as a fresh tree's"
        );
    });
}

/// Run `f` on a thread of its own: its own reactive graph, so what it
/// counts is its own whatever ran on the test's thread before.
fn on_its_own_thread(f: impl FnOnce() + Send + 'static) {
    if let Err(panic) = std::thread::spawn(f).join() {
        std::panic::resume_unwind(panic);
    }
}

/// Lay `root` out on `tree` with `motion` and `ts`, kept across passes as a
/// window keeps them.
fn lay<'t>(tree: &'t mut LayoutTree, root: &Element, motion: &mut Motion, ts: &mut TextSystem) -> &'t Frame {
    tree.layout(root, 400.0, 300.0, &UiState::default(), motion, ts)
}

/// A node with a transition removed in a pass that panics: the motion
/// kept across the panic and the retry holds no state for it after.
#[test]
fn a_pass_that_panics_leaves_motion_nothing_of_the_nodes_it_dropped() {
    on_its_own_thread(|| {
        let boom = crate::reactive::Signal::new(false);
        let root = |moving: bool| {
            let d = div().child(div().named("b").height(10.0).width(move || {
                if boom.get() {
                    std::panic::panic_any("a binding panicked");
                }
                10.0
            }));
            if moving {
                d.child(
                    div()
                        .named("m")
                        .size(10.0)
                        .transition(Properties::BACKGROUND)
                        .background(Color::BLACK),
                )
                .into_element()
            } else {
                d.into_element()
            }
        };
        let (mut motion, mut ts) = (Motion::default(), TextSystem::new());
        let mut tree = LayoutTree::new();
        lay(&mut tree, &root(true), &mut motion, &mut ts);
        assert_eq!(motion.paths().count(), 1, "the moving node has a state");
        boom.set(true);
        let gone = root(false);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lay(&mut tree, &gone, &mut motion, &mut ts);
        }));
        assert!(unwound.is_err(), "the pass panicked");
        boom.set(false);
        lay(&mut tree, &gone, &mut motion, &mut ts);
        assert_eq!(motion.paths().count(), 0, "the node that went left no state behind");
    });
}

/// Two sibling layers, B opened before A: after a pass that panics, the
/// retry paints them in the order they opened, B under A, not in tree
/// order.
#[test]
fn a_pass_that_panics_keeps_the_order_its_layers_opened_in() {
    on_its_own_thread(|| {
        let boom = crate::reactive::Signal::new(false);
        let layer = |name: &'static str| {
            deferred(
                Positioning::Anchored {
                    side: Side::Bottom,
                    align: SideAlign::Start,
                    offset: 0.0,
                    match_width: false,
                    fit_height: false,
                },
                div().named(name).width(20.0).height(20.0),
            )
        };
        let root = move |a: bool| {
            let holder = |open: bool, name| {
                let d = div().width(40.0).height(20.0);
                if open { d.child(layer(name)) } else { d }
            };
            div()
                .width(move || {
                    if boom.get() {
                        std::panic::panic_any("a binding panicked");
                    }
                    200.0
                })
                .child(holder(a, "a"))
                .child(holder(true, "b"))
                .into_element()
        };
        let (mut motion, mut ts) = (Motion::default(), TextSystem::new());
        let mut tree = LayoutTree::new();
        let layers = |f: &Frame| (find(f, "a").layer, find(f, "b").layer);
        lay(&mut tree, &root(false), &mut motion, &mut ts);
        let both = root(true);
        assert_eq!(
            layers(lay(&mut tree, &both, &mut motion, &mut ts)),
            (2, 1),
            "B opened first"
        );
        boom.set(true);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            lay(&mut tree, &both, &mut motion, &mut ts);
        }));
        assert!(unwound.is_err(), "the pass panicked");
        boom.set(false);
        assert_eq!(
            layers(lay(&mut tree, &both, &mut motion, &mut ts)),
            (2, 1),
            "still B under A"
        );
    });
}

/// A closure that panicked after reading a signal, whose write is then
/// no longer bound (the node keeps another binding): its tracker reads
/// nothing, so a write to that signal asks for no resolve.
#[test]
fn a_binding_that_panicked_then_went_reads_nothing() {
    on_its_own_thread(|| {
        let marks: Marks = Rc::default();
        let w = crate::reactive::Signal::new(10.0_f32);
        let mut before = div().opacity(move || 0.5).width(move || -> f32 {
            if w.get() > 0.0 {
                std::panic::panic_any("a binding panicked");
            }
            0.0
        });
        let mut after = div().opacity(move || 0.5);
        let ops = |d: &mut crate::element::Div| {
            let (style, b) = d.style_and_bindings();
            (style.clone(), Rc::clone(b.and_then(|b| b.ops()).expect("bound")))
        };
        let ((s1, o1), (s2, o2)) = (ops(&mut before), ops(&mut after));
        let mut sb = binding::StyleBinding::new(&marks, 0);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sb.get(&o1, &s1)));
        assert!(unwound.is_err(), "the width's closure panicked");
        sb.get(&o2, &s2);
        marks.borrow_mut().clear();
        w.set(20.0);
        assert!(marks.borrow().is_empty(), "what the closure read marks nothing");
    });
}

/// A bound setter's closure, or a restyle replayed after it, that
/// panicked runs again at the next resolve, no signal written since: what
/// it half did is not kept.
#[test]
fn a_binding_or_restyle_that_panicked_runs_again_at_the_next_resolve() {
    use std::cell::Cell;
    for restyle in [false, true] {
        let marks: Marks = Rc::default();
        let w = crate::reactive::Signal::new(10.0_f32);
        let boom = Rc::new(Cell::new(false));
        let (in_binding, in_restyle) = (Rc::clone(&boom), Rc::clone(&boom));
        let mut d = div().width(move || {
            if !restyle && in_binding.get() {
                std::panic::panic_any("a binding panicked");
            }
            w.get()
        });
        let (style, bindings) = d.style_and_bindings();
        let bindings = bindings.expect("a node keeps bindings");
        if restyle {
            assert!(bindings.restyle(Rc::new(move |s: Style| {
                if in_restyle.get() {
                    std::panic::panic_any("a restyle panicked");
                }
                s
            })));
        }
        let ops = Rc::clone(bindings.ops().expect("bound"));
        let resolved = || {
            let mut s = style.clone();
            bindings.apply_to(&mut s);
            s
        };
        let mut sb = binding::StyleBinding::new(&marks, 0);
        assert!(sb.get(&ops, style) == resolved());
        w.set(20.0);
        boom.set(true);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sb.get(&ops, style)));
        assert!(unwound.is_err(), "it panicked");
        boom.set(false);
        assert!(
            sb.get(&ops, style) == resolved(),
            "the width as it is now (restyle: {restyle})"
        );
    }
}

#[test]
fn flex_row_places_children_side_by_side_with_gap() {
    let f = frame(
        div()
            .display(Display::Flex)
            .gap(8.0)
            .padding(16.0)
            .child(div().named("a").size(40.0))
            .child(div().named("b").size(40.0)),
    );
    let (a, b) = (find(&f, "a").bounds, find(&f, "b").bounds);
    assert_eq!((a.x, a.y, a.width), (16.0, 16.0, 40.0));
    assert_eq!(b.x, 64.0, "40px box + 8px gap after the first");
}

#[test]
fn text_wraps_inside_a_narrow_box() {
    let f = frame(div().child(div().named("box").width(60.0).child(text("several words that wrap"))));
    let b = find(&f, "box").bounds;
    assert!(b.height >= 48.0, "at least two 24px lines: {b}");
}

#[test]
fn a_flex_item_shrinks_to_its_widest_word_and_no_further() {
    let f = frame(
        div().child(
            div()
                .display(Display::Flex)
                .align_items(Align::Start)
                .width(20.0)
                .child(div().named("prefix").child(text("https://")))
                .child(div().named("rest").flex(Flex::grow(1.0))),
        ),
    );
    let b = find(&f, "prefix").bounds;
    assert!(b.width > 20.0, "as wide as the word, overflowing: {b}");
    assert_eq!(b.height, 24.0, "the word on one line: {b}");
}

#[test]
fn a_deferred_layer_opens_below_its_anchor_and_stays_on_screen() {
    let f = frame(
        div().padding(8.0).child(
            div()
                .named("anchor")
                .margin_left(320.0)
                .width(60.0)
                .height(20.0)
                .child(deferred(
                    Positioning::Anchored {
                        side: Side::Bottom,
                        align: SideAlign::Start,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    },
                    div().named("menu").width(200.0).height(50.0),
                )),
        ),
    );
    let anchor = find(&f, "anchor").bounds;
    let menu = find(&f, "menu");
    assert_eq!(menu.layer, 1);
    assert_eq!(menu.bounds.y, anchor.bottom() + 4.0);
    assert_eq!(
        menu.bounds.right(),
        400.0 - EDGE_MARGIN,
        "pushed back inside the window"
    );
}

#[test]
fn a_scroll_offset_is_set_in_constant_work_however_many_paths_are_kept() {
    let mut state = UiState::default();
    let path = |i: usize| NodePath::new([PathSeg::Index(0), PathSeg::Index(i)].into());
    for i in 0..1_000 {
        state.set_scroll((path(i), None), (0.0, 1.0));
    }
    let (a, b) = (crate::reactive::Scope::new(), crate::reactive::Scope::new());
    let (a, b) = (a.run(crate::reactive::owner), b.run(crate::reactive::owner));
    let before = state.scroll.visited;
    state.set_scroll((path(500), a), (0.0, 20.0));
    state.set_scroll((path(500), b), (0.0, 30.0));
    assert_eq!(state.scroll.visited - before, 2, "one entry a write");
    assert_eq!(
        state.scroll.get(&(path(500), a)),
        None,
        "the owner replaced at the path left nothing"
    );
    assert_eq!(state.scroll.get(&(path(500), b)), Some(&(0.0, 30.0)));
    assert_eq!(state.scroll.get(&(path(500), None)), None);
    assert_eq!(state.scroll.paths().count(), 1_000, "one entry a path");
}

/// A layout pass places the scroll writes since the last pass, and
/// only those: one change among 1,000 remembered offsets is one entry.
#[test]
fn a_pass_places_only_the_scrolls_written_since_the_last() {
    let mut state = UiState::default();
    let mut ts = TextSystem::new();
    let el = div()
        .child(
            div()
                .named("list")
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .children((0..10).map(|_| div().height(40.0))),
        )
        .into_element();
    let mut tree = LayoutTree::new();
    tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let list = find(tree.frame(), "list").path.clone();
    for i in 0..999 {
        let other = NodePath::new([PathSeg::Index(7), PathSeg::Index(i)].into());
        state.set_scroll((other, None), (0.0, 1.0));
    }
    state.set_scroll((list.clone(), None), (0.0, 10.0));
    tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(state.scroll.logged(), 0, "the pass took the log");
    let before = tree.scroll_marked;
    for i in 0..10_000_u16 {
        state.set_scroll((list.clone(), None), (0.0, f32::from(i % 300)));
    }
    state.set_scroll((list, None), (0.0, 50.0));
    assert_eq!(state.scroll.logged(), 1, "one path written, one entry");
    tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(tree.scroll_marked - before, 1, "one placement for it");
    let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
    assert_eq!(offset, Some(50.0), "and it is placed");
}

/// Driven by the public API alone, a frame at a time, the window's
/// input and layout keep the scroll log bounded: each pass takes it.
#[test]
fn ten_thousand_scrolled_frames_leave_the_scroll_log_bounded() {
    let mut ui = crate::interact::InputState::default();
    let mut ts = TextSystem::new();
    let el = div()
        .child(
            div()
                .named("list")
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .children((0..10).map(|_| div().height(40.0))),
        )
        .into_element();
    let mut tree = LayoutTree::new();
    tree.layout(&el, 400.0, 300.0, ui.state(), &mut Motion::default(), &mut ts);
    for k in 0..10_000_u32 {
        let step = crate::compositor::WheelStep {
            point: (50.0, 50.0),
            at: f32::from(u16::try_from(k % 1000).unwrap_or(0)),
            ..crate::compositor::WheelStep::default()
        };
        let dy = if k % 2 == 0 { 30.0 } else { -30.0 };
        ui.wheel_at(tree.frame(), step, (0.0, dy), None);
        tree.layout(&el, 400.0, 300.0, ui.state(), &mut Motion::default(), &mut ts);
        assert!(ui.state().scroll.logged() == 0, "frame {k}: the pass took the log");
        if k == 0 {
            let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
            assert_eq!(offset, Some(30.0), "the wheel scrolls it");
        }
    }
    let offset = find(tree.frame(), "list").scroll.map(|s| s.offset.1);
    assert_eq!(offset, Some(0.0), "it did scroll, back and forth");
}

#[test]
fn scrolled_children_move_up_and_are_clipped() {
    let mut state = UiState::default();
    let mut ts = TextSystem::new();
    let tree = || {
        div()
            .child(
                div()
                    .named("list")
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .children((0..10).map(|i| div().named(if i == 0 { "first" } else { "item" }).height(40.0))),
            )
            .into_element()
    };
    let before = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let path = find(&before, "list").path.clone();
    state.scroll.insert((path, None), (0.0, 1000.0));
    let after = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let list = find(&after, "list");
    assert_eq!(list.scroll.map(|s| s.offset.1), Some(300.0), "clamped to 400 - 100");
    assert_eq!(find(&after, "first").bounds.y, -300.0);
    assert_eq!(find(&after, "first").clip, Some(list.bounds));
}

#[test]
fn a_scroll_offset_moves_content_by_whole_device_pixels() {
    let mut state = UiState {
        scale: 2.0,
        ..UiState::default()
    };
    let mut ts = TextSystem::new();
    let tree = || {
        div()
            .child(
                div()
                    .named("list")
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .children((0..10).map(|i| div().named(if i == 0 { "first" } else { "item" }).height(40.0))),
            )
            .into_element()
    };
    let before = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let path = find(&before, "list").path.clone();
    state.scroll.insert((path, None), (0.0, 10.3));
    let after = layout(&tree(), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(
        find(&after, "first").bounds.y,
        -10.0,
        "10.3 is 20.6 device pixels: 20 whole ones"
    );
}

// ── The retained tree against a cold layout (L1, L3, L7) ────────────────

/// A seeded generator (an LCG: tests repeat).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }
    fn below(&mut self, n: u64) -> usize {
        usize::try_from(self.next() % n.max(1)).unwrap()
    }
}

/// An element tree as data, mutated step by step.
#[derive(Clone, Debug)]
struct Spec {
    look: u8,
    key: Option<u64>,
    text: Option<String>,
    kids: Vec<Spec>,
}

const WORDS: [&str; 8] = ["a", "box", "several", "words", "that", "wrap", "longer-word", "x"];

fn words(r: &mut Rng) -> String {
    (0..=r.below(6))
        .map(|_| WORDS[r.below(8)])
        .collect::<Vec<_>>()
        .join(" ")
}

fn spec(r: &mut Rng, depth: usize) -> Spec {
    if depth == 0 || r.below(4) == 0 {
        // A paragraph, now and then holding elements in its lines.
        let inline = if depth > 0 && r.below(3) == 0 { r.below(3) } else { 0 };
        return Spec {
            look: u8::try_from(r.below(8)).unwrap(),
            key: None,
            text: Some(words(r)),
            kids: (0..inline).map(|_| spec(r, depth - 1)).collect(),
        };
    }
    Spec {
        look: u8::try_from(r.below(64)).unwrap(),
        key: (r.below(3) == 0).then(|| r.next() % 5),
        text: None,
        kids: (0..r.below(5)).map(|_| spec(r, depth - 1)).collect(),
    }
}

/// Siblings' keys are unique (a repeated key is the caller's error).
fn element(s: &Spec) -> Element {
    built(s, true)
}

fn built(s: &Spec, keyed: bool) -> Element {
    if let Some(t) = &s.text {
        let mut seen = HashSet::new();
        let p = s.kids.iter().fold(text(t.clone()), |p, k| {
            p.child(built(k, k.key.is_none_or(|key| seen.insert(key))))
                .span(" and more")
        });
        let d = div().child(p);
        return match s.look % 4 {
            0 => d.width(50.0),
            1 => d.flex(Flex::grow(1.0)),
            2 => d.padding(4.0),
            _ => d,
        }
        .into_element();
    }
    let mut d = match s.look % 8 {
        0 => div().display(Display::Flex),
        1 => div().display(Display::Flex).flex_direction(FlexDirection::Column),
        2 => div()
            .display(Display::Grid)
            .grid_columns(Tracks::repeat(2, Track::Fr(1.0))),
        3 => div().display(Display::Flex).flex_wrap(true),
        4 => div().display(Display::Block),
        5 => div().position(Position::Absolute).top(4.0).left(8.0),
        6 => div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .height(60.0)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden),
        _ => div(),
    };
    d = match (s.look / 8) % 6 {
        0 => d.padding(8.0),
        1 => d.gap(4.0).align_items(Align::Center),
        2 => d.margin_left(Length::Auto),
        3 => d.width(120.0),
        4 => d.justify_content(Justify::Between).height(40.0),
        _ => d.max_width(Length::calc(1.0, -20.0)),
    };
    if s.look >= 60 {
        d = d.display(Display::None);
    }
    if let (Some(k), true) = (s.key, keyed) {
        d = d.key(k);
    }
    let mut seen = HashSet::new();
    d.children(
        s.kids
            .iter()
            .map(|k| built(k, k.key.is_none_or(|key| seen.insert(key)))),
    )
    .into_element()
}

/// One random change somewhere in `s`.
fn mutate(s: &mut Spec, r: &mut Rng) {
    if !s.kids.is_empty() && r.below(3) != 0 {
        let i = r.below(s.kids.len() as u64);
        return mutate(&mut s.kids[i], r);
    }
    match r.below(7) {
        0 => s.look = u8::try_from(r.below(64)).unwrap(),
        1 if s.text.is_some() => s.text = Some(words(r)),
        2 => {
            let at = r.below(s.kids.len() as u64 + 1);
            s.kids.insert(at, spec(r, 2));
        }
        3 if !s.kids.is_empty() => {
            let at = r.below(s.kids.len() as u64);
            s.kids.remove(at);
        }
        4 if s.kids.len() > 1 => {
            let last = s.kids.len() - 1;
            s.kids.swap(0, last);
        }
        5 if s.text.is_none() => s.key = (r.below(2) == 0).then(|| r.next() % 5),
        _ => s.kids.reverse(),
    }
}

fn check(seed: u64, schedule: Schedule) {
    let mut r = Rng(seed);
    let mut s = spec(&mut r, 4);
    let mut tree = LayoutTree::with_schedule(schedule);
    let mut ts = TextSystem::new();
    let state = UiState::default();
    for step in 0..40 {
        let el = div().width(pct(100.0)).child(element(&s)).into_element();
        let width = if step % 13 == 12 { 300.0 } else { 400.0 };
        let warm = tree.layout(&el, width, 300.0, &state, &mut Motion::default(), &mut ts);
        let cold = layout(&el, width, 300.0, &state, &mut Motion::default(), &mut ts);
        let d = crate::inspect::diff(&cold.nodes, &warm.nodes);
        assert!(
            d.is_empty() && warm.nodes.len() == cold.nodes.len(),
            "seed {seed} step {step}: the retained tree differs from a cold layout:\n{d}\n{s:#?}"
        );
        mutate(&mut s, &mut r);
    }
}

#[test]
fn patched_trees_lay_out_as_cold_ones() {
    for seed in 0..60 {
        check(seed, Schedule::default());
    }
}

#[test]
fn patched_trees_lay_out_alike_in_parallel_and_in_reverse() {
    for seed in 0..30 {
        check(
            seed,
            Schedule {
                parallel: true,
                reverse: false,
            },
        );
        check(
            seed,
            Schedule {
                parallel: false,
                reverse: true,
            },
        );
    }
}

#[test]
fn an_unchanged_tree_runs_no_layout() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let el = div()
        .display(Display::Flex)
        .child(div().child(text("hello")))
        .child(div().width(10.0))
        .into_element();
    let state = UiState::default();
    tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(tree.stats(), Stats::default());
}

/// A table-like tree big enough to be laid out in parallel jobs.
fn rows(n: usize, tall: usize) -> Element {
    div()
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .width(900.0)
        .children((0..n).map(|i| {
            div()
                .display(Display::Grid)
                .grid_columns(Tracks::repeat(4, Track::Fr(1.0)))
                .gap(4.0)
                .children((0..4).map(|c| {
                    let words = if i == tall {
                        "a much longer cell that wraps over lines"
                    } else {
                        WORDS[(i + c) % 8]
                    };
                    let p = text(format!("{words} {i}.{c}"));
                    // A cell holding an element in its line.
                    let p = if c == 3 {
                        p.child(div().width(12.0).height(30.0).child(text("x"))).span(" y")
                    } else {
                        p
                    };
                    div().padding(4.0).child(p)
                }))
        }))
        .into_element()
}

fn exact(f: &Frame) -> Vec<(u32, u32, u32, u32)> {
    f.nodes
        .iter()
        .map(|n| {
            (
                n.bounds.x.to_bits(),
                n.bounds.y.to_bits(),
                n.bounds.width.to_bits(),
                n.bounds.height.to_bits(),
            )
        })
        .collect()
}

#[test]
fn parallel_serial_and_reversed_layouts_are_bit_identical() {
    let state = UiState::default();
    let mut ts = TextSystem::new();
    let schedules = [
        Schedule {
            parallel: false,
            reverse: false,
        },
        Schedule {
            parallel: true,
            reverse: false,
        },
        Schedule {
            parallel: false,
            reverse: true,
        },
        Schedule {
            parallel: true,
            reverse: true,
        },
    ];
    let mut trees: Vec<LayoutTree> = schedules.iter().map(|s| LayoutTree::with_schedule(*s)).collect();
    let mut jobs = 0;
    for (step, tall) in [usize::MAX, 17, 300, usize::MAX].into_iter().enumerate() {
        let el = rows(400, tall);
        let frames: Vec<Vec<_>> = trees
            .iter_mut()
            .map(|t| exact(t.layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts)))
            .collect();
        jobs += trees[1].stats().jobs;
        let cold = exact(&layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts));
        for (f, s) in frames.iter().zip(&schedules) {
            assert!(*f == cold, "step {step}: {s:?} differs from a cold layout");
        }
    }
    // The parallel schedule did run jobs on the pool.
    assert!(jobs > 0 || engine::pool().current_num_threads() < 2);
}

/// A cold layout of a tree the pool would take runs on its own thread:
/// no job reaches a pool thread's text system, a live cache.
#[test]
fn a_cold_layout_hands_the_pool_no_jobs() {
    let state = UiState::default();
    let mut ts = TextSystem::new();
    let el = rows(400, usize::MAX);
    let mut live = LayoutTree::with_schedule(Schedule {
        parallel: true,
        reverse: false,
    });
    live.layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts);
    if engine::pool().current_num_threads() < 2 {
        return;
    }
    assert!(live.stats().jobs > 0, "the tree is one the pool takes");
    let before = engine::handed();
    let cold =
        crate::engine::in_cold_frame(|| exact(&layout(&el, 1000.0, 800.0, &state, &mut Motion::default(), &mut ts)));
    assert_eq!(engine::handed(), before, "the cold layout handed the pool no job");
    assert!(cold == exact(live.frame()), "and lays out the same");
}

// ── Atomic inlines in a paragraph ───────────────────────────────────────

/// A paragraph's first baseline and its one line's height, alone.
fn strut() -> (f32, f32) {
    let mut ts = TextSystem::new();
    let t = crate::style::ResolvedText::root(Color::BLACK);
    let m = ts.metrics("ab", &t, None);
    (m.first_baseline.expect("a line"), m.height)
}

fn near(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 1.0, "{what}: {a} is not {b}");
}

#[test]
fn a_box_with_a_baseline_sits_on_it_hangs_below_and_grows_the_line() {
    let (ascent, line) = strut();
    // A 60px box whose own first line is at its top: its baseline is
    // the strut's, so it starts at the line's top and hangs 60 - ascent
    // below the baseline, past the strut's descent.
    let f = frame(
        div().width(300.0).child(
            text("ab ")
                .named("p")
                .child(div().named("box").width(30.0).height(60.0).child(text("x")))
                .span(" cd"),
        ),
    );
    let (p, b) = (find(&f, "p").bounds, find(&f, "box").bounds);
    assert!(line < 60.0);
    near(p.height, 60.0, "the line holds the box above and below its baseline");
    near(b.y - p.y, 0.0, "its baseline on the line's, at the strut's ascent");
    assert_eq!(b.height, 60.0);
    let ab = TextSystem::new()
        .measure("ab ", &crate::style::ResolvedText::root(Color::BLACK), None)
        .0;
    near(b.x - p.x, ab, "after the text before it");
    assert!(ascent > 0.0);
}

#[test]
fn a_line_holding_only_a_box_takes_the_strut_and_the_box() {
    let (ascent, line) = strut();
    let descent = line - ascent;
    let only = |h: f32| {
        let f = frame(
            div()
                .width(300.0)
                .child(text("").named("p").child(div().named("box").width(20.0).height(h))),
        );
        (find(&f, "p").bounds, find(&f, "box").bounds)
    };
    // No line box in it: its bottom edge on the baseline.
    let (p, b) = only(10.0);
    near(p.height, ascent.max(10.0) + descent, "a short box: the strut");
    near(b.y - p.y + b.height, ascent, "its bottom on the baseline");
    let (p, b) = only(40.0);
    near(
        p.height,
        40.0 + descent,
        "a tall box above the baseline, the strut's descent below",
    );
    near(b.y - p.y, 0.0, "at the line's top");
}

#[test]
fn a_box_wraps_with_the_words_around_it() {
    let para = |w: f32| {
        let f = frame(
            div().child(
                div().width(w).child(
                    text("several words before ")
                        .named("p")
                        .child(div().named("box").width(40.0).height(10.0)),
                ),
            ),
        );
        (find(&f, "p").bounds, find(&f, "box").bounds)
    };
    let (wide, at_wide) = para(390.0);
    let (narrow, at_narrow) = para(150.0);
    assert!(
        at_wide.x - wide.x > 100.0,
        "on the first line after the words: {at_wide}"
    );
    assert!(
        narrow.height > wide.height,
        "two lines when narrow: {narrow} {at_narrow} vs {wide} {at_wide}"
    );
    assert!(at_narrow.y > at_wide.y, "moved down a line: {at_narrow} vs {at_wide}");
}

#[test]
fn an_inline_box_sizes_to_fit_and_is_a_node_below_its_paragraph() {
    let f = frame(
        div().width(300.0).child(
            text("see ")
                .named("p")
                .child(div().named("chip").padding(4.0).child(text("host")))
                .span(" here"),
        ),
    );
    let (p, chip) = (find(&f, "p"), find(&f, "chip"));
    let host = TextSystem::new()
        .measure("host", &crate::style::ResolvedText::root(Color::BLACK), None)
        .0;
    near(chip.bounds.width, host + 8.0, "shrunk to fit its content, not the line");
    let pi = f.nodes.iter().position(|n| n.name == Some("p")).unwrap();
    assert_eq!(chip.parent, Some(pi), "the chip is the paragraph's child");
    let Content::Text { content, boxes, .. } = &p.content else {
        panic!("a paragraph")
    };
    assert_eq!(boxes.len(), 1);
    assert_eq!(crate::text::readable(content), "see  here");
}

#[test]
fn a_changed_inline_box_lays_out_as_a_cold_tree() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    for w in [20.0, 80.0, 200.0, 20.0] {
        let el = div()
            .width(240.0)
            .child(
                text("some words around ")
                    .child(div().named("box").width(w).height(w / 2.0).child(text("in")))
                    .span(" a box"),
            )
            .into_element();
        let warm = exact(tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
        let cold = exact(&layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
        assert!(
            warm == cold,
            "a {w}px box: the retained tree differs from a cold layout"
        );
    }
}

/// The text of the frame node showing `content`.
fn shown<'a>(f: &'a Frame, content: &str) -> &'a ResolvedText {
    f.nodes
        .iter()
        .find_map(|n| match &n.content {
            Content::Text { content: c, text, .. } if &**c == content => Some(text),
            _ => None,
        })
        .expect("a node shows it")
}

/// A paragraph of many spans replaced at its key by plain text lets go
/// of the buffer its runs were resolved into, and of the runs it showed.
#[test]
fn a_paragraph_that_loses_its_spans_lets_go_of_their_buffer() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    let p = |el: Text| div().child(el.key(key_of("p")).named("p")).into_element();
    let spans = (0..20_000).fold(text(""), |t, _| t.span(span("x").font_weight(700)));
    tree.layout(&p(spans), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let ex = |tree: &LayoutTree| {
        tree.extras
            .iter()
            .flatten()
            .find(|e| e.name == Some("p"))
            .map(|e| (e.runs.capacity(), e.span_content.capacity()))
            .expect("the paragraph")
    };
    let (runs, content) = ex(&tree);
    assert!(
        runs * std::mem::size_of::<Run>() > 64 * 1024,
        "its buffer is large: {runs} runs"
    );
    assert!(content >= 20_000, "one content style a span: {content}");
    tree.layout(&p(text("plain")), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(ex(&tree), (0, 0), "plain text keeps no buffer for runs");
}

/// A bound paragraph replaced at its key by plain text drops its
/// binding: the signal it read resolves nothing, and the text it gave
/// is held nowhere.
#[test]
fn a_bound_paragraph_made_plain_drops_its_binding() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    let read = crate::reactive::Signal::new(1_u32);
    let p = |el: Text| div().child(el.key(key_of("p"))).into_element();
    let bound = p(crate::element::text_bound(move || format!("bound {}", read.get())));
    let f = tree.layout(&bound, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let old = f
        .nodes
        .iter()
        .find_map(|n| match &n.content {
            Content::Text { content, .. } if &**content == "bound 1" => Some(Arc::clone(content)),
            _ => None,
        })
        .expect("the bound text");
    tree.layout(&p(text("plain")), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(Arc::strong_count(&old), 1, "the old text is held nowhere else");
    read.set(2);
    assert!(
        tree.marks.borrow().is_empty(),
        "a write to the old signal marks no node"
    );
}

/// A parent's underline reaches a child in its flow, not an atomic
/// inline nor an absolutely positioned box (css-text-decor-3 §2.1),
/// laid out and when the underline comes on later, repainted.
#[test]
fn decorations_stop_at_atomic_inlines_and_out_of_flow_boxes() {
    let el = |underline: bool| {
        div()
            .underline(underline)
            .child(text("normal"))
            .child(text("inline").inline(true))
            .child(text("absolute").position(Position::Absolute))
            .into_element()
    };
    let check = |f: &Frame| {
        assert!(shown(f, "normal").decorations.underline(), "a child in flow draws it");
        assert!(!shown(f, "inline").decorations.underline(), "an atomic inline does not");
        assert!(
            !shown(f, "absolute").decorations.underline(),
            "an out-of-flow box does not"
        );
    };
    check(&frame(el(true)));
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    tree.layout(&el(false), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    check(tree.layout(&el(true), 400.0, 300.0, &state, &mut Motion::default(), &mut ts));
}

/// A span's vertical padding only grows its background (CSS pads an
/// inline box so, its line box keeping its height): what the
/// paragraph is measured by leaves it out.
#[test]
fn a_runs_vertical_padding_is_not_measured() {
    let run = Run {
        range: 0..1,
        text: ResolvedText::root(Color::BLACK),
        background: Some(Color::BLACK),
        radius: 2.0,
        padding: Edges {
            top: 3.0,
            right: 4.0,
            bottom: 5.0,
            left: 6.0,
        },
    };
    let m = Leaf::measuring_run(&run);
    assert_eq!(
        m.padding,
        Edges {
            top: 0.0,
            right: 4.0,
            bottom: 0.0,
            left: 6.0,
        },
        "the sides take space in the line, the top and bottom none"
    );
}

/// A paragraph that alternates between 20,000 and 9,000 spans keeps the
/// room of the larger: its scratch is not cut on the small update and
/// grown again on the large one.
#[test]
fn a_paragraph_alternating_sizes_keeps_its_scratch() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    let p = |n: usize| {
        let spans = (0..n).fold(text(""), |t, _| t.span(span("x").font_weight(700)));
        div().child(spans.key(key_of("p")).named("p")).into_element()
    };
    let room = |tree: &LayoutTree| {
        tree.extras
            .iter()
            .flatten()
            .find(|e| e.name == Some("p"))
            .map(|e| e.runs.capacity())
            .expect("the paragraph")
    };
    let (big, small) = (p(20_000), p(9_000));
    tree.layout(&big, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    let peak = room(&tree);
    for step in 0..3 {
        tree.layout(&small, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(room(&tree), peak, "step {step}: the small update keeps the room");
        tree.layout(&big, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        assert_eq!(room(&tree), peak, "step {step}: the large one needs no more");
    }
}

/// A layout pass that handed the pool no jobs leaves the pool's text
/// systems alone (what they hold is another pass's to clear), and one
/// that did clears what they shaped before it returns: no flag shared
/// between passes on different threads decides either. The pool is the
/// process's, shared with tests running beside this one: an attempt
/// another test's pass disturbed is tried again.
#[test]
fn each_pass_clears_the_pool_after_its_own_jobs() {
    use super::engine::Worker;
    let attempt = || {
        // Another pass's jobs, still running: they shaped text.
        engine::pool().broadcast(|_| {
            TextSystem::with_worker(|t| {
                t.measure("words another pass shaped", &ResolvedText::root(Color::BLACK), None);
            });
        });
        let shaped = leaf::idle_footprint();
        // A pass on another thread that handed the pool nothing ends.
        std::thread::spawn(leaf::end_pass).join().expect("a pass ends");
        let kept = leaf::idle_footprint();
        // A pass that handed it jobs clears after them.
        let mut tree = LayoutTree::with_schedule(Schedule {
            parallel: true,
            reverse: false,
        });
        let mut ts = TextSystem::new();
        tree.layout(
            &rows(400, 17),
            1000.0,
            800.0,
            &UiState::default(),
            &mut Motion::default(),
            &mut ts,
        );
        let cleared = tree.stats().jobs == 0 || leaf::idle_footprint() == 0;
        (shaped, kept, cleared)
    };
    let tries: Vec<_> = (0..8).map(|_| attempt()).collect();
    assert!(
        tries
            .iter()
            .any(|&(shaped, kept, cleared)| shaped > 0 && kept >= shaped && cleared),
        "(shaped, kept by a pass that handed nothing, cleared by one that did): {tries:?}"
    );
}

/// Two empty spans at the same byte and a binding that moves the same
/// cursor from the first to the second: their range and cursor are the
/// same, and the target is still the second's, told apart by which
/// span it is.
#[test]
fn a_span_target_follows_its_span_not_its_start() {
    let mut tree = LayoutTree::new();
    let mut ts = TextSystem::new();
    let state = UiState::default();
    let on = crate::reactive::Signal::new(false);
    let el = div()
        .child(
            text("go ")
                .span(span("").cursor(move || if on.get() { None } else { Some(Cursor::Pointer) }))
                .span(span("").cursor(move || if on.get() { Some(Cursor::Pointer) } else { None }))
                .span("link"),
        )
        .into_element();
    let targets = |f: &Frame| -> Vec<(usize, std::ops::Range<usize>)> {
        f.behaviour
            .iter()
            .flat_map(|b| b.spans.iter())
            .map(|t| (t.index, t.range.clone()))
            .collect()
    };
    let f = tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(targets(f), [(0, 3..3)], "the first empty span takes the pointer");
    on.set(true);
    let f = tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
    assert_eq!(targets(f), [(1, 3..3)], "then the second, at the same byte");
}

/// A button its paragraph's clamp hides is neither focusable nor a
/// target of assistive technology; shown, it is both.
#[test]
fn a_hidden_inline_is_no_focus_and_no_action_target() {
    let focus = FocusHandle::new();
    let el = |width: f32| {
        div()
            .child(
                text("words before ")
                    .child(
                        div()
                            .inline(true)
                            .padding_x(4.0)
                            .track_focus(focus)
                            .on_click(|_| {})
                            .child(text("the button")),
                    )
                    .line_clamp(1)
                    .width(width),
            )
            .into_element()
    };
    for (width, shown) in [(400.0, true), (100.0, false)] {
        let f = frame(el(width));
        let i = (0..f.nodes.len())
            .find(|&i| f.behaviour(i).is_some_and(|b| b.handlers.click.is_some()))
            .expect("the button");
        assert_eq!(f.nodes[i].look.invisible, !shown, "{width}px: hidden or shown");
        let id = crate::access::id_of(&f.nodes[i].path);
        assert_eq!(
            crate::access::target(&f, id).is_some(),
            shown,
            "{width}px: a screen reader's action reaches it only shown"
        );
        assert_eq!(
            crate::focus::node_of(&f, focus.id()).is_some(),
            shown,
            "{width}px: it holds focus only shown"
        );
    }
}

/// The trees L0 counts child asks over: what a parent asks its children,
/// by kind (`Stats::asked`, `Stats::ran`).
mod asks {
    use super::*;
    use taffy::{AvailableSpace, Size};

    /// A row of text leaves, each sized by its content.
    fn text_row() -> Element {
        div()
            .display(Display::Flex)
            .gap(4.0)
            .children((0..8).map(|i| text(format!("{} {i}", WORDS[i % 8]))))
            .into_element()
    }

    /// Flex in flex: rows of cells that grow and wrap.
    fn nested_flex() -> Element {
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(4.0)
            .children((0..4).map(|r| {
                div().display(Display::Flex).gap(4.0).children((0..4).map(move |c| {
                    div()
                        .display(Display::Flex)
                        .flex_grow(1.0)
                        .padding(4.0)
                        .child(text(format!("{} {r}.{c} words", WORDS[(r + c) % 8])))
                }))
            }))
            .into_element()
    }

    /// A grid of cards, each a column of a title and a body.
    fn card_grid() -> Element {
        div()
            .display(Display::Grid)
            .grid_columns(Tracks::repeat(3, Track::Fr(1.0)))
            .gap(8.0)
            .children((0..6).map(|i| {
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .padding(8.0)
                    .child(text(format!("Card {i}")))
                    .child(text("a body that wraps over a few lines of the card"))
            }))
            .into_element()
    }

    /// The window of a virtual list: a scroll box of fixed-height rows,
    /// each a flex row of an icon, a label and a trailing note.
    fn list_window() -> Element {
        div()
            .overflow_y(Overflow::Scroll)
            .height(120.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .children((0..12).map(|i| {
                div()
                    .display(Display::Flex)
                    .gap(4.0)
                    .height(24.0)
                    .child(div().width(16.0).height(16.0))
                    .child(text(format!("row {i} label")).flex_grow(1.0))
                    .child(text("note"))
            }))
            .into_element()
    }

    /// A row too narrow for its text: the leaves wrap at their min-content.
    fn wrapping_row() -> Element {
        div()
            .display(Display::Flex)
            .width(150.0)
            .gap(4.0)
            .children((0..4).map(|i| text(format!("{} several words that wrap {i}", WORDS[i % 8]))))
            .into_element()
    }

    fn trees() -> [(&'static str, Element); 5] {
        [
            ("text row", text_row()),
            ("nested flex", nested_flex()),
            ("card grid", card_grid()),
            ("list window", list_window()),
            ("wrapping row", wrapping_row()),
        ]
    }

    fn laid(el: &Element) -> (Stats, Vec<(u32, u32, u32, u32)>) {
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        tree.layout(el, 400.0, 300.0, &UiState::default(), &mut Motion::default(), &mut ts);
        (tree.stats(), exact(tree.frame()))
    }

    fn kinds(a: Asks) -> [u64; 4] {
        [a.min_content, a.max_content, a.definite, a.placement]
    }

    /// What a parent asks its children, and the leaf measures that
    /// answered, for each tree: a flex row measures a text leaf once for
    /// its base size and its automatic minimum together. Before, the
    /// measures were 24, 48, 12, 72 and 12 (the asks are the same: the
    /// second finds its answer kept).
    #[test]
    fn a_flex_row_measures_a_text_leaf_once_for_its_base_size_and_its_minimum() {
        // [asked, ran] as [min, max, definite, placement], then measures.
        let want: [(&str, [u64; 4], [u64; 4], u64); 5] = [
            ("text row", [8, 8, 0, 8], [8, 8, 0, 9], 16),
            ("nested flex", [212, 164, 0, 36], [52, 84, 0, 37], 32),
            ("card grid", [54, 30, 0, 18], [18, 6, 0, 19], 12),
            ("list window", [132, 24, 0, 48], [96, 0, 0, 49], 48),
            ("wrapping row", [4, 4, 0, 4], [4, 4, 0, 5], 8),
        ];
        for ((name, el), (n, asked, ran, measures)) in trees().into_iter().zip(want) {
            let (s, _) = laid(&el);
            assert_eq!(name, n);
            assert_eq!(
                (kinds(s.asked), kinds(s.ran), s.measures),
                (asked, ran, measures),
                "{name}: asked, ran, measures"
            );
        }
    }

    /// The boxes of every tree, to the bit, as they were before a leaf
    /// measured its minimum with its base size.
    #[test]
    fn the_boxes_are_the_same_to_the_bit() {
        let want: [(&str, u64); 5] = [
            ("text row", 17_048_616_598_677_835_447),
            ("nested flex", 7_265_697_887_972_996_053),
            ("card grid", 16_836_433_504_534_575_509),
            ("list window", 13_040_370_934_046_842_613),
            ("wrapping row", 7_065_004_713_279_509_305),
        ];
        let got = trees().map(|(name, el)| {
            let (_, rects) = laid(&el);
            let hash = rects
                .iter()
                .flat_map(|r| [r.0, r.1, r.2, r.3])
                .fold(0xcbf2_9ce4_8422_2325_u64, |h, v| {
                    (h ^ u64::from(v)).wrapping_mul(0x0100_0000_01b3)
                });
            (name, hash)
        });
        assert_eq!(got, want);
    }

    /// Every run of an algorithm is an ask, the root's own aside: the
    /// requests a paragraph puts to the elements in its lines are counted
    /// as a parent's are.
    #[test]
    fn a_paragraphs_requests_to_its_inline_children_are_asks() {
        let el = div()
            .display(Display::Flex)
            .child(
                text("some words ")
                    .child(div().width(12.0).height(30.0).child(text("x")))
                    .span(" and more"),
            )
            .into_element();
        let (s, _) = laid(&el);
        assert!(s.asked.total() > 0, "the paragraph's child was asked");
        assert!(s.ran.total() <= s.asked.total() + 1, "{s:?}");
        for (name, el) in trees() {
            let (s, _) = laid(&el);
            assert!(s.ran.total() <= s.asked.total() + 1, "{name}: {s:?}");
        }
    }

    fn plain() -> ResolvedText {
        ResolvedText::root(Color::BLACK)
    }

    fn text_leaf(content: &str, t: ResolvedText, runs: Vec<Run>) -> Leaf {
        Leaf::Text {
            content: content.into(),
            text: t,
            runs: runs.into(),
            objects: Vec::new(),
        }
    }

    fn input_leaf(content: &str, multiline: bool) -> Leaf {
        Leaf::Input {
            content: content.into(),
            text: plain(),
            multiline,
        }
    }

    /// What `measure_pair` answers, each with its own text system, and what
    /// two `measure` calls answer: sizes and baselines, to the bit.
    fn same_pair(name: &str, leaf: &Leaf) {
        use super::engine::Measure;
        for known in [
            Size {
                width: None,
                height: None,
            },
            Size {
                width: None,
                height: Some(21.0),
            },
        ] {
            for cross in [
                AvailableSpace::MaxContent,
                AvailableSpace::MinContent,
                AvailableSpace::Definite(50.0),
            ] {
                let max = Size {
                    width: AvailableSpace::MaxContent,
                    height: cross,
                };
                let min = Size {
                    width: AvailableSpace::MinContent,
                    height: cross,
                };
                let ([a, b], passes) = leaf.measure_pair(&mut TextSystem::new(), known, max, min);
                let (c, d) = (
                    leaf.measure(&mut TextSystem::new(), known, max),
                    leaf.measure(&mut TextSystem::new(), known, min),
                );
                assert_eq!(
                    format!("{a:?}{b:?}"),
                    format!("{c:?}{d:?}"),
                    "{name} {known:?} {cross:?}"
                );
                let bits = |m: super::engine::Measured| (m.size.width.to_bits(), m.size.height.to_bits());
                assert_eq!((bits(a), bits(b)), (bits(c), bits(d)), "{name}: size bits");
                let shares = matches!(leaf, Leaf::Text { .. });
                assert_eq!(passes, if shares { 1 } else { 2 }, "{name}: passes");
            }
        }
    }

    #[test]
    fn a_pair_measure_is_two_measures_to_the_bit() {
        const LONG: &str = "Several words that wrap, and one unbrokenAndVeryLongWordIndeed, then more words after it";
        let bold = ResolvedText {
            weight: 700,
            size: 22.0,
            ..plain()
        };
        let styled = vec![
            Run {
                range: 8..14,
                text: bold,
                background: Some(Color::BLACK),
                radius: 2.0,
                padding: Edges {
                    top: 3.0,
                    right: 4.0,
                    bottom: 5.0,
                    left: 6.0,
                },
            },
            Run {
                range: 20..25,
                text: ResolvedText {
                    italic: true,
                    ..plain()
                },
                background: None,
                radius: 0.0,
                padding: Edges::default(),
            },
        ];
        let leaves = [
            ("plain", text_leaf(LONG, plain(), vec![])),
            ("one word", text_leaf("word", plain(), vec![])),
            ("empty", text_leaf("", plain(), vec![])),
            ("styled runs", text_leaf(LONG, plain(), styled)),
            (
                "font fallback",
                text_leaf("日本語のテキスト mixed 😀 emoji and العربية", plain(), vec![]),
            ),
            (
                "nowrap",
                text_leaf(
                    LONG,
                    ResolvedText {
                        nowrap: true,
                        ..plain()
                    },
                    vec![],
                ),
            ),
            (
                "truncate",
                text_leaf(
                    LONG,
                    ResolvedText {
                        truncate: true,
                        nowrap: true,
                        ..plain()
                    },
                    vec![],
                ),
            ),
            (
                "line clamp",
                text_leaf(
                    LONG,
                    ResolvedText {
                        line_clamp: Some(2),
                        ..plain()
                    },
                    vec![],
                ),
            ),
            (
                "centered",
                text_leaf(
                    LONG,
                    ResolvedText {
                        align: TextAlign::Center,
                        ..plain()
                    },
                    vec![],
                ),
            ),
            ("input", input_leaf("typed text in a field", false)),
            (
                "textarea",
                input_leaf("typed text in a field that wraps over lines", true),
            ),
            ("empty textarea", input_leaf("", true)),
        ];
        for (name, leaf) in &leaves {
            same_pair(name, leaf);
        }
    }

    /// Text and width change between two layouts of one retained tree: the
    /// boxes are a fresh tree's, to the bit.
    #[test]
    fn a_retained_row_laid_again_after_its_text_and_width_change_is_a_fresh_one() {
        let row = |words: &str, width: f32| {
            div()
                .display(Display::Flex)
                .width(width)
                .gap(4.0)
                .children((0..3).map(|i| text(format!("{words} {i}"))))
                .into_element()
        };
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        for (words, width) in [
            ("a few words", 400.0),
            ("a few more words that wrap", 400.0),
            ("a few more words that wrap", 120.0),
            ("short", 120.0),
            ("short", 300.0),
        ] {
            let el = row(words, width);
            tree.layout(&el, 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
            let (_, fresh) = laid(&el);
            assert_eq!(exact(tree.frame()), fresh, "{words:?} at {width}");
        }
    }

    /// A leaf that was a block's child (only placed, in its inherent sizing
    /// mode) becomes a flex row's: the row's first probe finds the sizing
    /// mode changed and clears what the leaf cached, but not what its pair
    /// measure kept, so the leaf is measured once for both probes, not twice
    /// (12 measures, not 8, without it).
    #[test]
    fn a_pair_measure_kept_survives_a_change_of_sizing_mode() {
        let el = |row: bool| {
            div()
                .display(if row { Display::Flex } else { Display::Block })
                .width(400.0)
                .children((0..4).map(|i| text(format!("word {i} and more"))))
                .into_element()
        };
        let mut tree = LayoutTree::new();
        let mut ts = TextSystem::new();
        let state = UiState::default();
        tree.layout(&el(false), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        tree.layout(&el(true), 400.0, 300.0, &state, &mut Motion::default(), &mut ts);
        let (stats, moved) = (tree.stats(), exact(tree.frame()));
        let (fresh_stats, fresh) = laid(&el(true));
        assert_eq!(moved, fresh, "the boxes of a fresh tree");
        // Four leaves: one pair measure each for the two probes, and one
        // for the final layout, at most as many as a fresh tree's.
        assert!(
            stats.measures <= fresh_stats.measures,
            "{} measures, a fresh tree's {}",
            stats.measures,
            fresh_stats.measures
        );
        assert_eq!(stats.measures, 4 * 2, "{stats:?}");
    }
}
