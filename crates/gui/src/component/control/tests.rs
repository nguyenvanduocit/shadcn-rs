//! Control nodes (ADR-0026, K4a): each invariant of K3 that K4a holds, the
//! Zgui and Burin cases (research 2026-11-24), and the cold check's proof:
//! a suppressed insert or move fails it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::super::{Root, component, on_unmounted, take_due};
use super::*;
use crate::element::{div, text};
use crate::reactive::{Scope, Signal};

fn owned_root(render: impl Fn() -> Element + 'static) -> Root {
    Scope::detached().run(|| Root::new(render))
}

/// The rows a test's views were handed, keyed by `u32`.
type Seen<T> = Rc<RefCell<Vec<Row<T, u32>>>>;

fn bump(c: &Cell<u32>) {
    c.set(c.get().saturating_add(1));
}

/// The texts of `el` in order, and the key each text's node is placed by.
fn placed(el: &Element) -> Vec<(String, Option<ElementKey>)> {
    let mut out = Vec::new();
    collect(el, &mut out);
    out
}

fn collect(el: &Element, out: &mut Vec<(String, Option<ElementKey>)>) {
    match el {
        Element::Text(t) => out.push((t.kind.content.clone(), t.common.component_key)),
        Element::Div(d) => d.kind.iter().for_each(|c| collect(c, out)),
        Element::Deferred(d) => collect(&d.child, out),
        _ => {}
    }
}

fn texts(el: &Element) -> Vec<String> {
    placed(el).into_iter().map(|(t, _)| t).collect()
}

/// How many children the root box was given.
fn kids(el: &Element) -> usize {
    el.kids().map_or(0, <[Element]>::len)
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

/// A box: "head", one text row per item, "tail". `views` counts the views
/// run.
fn listing(items: Signal<Vec<u32>>, views: &Rc<Cell<u32>>) -> impl Fn() -> Element + 'static {
    let views = Rc::clone(views);
    move || {
        let views = Rc::clone(&views);
        div()
            .child(text("head"))
            .child(each(
                move || items.get(),
                |i| *i,
                move |row| {
                    bump(&views);
                    text(row.item_untracked().to_string())
                },
            ))
            .child(text("tail"))
            .into_element()
    }
}

#[test]
fn a_rows_view_runs_once_and_a_reorder_mounts_nothing() {
    let items = Signal::new(vec![1_u32, 2, 3, 4]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    let first = root.resolve();
    assert_eq!(texts(&first), ["head", "1", "2", "3", "4", "tail"]);
    assert_eq!(
        kids(&first),
        6,
        "the rows are the box's own children: no box of their own"
    );
    assert_eq!(views.get(), 4);
    let before = structure_stats();
    items.set(vec![4, 3, 2, 1]);
    assert!(take_due());
    assert_eq!(texts(&root.resolve()), ["head", "4", "3", "2", "1", "tail"]);
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts), (0, 0), "a reorder mounts and disposes nothing");
    assert_eq!(d.moves, 3, "a reversal of four keeps one row and moves three");
    assert_eq!(views.get(), 4, "no view runs again");
}

/// Zgui and Burin: a rotation by one place is one move, and every row is
/// the one it was (the key its node is placed by).
#[test]
fn a_one_place_rotation_is_one_move_and_keeps_every_rows_identity() {
    let items = Signal::new((0_u32..10).collect::<Vec<_>>());
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    let before_keys = placed(&root.resolve());
    let before = structure_stats();
    items.set((1_u32..10).chain([0]).collect());
    let after_keys = placed(&root.resolve());
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts, d.moves), (0, 0, 1));
    for (text, key) in &before_keys {
        let now = after_keys.iter().find(|(t, _)| t == text).map(|(_, k)| *k);
        assert_eq!(now, Some(*key), "{text} keeps the key its node is placed by");
    }
    // And back the other way: one move again.
    let before = structure_stats();
    items.set((0_u32..10).collect());
    root.resolve();
    assert_eq!(since(before).moves, 1);
}

#[test]
fn inserts_and_removals_keep_the_rows_around_them() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    let first = placed(&root.resolve());
    let before = structure_stats();
    items.set(vec![0, 1, 3, 5]);
    let now = placed(&root.resolve());
    assert_eq!(texts(&root.resolve()), ["head", "0", "1", "3", "5", "tail"]);
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts, d.moves), (2, 1, 0));
    for kept in ["1", "3"] {
        let key = |v: &[(String, Option<ElementKey>)]| v.iter().find(|(t, _)| t == kept).map(|(_, k)| *k);
        assert_eq!(key(&first), key(&now), "{kept} is the same row");
    }
    assert_eq!(views.get(), 5);
}

