//! The windowed list (ADR-0026, K4c): the window is a keyed `each`. The
//! cases of the plan's M11 list (research 2026-11-24, Zgui and Burin):
//! a rotation by one is one move that keeps identity; a cleanup runs once,
//! after the last unmount; rows recycled near the end of ten million rows
//! are one key for their box, their paint, the hit test and accessibility;
//! the rows built stay as many when the source's row count changes.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use accesskit::{Action, ActionRequest, Role as AccessRole, TreeId};

use super::*;
use crate::app::Scenario;
use crate::component::{StructureStats, component, structure_stats};
use crate::element::text_bound;
use crate::inspect::OUTSIDE;
use crate::layout::Content;
use crate::reactive::{Signal, on_cleanup};
use crate::window::Input;

const MILLIONS: u64 = 10_000_000;

fn since(before: StructureStats) -> StructureStats {
    let now = structure_stats();
    StructureStats {
        commits: now.commits.saturating_sub(before.commits),
        mounts: now.mounts.saturating_sub(before.mounts),
        unmounts: now.unmounts.saturating_sub(before.unmounts),
        moves: now.moves.saturating_sub(before.moves),
    }
}

/// What the rows did, by key: views run, cleanups run, bindings run.
#[derive(Default)]
struct Log {
    views: RefCell<HashMap<ElementKey, u32>>,
    cleanups: RefCell<HashMap<ElementKey, u32>>,
    bindings: Cell<u32>,
}

impl Log {
    fn views(&self, k: ElementKey) -> u32 {
        self.views.borrow().get(&k).copied().unwrap_or(0)
    }

    fn cleanups(&self, k: ElementKey) -> u32 {
        self.cleanups.borrow().get(&k).copied().unwrap_or(0)
    }

    fn total_views(&self) -> u32 {
        self.views.borrow().values().sum()
    }

    fn total_cleanups(&self) -> u32 {
        self.cleanups.borrow().values().sum()
    }
}

fn bump(map: &RefCell<HashMap<ElementKey, u32>>, k: ElementKey) {
    let mut m = map.borrow_mut();
    let n = m.entry(k).or_insert(0);
    *n = n.saturating_add(1);
}

/// A row keyed `k`: as tall as its key says, its text bound to its row of
/// the data; its view and cleanups logged.
fn row_view(r: ListRow, log: &Rc<Log>) -> Element {
    let k = r.key();
    bump(&log.views, k);
    let l = Rc::clone(log);
    on_cleanup(move || bump(&l.cleanups, k));
    let l = Rc::clone(log);
    div()
        .height(if k.is_multiple_of(3) { 40.0 } else { 24.0 })
        .child(text_bound(move || {
            // Live runs: a cold frame runs every binding again.
            if !crate::engine::is_cold() {
                l.bindings.set(l.bindings.get().saturating_add(1));
            }
            format!("row {}", r.index())
        }))
        .into_element()
}

/// A list of `rows()` rows keyed `key(i)`, 300 by 400, in a box `shown`
/// mounts while it holds.
fn page(
    state: ListState,
    rows: Signal<u64>,
    key: impl Fn(u64) -> ElementKey + Clone + 'static,
    log: &Rc<Log>,
    shown: Signal<bool>,
) -> impl Fn() -> Element + 'static {
    let log = Rc::clone(log);
    move || {
        let log = Rc::clone(&log);
        let key = key.clone();
        div()
            .width(300.0)
            .child(crate::component::show(
                move || shown.get(),
                move || {
                    let log = Rc::clone(&log);
                    virtual_each(&state, move || rows.get(), key.clone(), move |r| row_view(r, &log)).height(400.0)
                },
            ))
            .into_element()
    }
}

fn scenario(page: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(page, 300, 400, 1.0).unwrap();
    s.settle().unwrap();
    s
}

/// The rows laid out, in paint order: their row of the data, as their text
/// says, and their box's top.
fn shown_rows(s: &Scenario) -> Vec<(u64, f32)> {
    s.last()
        .nodes
        .iter()
        .filter_map(|n| match &n.content {
            Content::Text { content, .. } => {
                let row = content.strip_prefix("row ")?.parse().ok()?;
                Some((row, n.bounds(OUTSIDE).y))
            }
            _ => None,
        })
        .collect()
}

