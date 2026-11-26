//! The windowed list (ADR-0026, K4c) against the list whose window is
//! chosen at render, like for like: the stress scene `list`'s rows (one
//! in seven two lines tall) and its script (wheel 5000, the middle, wheel
//! 3000 and -6000, the end, the start, then 30 wheels of 100), at ten
//! thousand and ten million rows. Each step's median time over the runs
//! and its most allocations on the UI thread are printed (run it with
//! `--release --nocapture` for numbers); the windowed list's steps stay
//! within the scene's allocation budgets, a frame at rest allocates no
//! more than the other list's, and no scroll moves a row.

use std::time::{Duration, Instant};

use gui::app::Scenario;
use gui::component::{StructureStats, structure_stats};
use gui::list::{ListState, virtual_each, virtual_list};
use gui::prelude::*;
use gui::{Color, Element, div, text};

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// The scene `list`'s budgets: one frame of a script step, and a frame at rest.
const ACTION_BUDGET: u64 = 7_500;
const STEADY_BUDGET: u64 = 85;

const BORDER: Color = Color::BLACK;

/// The scene's row `i`.
fn row(i: u64) -> Element {
    let d = div()
        .padding_x(16.0)
        .padding_y(8.0)
        .border_bottom(1.0)
        .border_color(BORDER)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .child(text(format!("Row {i}")).font_weight(500));
    let d = if i.is_multiple_of(7) {
        d.child(
            text(format!("{i} is a multiple of seven, so this row has a second line"))
                .text_size(14.0)
                .line_height(20.0),
        )
    } else {
        d
    };
    d.into_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    /// `virtual_list`: the window chosen as its owner renders.
    Render,
    /// `virtual_each`: the window a keyed `each`'s source.
    Each,
}

fn page(path: Path, state: ListState, count: u64) -> impl Fn() -> Element + 'static {
    move || {
        let list = match path {
            Path::Render => virtual_list(&state, count, |i| i, row),
            Path::Each => virtual_each(&state, move || count, |i| i, |r| row(r.index_untracked())),
        };
        div()
            .size(pct(100.0))
            .padding(24.0)
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(
                list.flex(Flex::grow(1.0))
                    .min_height(0.0)
                    .border(1.0)
                    .border_color(BORDER),
            )
            .into_element()
    }
}

/// A script step: its name, how many frames it draws, what it does.
type Act = fn(&mut Scenario, ListState);

const SCRIPT: &[(&str, u64, Act)] = &[
    ("wheel 5000", 1, |s, _| {
        s.wheel(5_000.0);
    }),
    ("middle", 1, |s, st| s.act("middle", || st.scroll_to_fraction(0.5))),
    ("wheel 3000", 1, |s, _| {
        s.wheel(3_000.0);
    }),
    ("wheel -6000", 1, |s, _| {
        s.wheel(-6_000.0);
    }),
    ("end", 1, |s, st| s.act("end", || st.scroll_to_fraction(1.0))),
    ("start", 1, |s, st| s.act("start", || st.scroll_to_fraction(0.0))),
    ("wheel 100 x30", 30, |s, _| {
        for _ in 0..30 {
            s.wheel(100.0);
            s.settle().unwrap_or_default();
        }
    }),
];