/// A kept key gets its new data (its item signal); its index is written
/// when it moved; neither runs its view again.
#[test]
fn a_kept_key_gets_its_new_data_and_its_new_index() {
    let items = Signal::new(vec![(1_u32, "a"), (2, "b"), (3, "c")]);
    let rows: Seen<(u32, &'static str)> = Rc::default();
    let r = Rc::clone(&rows);
    let root = owned_root(move || {
        let r = Rc::clone(&r);
        div()
            .child(each(
                move || items.get(),
                |item| item.0,
                move |row| {
                    r.borrow_mut().push(row);
                    text(row.key().to_string())
                },
            ))
            .into_element()
    });
    root.resolve();
    let row_of = |k: u32| rows.borrow().iter().copied().find(|r| r.key() == k).expect("a row");
    assert_eq!(row_of(2).item_untracked(), (2, "b"));
    assert_eq!(row_of(2).index(), 1);
    items.set(vec![(3, "c"), (2, "B"), (1, "a")]);
    root.resolve();
    assert_eq!(rows.borrow().len(), 3, "no view ran again");
    assert_eq!(row_of(2).item_untracked(), (2, "B"), "a kept key gets its new data");
    assert_eq!(
        (row_of(3).index(), row_of(2).index(), row_of(1).index()),
        (0, 1, 2),
        "each index follows its row"
    );
}

/// Duplicate keys (Vapor and Dioxus check only in dev builds; Floem and
/// Leptos lose rows): every item is shown, the n-th equal key is that key's
/// n-th occurrence, and the commit reports it in every build.
#[test]
fn duplicate_keys_show_every_item_and_are_reported() {
    drop(take_duplicate_keys());
    let items = Signal::new(vec![1_u32, 1, 2]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    assert_eq!(texts(&root.resolve()), ["head", "1", "1", "2", "tail"]);
    let found = take_duplicate_keys();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].key.as_str(), found[0].count, found[0].node), ("1", 2, "each"));
    let before = structure_stats();
    items.set(vec![1, 2, 1]);
    assert_eq!(texts(&root.resolve()), ["head", "1", "2", "1", "tail"]);
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts), (0, 0), "(1, #0) and (1, #1) are kept");
}

/// Zgui and Burin: a row's cleanup runs exactly once, after its last
/// unmount; nothing runs it again, and the root's end runs the others'.
#[test]
fn a_rows_cleanup_runs_exactly_once_after_its_unmount() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let cleaned: Rc<RefCell<Vec<u32>>> = Rc::default();
    let c = Rc::clone(&cleaned);
    let scope = Scope::detached();
    let root = scope.run(|| {
        Root::new(move || {
            let c = Rc::clone(&c);
            div()
                .child(each(
                    move || items.get(),
                    |i| *i,
                    move |row| {
                        let c = Rc::clone(&c);
                        let k = row.key();
                        on_unmounted(move || c.borrow_mut().push(k));
                        text(k.to_string())
                    },
                ))
                .into_element()
        })
    });
    root.resolve();
    items.set(vec![1, 3]);
    root.resolve();
    assert_eq!(*cleaned.borrow(), [2]);
    // Moved, kept, resolved again: nothing more.
    items.set(vec![3, 1]);
    root.resolve();
    root.resolve();
    assert_eq!(*cleaned.borrow(), [2]);
    scope.dispose();
    drop(root);
    let mut all = cleaned.borrow().clone();
    all.sort_unstable();
    assert_eq!(all, [1, 2, 3], "each cleanup ran once");
}

/// The source is tracked by the node alone: a write to it runs no render
/// of the component that placed it, and a branch mounts once per mount.
#[test]
fn show_mounts_a_branch_per_mount_and_renders_nothing_around_it() {
    let open = Signal::new(false);
    let renders = Rc::new(Cell::new(0));
    let views = Rc::new(Cell::new(0));
    let (r, v) = (Rc::clone(&renders), Rc::clone(&views));
    let root = owned_root(move || {
        let (r, v) = (Rc::clone(&r), Rc::clone(&v));
        component((), move |_| {
            let (r, v) = (Rc::clone(&r), Rc::clone(&v));
            move || {
                bump(&r);
                let v = Rc::clone(&v);
                div()
                    .child(text("before"))
                    .child(
                        show(
                            move || open.get(),
                            move || {
                                bump(&v);
                                text("open")
                            },
                        )
                        .or_else(|| text("closed")),
                    )
                    .child(text("after"))
                    .into_element()
            }
        })
        .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["before", "closed", "after"]);
    let before = structure_stats();
    open.set(true);
    assert!(take_due());
    assert_eq!(texts(&root.resolve()), ["before", "open", "after"]);
    open.set(false);
    assert_eq!(texts(&root.resolve()), ["before", "closed", "after"]);
    open.set(true);
    assert_eq!(texts(&root.resolve()), ["before", "open", "after"]);
    assert_eq!(renders.get(), 1, "the component around it never renders again");
    assert_eq!(views.get(), 2, "the branch's view ran once per mount");
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts), (3, 3));
}