/// What a scroll over row boundaries did to a list of `rows` rows: the
/// structure counts, the views and the bindings that ran, and the rows
/// built after.
fn wheel_over(rows: u64) -> (StructureStats, u32, u32, u32, usize) {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    let count = Signal::new(rows);
    let mut s = scenario(page(state, count, |i| i, &log, Signal::new(true)));
    s.hover(100.0, 200.0);
    let (before, views, cleanups) = (structure_stats(), log.total_views(), log.total_cleanups());
    log.bindings.set(0);
    for _ in 0..24 {
        s.wheel(100.0);
        s.settle().unwrap();
    }
    assert!(state.anchor().row > 60, "{:?}", state.anchor());
    let stats = since(before);
    (
        stats,
        log.total_views().saturating_sub(views),
        log.total_cleanups().saturating_sub(cleanups),
        log.bindings.get(),
        shown_rows(&s).len(),
    )
}

/// A wheel over row boundaries mounts the rows that came and disposes the
/// rows that left; the rows kept run nothing (no view, no binding) and none
/// moves. The same at ten thousand and ten million rows (M11).
#[test]
fn a_wheel_mounts_what_came_and_disposes_what_left_alike_at_ten_thousand_and_ten_million() {
    let small = wheel_over(10_000);
    let large = wheel_over(MILLIONS);
    let (stats, views, cleanups, bindings, built) = large;
    assert!(stats.mounts > 0, "the window moved: {stats:?}");
    assert_eq!(stats.moves, 0, "a scroll moves no row");
    assert_eq!(
        u64::from(views),
        stats.mounts,
        "a view ran per mount, none for a kept row"
    );
    assert_eq!(u64::from(cleanups), stats.unmounts, "a cleanup ran per row disposed");
    // The rows that came read their row once as they laid out; a kept
    // row's binding did not run again.
    assert_eq!(u64::from(bindings), stats.mounts, "only the new rows' bindings ran");
    assert!(built < 80, "{built} rows built");
    assert_eq!(
        (
            small.0.mounts,
            small.0.unmounts,
            small.0.moves,
            small.1,
            small.2,
            small.3,
            small.4
        ),
        (
            stats.mounts,
            stats.unmounts,
            stats.moves,
            views,
            cleanups,
            bindings,
            built
        ),
        "ten thousand and ten million rows do the same work"
    );
}

/// The rows built stay as many when the data's row count changes, and none
/// is mounted or disposed: what the total costs is apart from the window.
#[test]
fn the_rows_built_stay_as_many_when_the_source_row_count_changes() {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    let count = Signal::new(10_000);
    let mut s = scenario(page(state, count, |i| i, &log, Signal::new(true)));
    let built = shown_rows(&s);
    let before = structure_stats();
    s.act("ten million rows", || count.set(MILLIONS));
    s.settle().unwrap();
    assert_eq!(shown_rows(&s), built, "the same rows, where they were");
    let stats = since(before);
    assert_eq!((stats.mounts, stats.unmounts, stats.moves), (0, 0, 0), "{stats:?}");
    assert_eq!(state.len(), MILLIONS);
    s.act("back to ten thousand", || count.set(10_000));
    s.settle().unwrap();
    assert_eq!(shown_rows(&s), built);
    let stats = since(before);
    assert_eq!((stats.mounts, stats.unmounts), (0, 0), "{stats:?}");
}

/// Data rotated by one: the first key goes last. One row moves, as itself
/// (no row mounted or disposed, no view run), and each row is written its
/// new row of the data.
#[test]
fn a_rotation_by_one_is_one_move_that_keeps_identity() {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    let order = Signal::new((0..10_u64).collect::<Vec<_>>());
    let key = move |i: u64| {
        order.with(|o| {
            o.get(usize::try_from(i).unwrap_or(usize::MAX))
                .copied()
                .unwrap_or(u64::MAX)
        })
    };
    let mut s = scenario(page(state, Signal::new(10), key, &log, Signal::new(true)));
    assert_eq!(shown_rows(&s).len(), 10);
    let (before, views) = (structure_stats(), log.total_views());
    s.act("rotate by one", || order.update(|o| o.rotate_left(1)));
    s.settle().unwrap();
    let stats = since(before);
    assert_eq!((stats.mounts, stats.unmounts, stats.moves), (0, 0, 1), "{stats:?}");
    assert_eq!(log.total_views(), views, "no view ran again");
    assert_eq!(log.total_cleanups(), 0, "no row was disposed");
    // Each row shows its new row of the data, in data order.
    let shown: Vec<u64> = shown_rows(&s).into_iter().map(|(r, _)| r).collect();
    assert_eq!(shown, (0..10).collect::<Vec<_>>());
    // Key 0, first before, is the last row now: it is 40 tall (its key),
    // the rest as their keys say.
    let last = s
        .last()
        .nodes
        .iter()
        .find(|n| n.path.0.last().and_then(|p| p.element_key()) == Some(0));
    assert!(last.is_some_and(|n| n.bounds(OUTSIDE).height == 40.0 && n.bounds(OUTSIDE).y > 200.0));
}