/// One step's cost: its median time, its most allocations per frame, and
/// what its structure commits did.
#[derive(Debug)]
struct Step {
    name: &'static str,
    time: Duration,
    allocations: u64,
    structure: StructureStats,
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

#[expect(clippy::unwrap_used, reason = "a scenario that cannot run is a failed test")]
fn run(path: Path, count: u64, runs: usize) -> (Vec<Step>, u64) {
    let state = ListState::new(36.0);
    let mut s = Scenario::measuring(page(path, state, count), 1024, 768, 1.0).unwrap();
    s.settle().unwrap();
    s.hover(500.0, 400.0);
    // Warm: the glyphs, the layout pool.
    for _ in 0..3 {
        s.wheel(400.0);
        s.settle().unwrap();
    }
    // Per step: its times, its most allocations a frame, its last structure counts.
    let mut seen: Vec<(Vec<Duration>, u64, StructureStats)> =
        vec![(Vec::new(), 0, StructureStats::default()); SCRIPT.len()];
    for _ in 0..runs {
        s.act("back", || state.scroll_to_fraction(0.0));
        s.settle().unwrap();
        for (&(_, frames, act), (times, allocations, structure)) in SCRIPT.iter().zip(seen.iter_mut()) {
            let before = structure_stats();
            let t = Instant::now();
            let ((), heap) = gui::heap::measure(|| {
                act(&mut s, state);
                s.settle().unwrap();
            });
            times.push(t.elapsed());
            let per_frame = heap
                .map_or(u64::MAX, |h| h.allocations)
                .checked_div(frames)
                .unwrap_or(u64::MAX);
            *allocations = (*allocations).max(per_frame);
            *structure = since(before);
        }
    }
    let steps = SCRIPT
        .iter()
        .zip(seen)
        .map(|(&(name, _, _), (mut t, allocations, structure))| {
            t.sort_unstable();
            Step {
                name,
                time: t.get(t.len() / 2).copied().unwrap_or_default(),
                allocations,
                structure,
            }
        })
        .collect();
    // At rest: a frame with nothing to do.
    let mut steady = 0;
    for _ in 0..5 {
        let ((), heap) = gui::heap::measure(|| {
            s.frame("steady").unwrap();
        });
        steady = steady.max(heap.map_or(u64::MAX, |h| h.allocations));
    }
    (steps, steady)
}

#[test]
fn the_windowed_list_against_the_list_built_at_render() {
    let runs = if cfg!(debug_assertions) { 1 } else { 15 };
    for count in [10_000, 10_000_000] {
        let (render, render_steady) = run(Path::Render, count, runs);
        let (each, each_steady) = run(Path::Each, count, runs);
        eprintln!(
            "{count} rows, median of {runs}: step | render ms, allocations/frame | each ms, allocations/frame, mounts, unmounts, moves"
        );
        for (r, e) in render.iter().zip(&each) {
            eprintln!(
                "  {:<14} | {:>7.2} {:>6} | {:>7.2} {:>6} {:>4} {:>4} {:>2}",
                r.name,
                r.time.as_secs_f64() * 1e3,
                r.allocations,
                e.time.as_secs_f64() * 1e3,
                e.allocations,
                e.structure.mounts,
                e.structure.unmounts,
                e.structure.moves,
            );
        }
        eprintln!("  at rest, allocations | render {render_steady} | each {each_steady}");
        for e in &each {
            assert_eq!(e.structure.moves, 0, "{count} rows, {}: a scroll moves no row", e.name);
        }
        // The cold check allocates: budgets are judged without it, as the
        // stress run's `--cold` does.
        if gui::app::cold_check_asked() {
            continue;
        }
        for e in &each {
            assert!(
                e.allocations <= ACTION_BUDGET,
                "{count} rows, {}: {} allocations a frame",
                e.name,
                e.allocations
            );
        }
        assert!(
            each_steady <= STEADY_BUDGET,
            "{count} rows at rest: {each_steady} allocations"
        );
        assert!(each_steady <= render_steady.max(STEADY_BUDGET));
    }
}

/// The whole-source duplicate-key check (ADR-0026, D033) is a debug
/// build's alone: a release build reads the keys the window needs, never
/// every row's.
#[test]
fn the_whole_source_key_check_is_a_debug_builds_alone() {
    use std::cell::Cell;
    use std::rc::Rc;
    let calls = Rc::new(Cell::new(0_u64));
    let c = Rc::clone(&calls);
    let state = ListState::new(30.0);
    let page = move || {
        let c = Rc::clone(&c);
        virtual_each(
            &state,
            || 10_000,
            move |i| {
                c.set(c.get().saturating_add(1));
                i
            },
            |r| row(r.index_untracked()),
        )
        .size(pct(100.0))
        .into_element()
    };
    let mut s = Scenario::measuring(page, 1024, 768, 1.0).unwrap();
    s.settle().unwrap();
    assert_eq!(
        calls.get() >= 10_000,
        cfg!(debug_assertions),
        "keys read at mount: {}",
        calls.get()
    );
}