#[test]
fn dynamic_view_mounts_again_only_for_a_value_that_differs() {
    let which = Signal::new("a");
    let views: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let v = Rc::clone(&views);
    let root = owned_root(move || {
        let v = Rc::clone(&v);
        div()
            .child(dynamic_view(
                move || which.get(),
                move |k: &&'static str| {
                    v.borrow_mut().push(k);
                    text(*k)
                },
            ))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["a"]);
    // Written again, equal: the source runs, nothing mounts.
    which.replace("a");
    assert!(take_due());
    root.resolve();
    which.set("b");
    assert_eq!(texts(&root.resolve()), ["b"]);
    which.set("a");
    assert_eq!(texts(&root.resolve()), ["a"]);
    assert_eq!(*views.borrow(), ["a", "b", "a"]);
}

/// A row's own component instance keeps its state through a move: the
/// row is the same instance tree.
#[test]
fn a_component_inside_a_row_keeps_its_state_through_a_move() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let setups = Rc::new(Cell::new(0));
    let s = Rc::clone(&setups);
    let root = owned_root(move || {
        let s = Rc::clone(&s);
        div()
            .child(each(
                move || items.get(),
                |i| *i,
                move |row| {
                    let s = Rc::clone(&s);
                    let k = row.key();
                    div().child(component(k, move |cx| {
                        bump(&s);
                        let props = cx.props();
                        move || text(props.get().to_string()).into_element()
                    }))
                },
            ))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["1", "2", "3"]);
    items.set(vec![3, 1, 2]);
    assert_eq!(texts(&root.resolve()), ["3", "1", "2"]);
    assert_eq!(setups.get(), 3, "no instance inside a moved row was made again");
}

/// Nested control nodes: an `each` inside a row of an `each`.
#[cfg(debug_assertions)]
#[test]
fn nested_lists_commit_each_on_its_own() {
    let outer = Signal::new(vec![1_u32, 2]);
    let inner = Signal::new(vec![10_u32, 20]);
    let root = owned_root(move || {
        div()
            .child(each(
                move || outer.get(),
                |i| *i,
                move |row| {
                    let k = row.key();
                    div().child(text(k.to_string())).child(each(
                        move || inner.get(),
                        |i| *i,
                        move |r| text(format!("{k}.{}", r.key())),
                    ))
                },
            ))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["1", "1.10", "1.20", "2", "2.10", "2.20"]);
    let before = structure_stats();
    inner.set(vec![20, 10]);
    assert_eq!(texts(&root.resolve()), ["1", "1.20", "1.10", "2", "2.20", "2.10"]);
    let d = since(before);
    assert_eq!((d.mounts, d.unmounts, d.moves), (0, 0, 2));
    assert_eq!(root.cold(|f| texts(&f.resolve())), Some(texts(&root.resolve())));
}

/// A control node where one element is placed (a component's root) gives
/// its one branch.
#[test]
fn a_control_node_at_a_components_root_gives_its_branch() {
    let open = Signal::new(true);
    let root = owned_root(move || {
        show(move || open.get(), || text("yes"))
            .or_else(|| text("no"))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["yes"]);
    open.set(false);
    assert_eq!(texts(&root.resolve()), ["no"]);
}

/// The cold frame builds the same rows from the sources, running no view
/// of a kept row.
#[cfg(debug_assertions)]
#[test]
fn the_cold_frame_lists_the_sources_and_runs_no_kept_view() {
    let items = Signal::new(vec![1_u32, 2, 3, 4]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    root.resolve();
    for next in [vec![4_u32, 1, 2, 3], vec![4, 2], vec![9, 4, 2, 7], vec![]] {
        items.set(next);
        let live = placed(&root.resolve());
        let ran = views.get();
        let cold = root.cold(|f| placed(&f.resolve())).expect("kept");
        assert_eq!(
            cold, live,
            "the cold rows are the live ones, in order, placed by the same keys"
        );
        assert_eq!(
            views.get(),
            ran,
            "the cold frame ran no view: every row it needed is live"
        );
    }
}

#[cfg(debug_assertions)]
fn refereed(root: &Root) -> Result<Vec<String>, String> {
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.cold(|f| texts(&f.resolve()))));
    caught.map(Option::unwrap_or_default).map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    })
}

/// The cold check's proof (ADR-0026): a commit that does not insert a row
/// fails it, by the referee and, apart from it, by the cold rows.
#[cfg(debug_assertions)]
#[test]
fn the_cold_check_fails_when_a_commit_skips_an_insert() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    root.resolve();
    suppress(Some(Suppress::Insert));
    items.set(vec![1, 2, 5, 3]);
    let live = texts(&root.resolve());
    suppress(None);
    assert_eq!(live, ["head", "1", "2", "3", "tail"], "the insert was skipped");
    let said = refereed(&root).expect_err("the referee fails the check");
    assert!(said.contains("key 5 (#0) is in the source and not mounted"), "{said}");
    // Apart from the referee: the cold rows differ from the live ones.
    referee_off(true);
    let cold = root.cold(|f| texts(&f.resolve()));
    referee_off(false);
    assert_eq!(cold.expect("kept"), ["head", "1", "2", "5", "3", "tail"]);
}