/// A row's cleanups run once, when it is disposed: scrolled out of the
/// window, or with the list. A row disposed once runs none again.
#[test]
fn a_cleanup_runs_once_after_the_last_unmount() {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    let shown = Signal::new(true);
    let mut s = scenario(page(state, Signal::new(MILLIONS), |i| i, &log, shown));
    assert_eq!(log.cleanups(0), 0);
    s.act("far away", || state.scroll_to_row(5_000));
    s.settle().unwrap();
    assert_eq!(log.cleanups(0), 1, "row 0 left the window");
    assert_eq!(log.views(0), 1);
    s.act("back", || state.scroll_to_row(0));
    s.settle().unwrap();
    assert_eq!(log.views(0), 2, "row 0 came back as a new mount");
    assert_eq!(log.cleanups(0), 1);
    let built = shown_rows(&s).len();
    s.act("the list goes", || shown.set(false));
    s.settle().unwrap();
    assert!(shown_rows(&s).is_empty());
    assert_eq!(log.cleanups(0), 2, "its second mount's cleanup, once");
    // Every mount's cleanup ran exactly once.
    let views = log.views.borrow().clone();
    for (k, n) in views {
        assert_eq!(log.cleanups(k), n, "row {k}");
    }
    assert!(built > 0);
    s.settle().unwrap();
    assert_eq!(log.cleanups(0), 2, "nothing runs again");
}

/// Near the end of ten million rows, rows that leave and come again (their
/// storage reused) are each one key: the box laid out for it, the text it
/// paints, the node a click there hits and the node a screen reader reads
/// are its own row's.
#[test]
fn rows_recycled_near_the_end_of_ten_million_are_one_key_for_bounds_paint_hit_and_access() {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    let mut s = scenario(page(state, Signal::new(MILLIONS), |i| i, &log, Signal::new(true)));
    s.act("near the end", || state.scroll_to_row(MILLIONS - 60));
    s.settle().unwrap();
    s.hover(100.0, 200.0);
    // Each step's accessibility id of each row built, by key: a row kept
    // from one step to the next keeps its id, and no id is two keys'.
    let ids = |s: &Scenario| -> HashMap<ElementKey, crate::geometry::NodePath> {
        s.last()
            .nodes
            .iter()
            .filter_map(|n| {
                let k = n.path.0.last().and_then(|p| p.element_key())?;
                matches!(n.path.0.last(), Some(crate::geometry::PathSeg::Component(..))).then(|| (k, n.path.clone()))
            })
            .collect()
    };
    let mut was = ids(&s);
    assert!(was.len() > 5, "each row is its row instance's root");
    let mut every: HashMap<accesskit::NodeId, ElementKey> = HashMap::new();
    for dy in [120.0, 200.0, -90.0, 300.0, -400.0, 250.0] {
        s.wheel(dy);
        s.settle().unwrap();
        let now = ids(&s);
        for (k, path) in &now {
            if let Some(before) = was.get(k) {
                assert_eq!(
                    crate::access::id_of(before),
                    crate::access::id_of(path),
                    "row {k} kept its id"
                );
            }
            let key = every.entry(crate::access::id_of(path)).or_insert(*k);
            assert_eq!(*key, *k, "an id names one key");
        }
        was = now;
    }
    let rows = shown_rows(&s);
    assert!(rows.len() > 5, "{rows:?}");
    assert!(rows.iter().all(|&(r, _)| (MILLIONS - 80..MILLIONS).contains(&r)));
    // Paint order is data order.
    assert!(rows.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 < w[1].1), "{rows:?}");
    let u = s.pipeline_mut().access_tree(1.0, "page");
    let access: HashMap<accesskit::NodeId, accesskit::Node> = u.nodes.into_iter().collect();
    let frame = s.last();
    let mut checked = 0;
    for (i, n) in frame.nodes.iter().enumerate() {
        let Content::Text { content, .. } = &n.content else {
            continue;
        };
        let Some(row) = content.strip_prefix("row ").and_then(|r| r.parse::<u64>().ok()) else {
            continue;
        };
        // The row node: the text's ancestor keyed as a row of the list.
        let wrapper = frame
            .ancestors(i)
            .find(|&a| frame.nodes[a].path.0.last().and_then(|p| p.element_key()).is_some())
            .unwrap();
        let w = &frame.nodes[wrapper];
        assert_eq!(
            w.path.0.last().and_then(|p| p.element_key()),
            Some(row),
            "its box is its key's"
        );
        let b = n.bounds(OUTSIDE);
        let wb = w.bounds(OUTSIDE);
        assert!(
            b.y >= wb.y && b.y + b.height <= wb.y + wb.height,
            "its text paints in its box"
        );
        // A click in its middle, where the viewport shows it, hits it.
        let (x, y) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
        if (1.0..399.0).contains(&y) {
            let hit = crate::interact::hit(frame, x, y).unwrap();
            assert!(
                std::iter::once(hit).chain(frame.ancestors(hit)).any(|a| a == wrapper),
                "a click on row {row} hits its row"
            );
        }
        // Its accessibility node is its row of the data.
        let id = crate::access::id_of(&w.path);
        let node = access.get(&id).unwrap();
        assert_eq!(node.role(), AccessRole::ListItem);
        assert_eq!(
            node.row_index(),
            usize::try_from(row).ok(),
            "a screen reader reads row {row}"
        );
        checked += 1;
    }
    assert_eq!(checked, rows.len());
}

/// The focused row is kept, the same row, when it scrolls out of the
/// window: pinned, its view not run again, focus on it; a screen reader's
/// `ScrollIntoView` on it brings it back.
#[test]
fn the_focused_row_stays_the_same_row_while_scrolled_away() {
    let state = ListState::new(24.0);
    let handles: Rc<RefCell<HashMap<ElementKey, crate::FocusHandle>>> = Rc::default();
    let log = Rc::new(Log::default());
    let (h, l) = (Rc::clone(&handles), Rc::clone(&log));
    let ui = move || {
        let (h, l) = (Rc::clone(&h), Rc::clone(&l));
        div()
            .width(300.0)
            .child(
                virtual_each(
                    &state,
                    || MILLIONS,
                    |i| i,
                    move |r| {
                        let focus = crate::FocusHandle::new();
                        h.borrow_mut().insert(r.key(), focus);
                        bump(&l.views, r.key());
                        div()
                            .height(24.0)
                            .track_focus(focus)
                            .child(text_bound(move || format!("row {}", r.index())))
                    },
                )
                .height(400.0),
            )
            .into_element()
    };
    let mut s = scenario(ui);
    let focus = handles.borrow().get(&3).copied().unwrap();
    s.act("focus row 3", || focus.focus());
    s.settle().unwrap();
    s.act("far away", || state.scroll_to_row(5_000));
    s.settle().unwrap();
    assert_eq!(s.read(crate::focus::focused), Some(focus.id()), "row 3 keeps focus");
    assert_eq!(state.focused_row(), Some(3));
    assert_eq!(state.pinned(), [3]);
    assert_eq!(log.views(3), 1, "the same row, not mounted again");
    let (_, y) = s.locate("row 3").unwrap();
    assert!(y < 0.0, "kept above the viewport");
    let u = s.pipeline_mut().access_tree(1.0, "page");
    let row3 = u
        .nodes
        .iter()
        .find(|(_, n)| n.role() == AccessRole::ListItem && n.row_index() == Some(3))
        .map(|(id, _)| *id)
        .unwrap();
    let out = s.send(Input::Act(ActionRequest {
        action: Action::ScrollIntoView,
        target_tree: TreeId::ROOT,
        target_node: row3,
        data: None,
    }));
    assert!(out.handled_by.is_some());
    s.settle().unwrap();
    s.settle().unwrap();
    assert!(s.locate("row 3").unwrap().1 >= 0.0, "row 3 is in view again");
    assert_eq!(log.views(3), 1, "and still the same row");
    assert_eq!(s.read(crate::focus::focused), Some(focus.id()));
}