/// The same for a keyed move the commit leaves undone.
#[cfg(debug_assertions)]
#[test]
fn the_cold_check_fails_when_a_commit_skips_a_move() {
    let items = Signal::new(vec![1_u32, 2, 3, 4]);
    let views = Rc::new(Cell::new(0));
    let root = owned_root(listing(items, &views));
    root.resolve();
    suppress(Some(Suppress::Move));
    items.set(vec![2, 3, 4, 1]);
    let live = texts(&root.resolve());
    suppress(None);
    assert_eq!(live, ["head", "1", "2", "3", "4", "tail"], "the move was skipped");
    let said = refereed(&root).expect_err("the referee fails the check");
    assert!(
        said.contains("the row of key 1 is out of the source's order in the tree"),
        "{said}"
    );
    referee_off(true);
    let cold = root.cold(|f| texts(&f.resolve()));
    referee_off(false);
    assert_eq!(cold.expect("kept"), ["head", "2", "3", "4", "1", "tail"]);
}

/// A commit that leaves a kept row's data stale fails the referee: the
/// cold rows would show the same stale signal.
#[cfg(debug_assertions)]
#[test]
fn the_referee_checks_each_rows_signals_against_the_source() {
    let items = Signal::new(vec![(1_u32, 'a'), (2, 'b')]);
    let rows: Seen<(u32, char)> = Rc::default();
    let r = Rc::clone(&rows);
    let root = owned_root(move || {
        let r = Rc::clone(&r);
        div()
            .child(each(
                move || items.get(),
                |i| i.0,
                move |row| {
                    r.borrow_mut().push(row);
                    text(row.key().to_string())
                },
            ))
            .into_element()
    });
    root.resolve();
    assert_eq!(refereed(&root), Ok(vec!["1".to_owned(), "2".to_owned()]));
    // Written behind the commit's back: what a commit that forgot the
    // write would leave.
    let first = rows.borrow().first().copied().expect("a row");
    first.item.set((1, 'z'));
    let said = refereed(&root).expect_err("stale data fails the check");
    assert!(said.contains("holds an item the source no longer has"), "{said}");
}

/// Commits with no change of rows allocate the same, however many rows: no
/// allocation per unchanged row. And a rotation of 10 thousand rows is one
/// move.
#[cfg(debug_assertions)]
#[test]
fn a_commit_allocates_nothing_per_unchanged_row() {
    fn per_commit(n: u32) -> (u64, u64) {
        let base: Rc<Vec<u32>> = Rc::new((0..n).collect());
        let rotated: Rc<Vec<u32>> = Rc::new((1..n).chain([0]).collect());
        let flip = Signal::new(false);
        let (b, r) = (Rc::clone(&base), Rc::clone(&rotated));
        let root = owned_root(move || {
            let (b, r) = (Rc::clone(&b), Rc::clone(&r));
            div()
                .child(each(
                    move || if flip.get() { (*r).clone() } else { (*b).clone() },
                    |i| *i,
                    |row| text(row.key().to_string()),
                ))
                .into_element()
        });
        root.resolve();
        let before = structure_stats();
        let ((), rotate) = crate::heap::allocations(|| {
            flip.set(true);
            root.resolve();
        });
        assert_eq!(since(before).moves, 1, "a rotation is one move");
        // The same items again (the source runs, nothing changes).
        let ((), same) = crate::heap::allocations(|| {
            flip.set(false);
            flip.set(true);
            crate::reactive::batch(|| ());
            root.resolve();
        });
        (rotate, same)
    }
    let small = per_commit(1_000);
    let large = per_commit(10_000);
    eprintln!("allocations per commit (rotate, unchanged): 1k rows {small:?}, 10k rows {large:?}");
    // The source's own `Vec` clone is one allocation either way; the
    // commit's buffers are a fixed few, not one per row.
    assert!(
        large.0 <= small.0.saturating_add(8),
        "rotation: {small:?} rows 1k, {large:?} rows 10k"
    );
    assert!(
        large.1 <= small.1.saturating_add(8),
        "no change: {small:?} rows 1k, {large:?} rows 10k"
    );
    assert!(large.0 < 100, "a rotation of 10k rows allocates {} times", large.0);
}