/// The active row ([`ListState::active_from`], read by the window's source)
/// stays built while it is scrolled away, and goes when it is no longer
/// active.
#[test]
fn the_active_row_stays_built_while_scrolled_away() {
    let state = ListState::new(24.0);
    let active = Signal::new(Some(800_u64));
    let log = Rc::new(Log::default());
    let l = Rc::clone(&log);
    let ui = move || {
        let l = Rc::clone(&l);
        component((), move |_| {
            state.active_from(move || active.get());
            move || {
                let l = Rc::clone(&l);
                div()
                    .width(300.0)
                    .child(virtual_each(&state, || MILLIONS, |i| i, move |r| row_view(r, &l)).height(400.0))
                    .into_element()
            }
        })
        .into_element()
    };
    let mut s = scenario(ui);
    assert_eq!(state.pinned(), [800], "built while away");
    assert!(s.locate("row 800").unwrap().1 < 0.0);
    s.act("to it", || state.scroll_to_row(790));
    s.settle().unwrap();
    assert!(state.pinned().is_empty(), "in the window now");
    assert_eq!(log.views(800), 1, "the same row");
    s.act("away again, and none active", || {
        state.scroll_to_row(0);
        active.set(None);
    });
    s.settle().unwrap();
    assert!(state.pinned().is_empty());
    assert!(s.locate("row 800").is_err());
    assert_eq!(log.cleanups(800), 1);
}

/// Rows inserted above the reader's row: the anchor follows its key, so the
/// rows in view stay where they were, the same rows, written their new
/// rows of the data.
#[test]
fn rows_inserted_above_keep_the_readers_rows_in_place() {
    let state = ListState::new(30.0);
    let log = Rc::new(Log::default());
    // Keys are ids; the data inserts `shift` ids before the first.
    let shift = Signal::new(0_u64);
    let count = Signal::new(MILLIONS);
    let key = move |i: u64| i.wrapping_sub(shift.get()).wrapping_add(1_000);
    let mut s = scenario(page(state, count, key, &log, Signal::new(true)));
    s.act("to 2 000", || state.scroll_to_row(2_000));
    s.settle().unwrap();
    let top_key = state.anchor().key.unwrap();
    let before: Vec<f32> = shown_rows(&s).into_iter().map(|(_, y)| y).collect();
    let (stats, views) = (structure_stats(), log.total_views());
    s.act("insert 5 rows at the start", || {
        state.splice(0, 0, 5);
        shift.set(5);
        count.set(MILLIONS + 5);
    });
    s.settle().unwrap();
    assert_eq!(state.anchor().key, Some(top_key), "the anchor kept its key");
    assert_eq!(state.anchor().row, 2_005);
    let after = shown_rows(&s);
    let ys: Vec<f32> = after.iter().map(|&(_, y)| y).collect();
    assert_eq!(ys, before, "the rows in view did not move");
    let stats = since(stats);
    assert_eq!((stats.mounts, stats.unmounts), (0, 0), "the same rows: {stats:?}");
    assert_eq!(log.total_views(), views);
    assert!(
        after.iter().all(|&(r, _)| (1_980..2_100).contains(&r)),
        "their new rows: {after:?}"
    );
}

/// Focus moved to a row and a scroll far away asked in the same turn: the
/// row is found by its own key under its row instance's root as the window
/// is worked out, so it is kept built and keeps focus.
#[test]
fn a_row_focused_then_scrolled_away_at_once_keeps_focus() {
    let state = ListState::new(30.0);
    let handles: Rc<RefCell<HashMap<ElementKey, crate::FocusHandle>>> = Rc::default();
    let h = Rc::clone(&handles);
    let ui = move || {
        let h = Rc::clone(&h);
        div()
            .width(300.0)
            .child(
                virtual_each(
                    &state,
                    || MILLIONS,
                    |i| i,
                    move |r| {
                        let focus = crate::FocusHandle::new();
                        h.borrow_mut().insert(r.key(), focus);
                        div()
                            .height(24.0)
                            .track_focus(focus)
                            .child(text_bound(move || format!("row {}", r.index())))
                    },
                )
                .height(400.0),
            )
            .into_element()
    };
    let mut s = scenario(ui);
    let focus = handles.borrow().get(&3).copied().unwrap();
    s.act("focus row 3 and jump away", || {
        focus.focus();
        state.scroll_to_row(5_000);
    });
    s.settle().unwrap();
    assert!(state.anchor().row >= 5_000, "the window moved");
    assert_eq!(s.read(crate::focus::focused), Some(focus.id()), "row 3 keeps focus");
    assert_eq!(state.focused_row(), Some(3), "kept built, pinned");
}