/// Timing of a keyed reorder and a rotation of 10 thousand rows, in the
/// build it runs in: `cargo test -p gui --release -- --ignored k4a_timing --nocapture`.
#[test]
#[ignore = "a measurement, not a check"]
fn k4a_timing() {
    let n = 10_000_u32;
    let order = Signal::new((0..n).collect::<Vec<u32>>());
    let root = owned_root(move || {
        div()
            .child(each(move || order.get(), |i| *i, |row| text(row.key().to_string())))
            .into_element()
    });
    let t = std::time::Instant::now();
    root.resolve();
    eprintln!("mount 10k rows: {:?}", t.elapsed());
    let runs = 20_u32;
    let mut reversed = std::time::Duration::ZERO;
    let mut rotated = std::time::Duration::ZERO;
    let mut shuffled = std::time::Duration::ZERO;
    let mut seed = 0x2545_f491_u64;
    for _ in 0..runs {
        let mut v = order.get_untracked();
        v.reverse();
        let t = std::time::Instant::now();
        order.set(v);
        root.resolve();
        reversed = reversed.saturating_add(t.elapsed());
        let mut v = order.get_untracked();
        v.rotate_left(1);
        let t = std::time::Instant::now();
        order.set(v);
        root.resolve();
        rotated = rotated.saturating_add(t.elapsed());
        let mut v = order.get_untracked();
        for i in (1..v.len()).rev() {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let j = usize::try_from(seed % u64::try_from(i.saturating_add(1)).unwrap_or(1)).unwrap_or(0);
            v.swap(i, j);
        }
        let t = std::time::Instant::now();
        order.set(v);
        root.resolve();
        shuffled = shuffled.saturating_add(t.elapsed());
    }
    eprintln!(
        "10k rows, per commit and resolve: reverse {:?}, rotate by one {:?}, shuffle {:?}",
        reversed / runs,
        rotated / runs,
        shuffled / runs
    );
}

/// Rows of `height` 20 in a column, each a box holding its key as text.
fn window(items: Signal<Vec<u64>>) -> impl Fn() -> Element + 'static {
    use crate::styled::Styled;
    move || {
        div()
            .width(200.0)
            .child(each(
                move || items.get(),
                |i| *i,
                |row| div().width(200.0).height(20.0).child(text(row.key().to_string())),
            ))
            .into_element()
    }
}

/// What names the node `text` shows in: its path (bounds and hit testing
/// find it by it), its accessibility id and the compositor's key (paint).
fn ids(s: &crate::Scenario, text: &str) -> (crate::geometry::NodePath, u64, crate::compositor::NodeKey) {
    let node = crate::inspect::find_text(&s.last().nodes, text).expect("on screen");
    (
        node.path.clone(),
        crate::access::id_of(&node.path).0,
        crate::compositor::NodeKey::of(&node.path),
    )
}

/// Zgui and Burin: a window near the end of 10 million rows shifted by one
/// row recycles one row (one out, one in) and keeps every other row's key:
/// its path, which bounds, paint, hit testing and accessibility are keyed
/// by. The same counts with 10 thousand source rows.
#[test]
fn a_window_near_the_end_of_ten_million_rows_keeps_each_kept_rows_key() {
    for total in [10_000_u64, 10_000_000] {
        let start = total.saturating_sub(30);
        let items = Signal::new((start..start.saturating_add(20)).collect::<Vec<u64>>());
        let mut s = crate::Scenario::measuring(window(items), 200, 400, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        let kept = total.saturating_sub(20).to_string();
        let before = ids(&s, &kept);
        let gone = ids(&s, &start.to_string());
        let stats = structure_stats();
        s.act("shift", || {
            items.set((start.saturating_add(1)..start.saturating_add(21)).collect())
        });
        s.frame("shifted").expect("a frame");
        let d = since(stats);
        assert_eq!(
            (d.mounts, d.unmounts, d.moves),
            (1, 1, 0),
            "{total} rows: one in, one out"
        );
        assert_eq!(ids(&s, &kept), before, "{total} rows: a kept row keeps its key");
        let came = ids(&s, &start.saturating_add(20).to_string());
        assert_ne!(came.0, gone.0, "no row is pooled across keys");
        assert!(crate::inspect::find_text(&s.last().nodes, &start.to_string()).is_none());
    }
}

/// Focus belongs to a key: a focused row moved by key keeps focus, and its
/// node its key.
#[test]
fn focus_moves_with_its_row() {
    use crate::focus::FocusHandle;
    use crate::styled::Styled;
    let order = Signal::new(vec![1_u32, 2, 3]);
    let handles: Rc<RefCell<Vec<(u32, FocusHandle)>>> = Rc::default();
    let h = Rc::clone(&handles);
    let mut s = crate::Scenario::measuring(
        move || {
            let h = Rc::clone(&h);
            div()
                .child(each(
                    move || order.get(),
                    |i| *i,
                    move |row| {
                        let focus = FocusHandle::new();
                        h.borrow_mut().push((row.key(), focus));
                        div()
                            .width(100.0)
                            .height(20.0)
                            .track_focus(focus)
                            .child(text(row.key().to_string()))
                    },
                ))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let two = handles
        .borrow()
        .iter()
        .find(|(k, _)| *k == 2)
        .map(|(_, f)| *f)
        .expect("row 2");
    s.act("focus", || two.focus());
    s.frame("focused").expect("a frame");
    let at = s.read(|| crate::focus::path_of(two.id()));
    assert!(at.is_some());
    s.act("reorder", || order.set(vec![3, 2, 1]));
    s.frame("moved").expect("a frame");
    s.act("reorder", || order.set(vec![2, 3, 1]));
    s.frame("moved again").expect("a frame");
    assert_eq!(handles.borrow().len(), 3, "no row mounted again");
    assert_eq!(s.read(crate::focus::focused), Some(two.id()), "focus stays on row 2");
    assert_eq!(s.read(|| crate::focus::path_of(two.id())), at, "and on its node");
}

/// The pipeline's cold check (`GUI_COLD_CHECK`) passes a run of commits,
/// and fails on a skipped insert and on a skipped move: by the referee,
/// and with the referee off, by the cold layout alone.
#[cfg(debug_assertions)]
#[test]
fn the_pipelines_cold_check_fails_on_a_skipped_insert_or_move() {
    use crate::engine::{CheckReport, Mismatch};
    for fault in [Suppress::Insert, Suppress::Move] {
        let items = Signal::new(vec![1_u64, 2, 3, 4]);
        let mut s = crate::Scenario::measuring(window(items), 200, 400, 1.0).expect("a scenario");
        s.check_cold(true);
        s.frame("first").expect("a frame");
        s.act("reorder", || items.set(vec![4, 1, 2, 3]));
        s.frame("reordered").expect("a frame");
        s.act("change", || items.set(vec![4, 7, 2]));
        s.frame("changed").expect("a frame");
        s.check_cold(false);
        suppress(Some(fault));
        s.act("faulty", || items.set(vec![2, 9, 7, 4]));
        s.frame("faulty").expect("a frame");
        suppress(None);
        let report = s.check_report();
        assert!(
            matches!(&report, CheckReport::Mismatch(Mismatch::Panicked(said)) if said.contains("structural referee")),
            "{fault:?}: {report:?}"
        );
        referee_off(true);
        let report = s.check_report();
        referee_off(false);
        assert!(
            matches!(&report, CheckReport::Mismatch(Mismatch::Layout(_))),
            "{fault:?}, the referee off: {report:?}"
        );
    }
}

// Round 2 (review r1): each repro below failed before its fix.

/// A: a range reaches its parent's real children through nested control
/// nodes (Codex r1-1): `show { each }` as a box's child gives both rows.
#[test]
fn a_nested_range_reaches_the_box_through_a_branch() {
    let items = Signal::new(vec![1_u32, 2]);
    let root = owned_root(move || {
        div()
            .child(text("head"))
            .child(show(
                || true,
                move || each(move || items.get(), |i| *i, |r| text(r.key().to_string())),
            ))
            .into_element()
    });
    let out = root.resolve();
    assert_eq!(texts(&out), ["head", "1", "2"]);
    assert_eq!(kids(&out), 3, "the inner rows are the box's own children");
    items.set(vec![2, 3, 1]);
    assert_eq!(texts(&root.resolve()), ["head", "2", "3", "1"]);
}

/// A: a row whose view is itself a list gives the inner rows, spliced.
#[test]
fn a_rows_view_that_is_a_list_splices_its_rows() {
    let root = owned_root(move || {
        div()
            .child(each(
                || vec![1_u32, 2],
                |i| *i,
                |r| {
                    let k = r.key();
                    each(
                        move || vec![10_u32, 20],
                        |i| *i,
                        move |s| text(format!("{k}.{}", s.key())),
                    )
                },
            ))
            .into_element()
    });
    let out = root.resolve();
    assert_eq!(texts(&out), ["1.10", "1.20", "2.10", "2.20"]);
    assert_eq!(kids(&out), 4);
}

/// A: where no box takes a range (a component's root, a layer), it is
/// placed in one implicit box, live and cold alike: nothing is dropped.
#[test]
fn a_range_where_one_element_goes_is_boxed_whole() {
    let items = Signal::new(vec![1_u32, 2, 3]);
    let root = owned_root(move || each(move || items.get(), |i| *i, |r| text(r.key().to_string())).into_element());
    let out = root.resolve();
    assert_eq!(texts(&out), ["1", "2", "3"]);
    assert_eq!(kids(&out), 3, "one implicit box holds the rows");
    items.set(vec![3]);
    let out = root.resolve();
    assert_eq!((texts(&out), kids(&out)), (vec!["3".to_owned()], 1), "still the box");
}

/// A: the cold check passes on nested ranges, and fails on each break
/// of the commit's records or of what reaches the tree.
#[cfg(debug_assertions)]
#[test]
fn the_referee_fails_on_each_break_and_passes_once_reverted() {
    let cases: [(Suppress, &str); 5] = [
        (Suppress::Insert, "in the source and not mounted"),
        (Suppress::Move, "out of the source's order"),
        (Suppress::DropRecord, "not in the node's rows"),
        (Suppress::SwapInstances, "names the instance of another row"),
        (Suppress::Truncate, "is not in the tree"),
    ];
    for (fault, said_part) in cases {
        for faulty in [false, true] {
            let items = Signal::new(vec![1_u32, 2, 3, 4]);
            let root = owned_root(move || {
                div()
                    .child(each(
                        move || items.get(),
                        |i| *i,
                        |r| {
                            let k = r.key();
                            show(
                                || true,
                                move || each(move || vec![k, k + 100], |i| *i, |s| text(s.key().to_string())),
                            )
                        },
                    ))
                    .into_element()
            });
            root.resolve();
            suppress(faulty.then_some(fault));
            // One inserted (9), one moved (1), one removed (4).
            items.set(vec![2, 3, 9, 1]);
            root.resolve();
            suppress(None);
            match (faulty, refereed(&root)) {
                (false, Ok(_)) => {}
                (true, Err(said)) => assert!(said.contains(said_part), "{fault:?}: {said}"),
                (f, r) => panic!("{fault:?}, faulty {f}: {r:?}"),
            }
        }
    }
}

/// C (Codex r1-2): an item not equal to itself no longer compiles
/// (`each`'s and `dynamic_view`'s compile-fail docs); an item whose
/// equality is total, checked by the referee, passes.
#[cfg(debug_assertions)]
#[test]
fn items_of_total_equality_pass_the_referee() {
    #[derive(Clone, Debug)]
    struct Bits(f32);
    impl PartialEq for Bits {
        fn eq(&self, other: &Self) -> bool {
            self.0.to_bits() == other.0.to_bits()
        }
    }
    impl Eq for Bits {}
    let root = owned_root(|| {
        div()
            .child(each(|| vec![Bits(f32::NAN)], |_| 0_u32, |_| text("nan")))
            .into_element()
    });
    root.resolve();
    assert!(refereed(&root).is_ok(), "{:?}", refereed(&root));
}

/// B: a subtree is torn down in one order (ADR-0026, "Teardown has one
/// order"): its children the last placed first, not in hash order.
#[test]
fn teardown_runs_rows_in_one_order_the_last_placed_first() {
    let runs: Vec<Vec<u32>> = (0..2)
        .map(|_| {
            let open = Signal::new(true);
            let order = Signal::new((0_u32..16).collect::<Vec<_>>());
            let log: Rc<RefCell<Vec<u32>>> = Rc::default();
            let l = Rc::clone(&log);
            let root = owned_root(move || {
                let l = Rc::clone(&l);
                div()
                    .child(show(
                        move || open.get(),
                        move || {
                            let l = Rc::clone(&l);
                            div().child(each(
                                move || order.get(),
                                |i| *i,
                                move |r| {
                                    let (l, k) = (Rc::clone(&l), r.key());
                                    on_unmounted(move || l.borrow_mut().push(k));
                                    text(k.to_string())
                                },
                            ))
                        },
                    ))
                    .into_element()
            });
            root.resolve();
            // Reordered first: list order is not mount order.
            order.set((0_u32..16).rev().collect());
            root.resolve();
            open.set(false);
            root.resolve();
            log.take()
        })
        .collect();
    assert_eq!(
        runs[0],
        (0_u32..16).collect::<Vec<_>>(),
        "the last in list order goes first"
    );
    assert_eq!(runs[0], runs[1]);
}

/// D: an unkeyed sibling of a control node keeps its path (so its scroll,
/// motion and accessibility id) as the range before it grows and shrinks.
#[test]
fn a_sibling_keeps_its_path_as_a_range_before_it_changes() {
    use crate::styled::Styled;
    let open = Signal::new(false);
    let items = Signal::new(vec![1_u64, 2]);
    let mut s = crate::Scenario::measuring(
        move || {
            div()
                .width(200.0)
                .child(show(move || open.get(), || text("row")))
                .child(each(move || items.get(), |i| *i, |r| text(r.key().to_string())))
                .child(text("A"))
                .child(text("B"))
                .into_element()
        },
        200,
        400,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let (a, b) = (ids(&s, "A"), ids(&s, "B"));
    s.act("open", || open.set(true));
    s.frame("opened").expect("a frame");
    assert_eq!((ids(&s, "A"), ids(&s, "B")), (a.clone(), b.clone()), "opening a branch");
    s.act("insert", || items.set(vec![0, 1, 2, 3]));
    s.frame("inserted").expect("a frame");
    assert_eq!((ids(&s, "A"), ids(&s, "B")), (a, b), "inserting rows");
}

// Round 3 (review r2): each repro below failed before its fix.

fn path_of(s: &crate::Scenario, text: &str) -> crate::geometry::NodePath {
    crate::inspect::find_text(&s.last().nodes, text)
        .expect("on screen")
        .path
        .clone()
}

/// A (Opus r2-1): an unkeyed component whose root is a range keeps its
/// static place beside a branch that opens.
#[test]
fn a_component_whose_root_is_a_range_keeps_its_path_beside_a_branch() {
    let open = Signal::new(false);
    let mut s = crate::Scenario::measuring(
        move || {
            div()
                .child(show(move || open.get(), || text("row")))
                .child(component((), |_| {
                    || each(|| vec![1_u32, 2], |i| *i, |r| text(format!("c{}", r.key()))).into_element()
                }))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let before = path_of(&s, "c1");
    s.act("open", || open.set(true));
    s.frame("opened").expect("a frame");
    assert_eq!(path_of(&s, "c1"), before);
}

/// A (Opus r2-1): keyed components whose root is a range keep their paths
/// as they are reordered.
#[test]
fn keyed_components_whose_root_is_a_range_keep_their_path_on_reorder() {
    let order = Signal::new(vec![1_u64, 2]);
    let mut s = crate::Scenario::measuring(
        move || {
            let mut d = div();
            for k in order.get() {
                d = d.child(
                    component((), move |_| {
                        move || show(|| true, move || text(format!("k{k}"))).into_element()
                    })
                    .key(k),
                );
            }
            d.into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("first").expect("a frame");
    let before = path_of(&s, "k1");
    s.act("swap", || order.set(vec![2, 1]));
    s.frame("swapped").expect("a frame");
    assert_eq!(path_of(&s, "k1"), before);
}

/// A (Codex r2-1): a row's component whose root is a range keeps its path
/// through a keyed move.
#[test]
fn component_root_range_keeps_path_on_move() {
    let items = Signal::new(vec![1_u32, 2]);
    let mut s = crate::Scenario::measuring(
        move || {
            div()
                .child(each(
                    move || items.get(),
                    |i| *i,
                    |row| {
                        let k = row.key();
                        component((), move |_| {
                            move || show(|| true, move || text(k.to_string())).into_element()
                        })
                    },
                ))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.frame("mount").expect("a frame");
    let before = path_of(&s, "1");
    s.act("swap", || items.set(vec![2, 1]));
    s.frame("swap").expect("a frame");
    assert_eq!(path_of(&s, "1"), before);
}

/// A: the referee checks each implicit box sits at its owner's place;
/// a box that loses it fails the check (revert proof by fault).
#[cfg(debug_assertions)]
#[test]
fn the_referee_fails_a_box_away_from_its_owners_place() {
    for faulty in [false, true] {
        let root = owned_root(|| {
            div()
                .child(component((), |_| || show(|| true, || text("in")).into_element()).key(7_u64))
                .into_element()
        });
        suppress(faulty.then_some(Suppress::Unplaced));
        root.resolve();
        suppress(None);
        match (faulty, refereed(&root)) {
            (false, Ok(_)) => {}
            (true, Err(said)) => assert!(said.contains("not at its owner's place"), "{said}"),
            (f, r) => panic!("faulty {f}: {r:?}"),
        }
    }
}

/// B (Codex r2-2): rows whose view is a layer pass the cold check.
#[cfg(debug_assertions)]
#[test]
fn deferred_rows_pass_the_referee() {
    use crate::element::{Positioning, deferred};
    let mut s = crate::Scenario::measuring(
        || {
            div()
                .child(each(
                    || vec![1_u32, 2],
                    |i| *i,
                    |row| deferred(Positioning::Fill, text(row.key().to_string())),
                ))
                .into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.check_cold(true);
    s.frame("mount").expect("a frame");
}

/// C (Opus r2-2): a key that stays duplicated is reported once, not once
/// per commit; reported again only after it was resolved and came back.
#[test]
fn a_duplicate_key_is_reported_once_while_it_stays() {
    drop(take_duplicate_keys());
    let tick = Signal::new(0_u32);
    let root = owned_root(move || {
        div()
            .child(each(
                move || {
                    let t = tick.get();
                    if t == 500 {
                        vec![(0_u32, t), (1, t)]
                    } else {
                        vec![(0_u32, t), (0, t)]
                    }
                },
                |i| i.0,
                |r| text(r.key().to_string()),
            ))
            .into_element()
    });
    root.resolve();
    for n in 1..1000_u32 {
        tick.set(n);
        root.resolve();
    }
    assert_eq!(
        take_duplicate_keys().len(),
        2,
        "once, then once more after it came back"
    );
}

/// D: a commit that leaves a row behind (its record, instance and node
/// kept as the source empties) fails the referee.
#[cfg(debug_assertions)]
#[test]
fn the_referee_fails_a_stale_row() {
    for faulty in [false, true] {
        let items = Signal::new(vec![1_u32]);
        let root = owned_root(move || {
            div()
                .child(each(move || items.get(), |i| *i, |r| text(r.key().to_string())))
                .into_element()
        });
        root.resolve();
        suppress(faulty.then_some(Suppress::KeepStale));
        items.set(Vec::new());
        root.resolve();
        suppress(None);
        match (faulty, refereed(&root)) {
            (false, Ok(_)) => {}
            (true, Err(said)) => assert!(said.contains("is not in the source"), "{said}"),
            (f, r) => panic!("faulty {f}: {r:?}"),
        }
    }
}

/// The longest increasing run picks the rows that stay.
#[test]
fn the_longest_increasing_run_is_found() {
    let stays = longest_increasing(&[1, 2, 3, 0]);
    assert_eq!(stays, [true, true, true, false]);
    let stays = longest_increasing(&[3, NONE, 0, 1, 2]);
    assert_eq!(stays, [false, false, true, true, true]);
    assert_eq!(longest_increasing(&[]), Vec::<bool>::new());
    let stays = longest_increasing(&[2, 1, 0]);
    assert_eq!(stays.iter().filter(|s| **s).count(), 1);
}
