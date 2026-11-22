//! Benchmark 5.2 of the research (`docs/research-2026-11-21.md`), on the
//! retained layout tree (ADR-0014): a table as stress `table` builds it,
//! a scroll container, the table, its body and 5 000 rows of five cells
//! (a checkbox, three texts, a badge), about 65 000 nodes, rows as grids of
//! `fr` columns, text measured at 7 px a character. `TaffyTree` built and
//! laid out each frame (what `gui` did before) runs alongside.
//!
//! `cargo run --release -p gui --example layout_bench [rows]`
//!
//! `[rows] cold|cold1|grow N` repeats one case `N` times instead, to profile
//! it (`cold1` on the calling thread alone).
//!
//! Cold layouts also report retired instructions ([`gui::instructions`],
//! the whole process on macOS) and the process's CPU time (user and system,
//! every thread): under a loaded machine time moves with the load, these
//! two much less.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    reason = "a benchmark: counts and text widths are small, and a cast to a float is what it measures with"
)]

use std::time::{Duration, Instant};

use gui::layout::engine::{Change, Measure, MeasureReads, Measured, Node, Schedule, Stats};
use gui::layout::{BoxStyle, Len};
use gui::style::Track;
use taffy::prelude::*;
use taffy::{Overflow, Point};

/// Text 7 px a character, 20 px a line, wrapped at spaces. As `gui`'s text,
/// it reads the width it is given alone, known or available alike
/// ([`MeasureReads::TEXT`]).
#[derive(Clone, Debug, PartialEq)]
struct Text(String);

impl Text {
    fn size(&self, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
        let words: Vec<usize> = self.0.split(' ').map(str::len).collect();
        let longest = words.iter().copied().max().unwrap_or(0) as f32 * 7.0;
        let all = self.0.len() as f32 * 7.0;
        let width = match known.width.map_or(available.width, AvailableSpace::Definite) {
            AvailableSpace::MinContent => longest,
            AvailableSpace::MaxContent => all,
            AvailableSpace::Definite(w) => w.min(all).max(longest),
        };
        let mut lines = 1_u32;
        let mut line = 0.0_f32;
        for w in words {
            let w = w as f32 * 7.0;
            if line > 0.0 && line + 7.0 + w > width + 0.01 {
                lines += 1;
                line = w;
            } else {
                line += if line > 0.0 { 7.0 + w } else { w };
            }
        }
        Size {
            width,
            height: lines as f32 * 20.0,
        }
    }
}

impl Measure for Text {
    type Cx = ();
    fn reads(&self) -> MeasureReads {
        MeasureReads::TEXT
    }
    fn measure(&self, (): &mut (), known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        self.size(known, available).into()
    }
}

type Ours = Node<BoxStyle, Text>;

fn boxed(f: impl FnOnce(&mut BoxStyle)) -> BoxStyle {
    let mut s = BoxStyle::default();
    f(&mut s);
    s
}

fn text(s: &str) -> Ours {
    Node::new(BoxStyle::default(), Some(Text(s.to_owned())))
}

fn with(style: BoxStyle, children: Vec<Ours>) -> Ours {
    let mut n = Node::new(style, None);
    n.children = children;
    n
}

fn cell(child: Ours) -> Ours {
    with(
        boxed(|s| {
            s.display = Display::Flex;
            s.padding = Rect {
                left: 8.0,
                right: 8.0,
                top: 8.0,
                bottom: 8.0,
            };
            s.align_items = AlignItems::CENTER;
        }),
        vec![child],
    )
}

fn row(i: usize) -> Ours {
    let checkbox = with(
        boxed(|s| {
            s.size = Size {
                width: Len::Px(16.0),
                height: Len::Px(16.0),
            };
            s.border = Rect {
                left: 1.0,
                right: 1.0,
                top: 1.0,
                bottom: 1.0,
            };
        }),
        Vec::new(),
    );
    let badge = with(
        boxed(|s| {
            s.display = Display::Flex;
            s.padding = Rect {
                left: 8.0,
                right: 8.0,
                top: 2.0,
                bottom: 2.0,
            };
        }),
        vec![text(if i.is_multiple_of(3) { "active" } else { "paused" })],
    );
    with(
        boxed(|s| {
            s.display = Display::Grid;
            s.grid_template_columns = vec![
                Track::Px(40.0),
                Track::Fr(1.0),
                Track::Fr(2.0),
                Track::Fr(1.0),
                Track::Fr(1.0),
            ];
            s.border.bottom = 1.0;
        }),
        vec![
            cell(checkbox),
            cell(text(&format!("user{i}@example.com"))),
            cell(text(&format!("Customer number {i}"))),
            cell(text(&format!("${}.00", (i * 37) % 1000))),
            cell(badge),
        ],
    )
}

fn table(rows: usize) -> Ours {
    let body = with(boxed(|s| s.display = Display::Flex), Vec::new());
    let mut body = body;
    body.style.flex_direction = FlexDirection::Column;
    body.children = (0..rows).map(row).collect();
    let table = with(boxed(|s| s.display = Display::Flex), vec![body]);
    let mut table = table;
    table.style.flex_direction = FlexDirection::Column;
    // `w-full` in a 1 200 px window: wrapping text makes rows taller, not the table wider.
    table.style.size.width = Len::Px(1200.0);
    let mut root = with(
        boxed(|s| {
            s.display = Display::Flex;
            s.flex_direction = FlexDirection::Column;
            s.overflow = Point {
                x: Overflow::Scroll,
                y: Overflow::Scroll,
            };
        }),
        vec![table],
    );
    root.recount();
    root
}

const WINDOW: Size<AvailableSpace> = Size {
    width: AvailableSpace::Definite(1200.0),
    height: AvailableSpace::Definite(800.0),
};

fn lay_out(root: &mut Ours, schedule: Schedule) -> (Duration, Stats) {
    let t = Instant::now();
    let stats = root.compute_root(WINDOW, &mut (), schedule);
    (t.elapsed(), stats)
}

/// One cold layout: wall time, CPU time of the process, instructions.
#[derive(Clone, Copy)]
struct Cost {
    wall: Duration,
    cpu: Duration,
    instructions: Option<u64>,
}

fn cpu_now() -> Duration {
    cpu_time::ProcessTime::try_now().map_or(Duration::ZERO, |t| t.as_duration())
}

/// Lay a fresh table out cold `n` times; the medians of each measure, and
/// the stats of the last run.
fn cold(n: usize, rows: usize, schedule: Schedule) -> (Cost, Cost, Stats) {
    let mut counter = gui::instructions::Instructions::start().ok();
    let mut costs = Vec::with_capacity(n);
    let mut stats = Stats::default();
    for _ in 0..n {
        let mut t = table(rows);
        let i0 = counter.as_mut().and_then(gui::instructions::Instructions::read);
        let c0 = cpu_now();
        let (wall, s) = lay_out(&mut t, schedule);
        let cpu = cpu_now().saturating_sub(c0);
        let i1 = counter.as_mut().and_then(gui::instructions::Instructions::read);
        stats = s;
        costs.push(Cost {
            wall,
            cpu,
            instructions: i0.zip(i1).map(|(a, b)| b.saturating_sub(a)),
        });
        drop(t);
    }
    let median = |mut v: Vec<Duration>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or_default()
    };
    let min = |v: Vec<Duration>| v.into_iter().min().unwrap_or_default();
    let instructions = || {
        let mut v: Vec<u64> = costs.iter().filter_map(|c| c.instructions).collect();
        v.sort_unstable();
        v.get(v.len() / 2).copied()
    };
    let walls = || costs.iter().map(|c| c.wall).collect::<Vec<_>>();
    let cpus = || costs.iter().map(|c| c.cpu).collect::<Vec<_>>();
    (
        Cost {
            wall: median(walls()),
            cpu: median(cpus()),
            instructions: instructions(),
        },
        Cost {
            wall: min(walls()),
            cpu: min(cpus()),
            instructions: instructions(),
        },
        stats,
    )
}

fn report_cold(what: &str, n: usize, rows: usize, schedule: Schedule) {
    let (median, best, s) = cold(n, rows, schedule);
    report(what, best.wall, s);
    println!(
        "{:<44} median {:>10}, cpu {:>10} (best {}), {} instructions",
        "",
        ms(median.wall),
        ms(median.cpu),
        ms(best.cpu),
        median
            .instructions
            .map_or_else(|| "no".to_owned(), |i| format!("{:.1}M", i as f64 / 1e6))
    );
}

/// Change the leaf at `path` to `content` and settle the tree as its owner does.
fn change(n: &mut Ours, path: &[usize], content: &str) -> Change {
    match path.split_first() {
        None => {
            n.leaf = Some(Text(content.to_owned()));
            n.settle(false, true, Change::None)
        }
        Some((&i, rest)) => {
            let reach = n.children.get_mut(i).map_or(Change::None, |c| change(c, rest, content));
            n.settle(false, false, reach)
        }
    }
}

// Row 2500, cell 2 (the customer), its text.
const CELL: [usize; 5] = [0, 0, 2500, 2, 0];

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1000.0)
}

fn report(what: &str, d: Duration, s: Stats) {
    println!(
        "{what:<44} {:>12}   {:>7} calls / {:>6} misses / {:>6} measures / {:>4} jobs",
        ms(d),
        s.calls,
        s.misses,
        s.measures,
        s.jobs
    );
}

/// The best of `n` runs of `f` on fresh state from `setup`.
fn best<T>(
    n: usize,
    mut setup: impl FnMut() -> T,
    mut f: impl FnMut(&mut T) -> (Duration, Stats),
) -> (Duration, Stats) {
    let mut out = (Duration::MAX, Stats::default());
    for _ in 0..n {
        let mut state = setup();
        let r = f(&mut state);
        if r.0 < out.0 {
            out = r;
        }
    }
    out
}

/// What `TaffyTree` built anew and laid out takes for the same table.
fn taffy_tree(rows: usize) -> Duration {
    let mut tree: TaffyTree<String> = TaffyTree::new();
    tree.disable_rounding();
    let t = Instant::now();
    let pad = |l, r, t, b| Rect {
        left: LengthPercentage::length(l),
        right: LengthPercentage::length(r),
        top: LengthPercentage::length(t),
        bottom: LengthPercentage::length(b),
    };
    let cell_style = Style {
        display: Display::Flex,
        padding: pad(8.0, 8.0, 8.0, 8.0),
        align_items: AlignItems::CENTER,
        ..Style::default()
    };
    let mut rows_ids = Vec::with_capacity(rows);
    for i in 0..rows {
        let leaf = |tree: &mut TaffyTree<String>, s: String| tree.new_leaf_with_context(Style::default(), s);
        let checkbox = tree
            .new_leaf(Style {
                size: Size::from_lengths(16.0, 16.0),
                border: pad(1.0, 1.0, 1.0, 1.0),
                ..Style::default()
            })
            .unwrap_or_else(|_| std::process::abort());
        let badge_text = leaf(&mut tree, (if i % 3 == 0 { "active" } else { "paused" }).to_owned());
        let texts = [
            format!("user{i}@example.com"),
            format!("Customer number {i}"),
            format!("${}.00", (i * 37) % 1000),
        ];
        let mut cells = Vec::new();
        let c = tree.new_with_children(cell_style.clone(), &[checkbox]);
        cells.extend(c);
        for t in texts {
            let l = leaf(&mut tree, t);
            if let Ok(l) = l {
                cells.extend(tree.new_with_children(cell_style.clone(), &[l]));
            }
        }
        if let Ok(b) = badge_text {
            let badge = tree.new_with_children(
                Style {
                    display: Display::Flex,
                    padding: pad(8.0, 8.0, 2.0, 2.0),
                    ..Style::default()
                },
                &[b],
            );
            if let Ok(badge) = badge {
                cells.extend(tree.new_with_children(cell_style.clone(), &[badge]));
            }
        }
        let row = tree.new_with_children(
            Style {
                display: Display::Grid,
                grid_template_columns: vec![
                    length(40.0),
                    minmax(length(0.0), fr(1.0)),
                    minmax(length(0.0), fr(2.0)),
                    minmax(length(0.0), fr(1.0)),
                    minmax(length(0.0), fr(1.0)),
                ],
                border: pad(0.0, 0.0, 0.0, 1.0),
                ..Style::default()
            },
            &cells,
        );
        rows_ids.extend(row);
    }
    let column = Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        ..Style::default()
    };
    let Ok(body) = tree.new_with_children(column.clone(), &rows_ids) else {
        return t.elapsed();
    };
    let Ok(table) = tree.new_with_children(
        Style {
            size: Size {
                width: Dimension::length(1200.0),
                height: Dimension::auto(),
            },
            ..column.clone()
        },
        &[body],
    ) else {
        return t.elapsed();
    };
    let Ok(root) = tree.new_with_children(
        Style {
            overflow: Point {
                x: Overflow::Scroll,
                y: Overflow::Scroll,
            },
            ..column
        },
        &[table],
    ) else {
        return t.elapsed();
    };
    let laid = tree.compute_layout_with_measure(root, WINDOW, |input, _, ctx, style| {
        let ctx = ctx.map(|s| Text(s.clone()));
        taffy::compute_leaf_layout(
            input,
            style,
            |_, _| 0.0,
            |known, available| ctx.as_ref().map_or(Size::ZERO, |t| t.size(known, available)),
        )
    });
    let took = t.elapsed();
    assert!(laid.is_ok(), "TaffyTree could not lay the table out");
    took
}

fn main() {
    let rows: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(5000);
    let threads = gui::layout::engine::pool().current_num_threads();
    let mut probe = table(rows);
    println!(
        "benchmark 5.2: {rows} rows, {} nodes, layout pool of {threads} threads\n",
        probe.count()
    );
    let parallel = Schedule {
        parallel: true,
        reverse: false,
    };
    let serial = Schedule::default();
    if let Some(case) = std::env::args().nth(2) {
        let n: usize = std::env::args().nth(3).and_then(|a| a.parse().ok()).unwrap_or(100);
        profile(&case, rows, n, if case == "cold1" { serial } else { parallel });
        return;
    }

    // Warm the pool and the text measure.
    lay_out(&mut probe, parallel);
    let taffy = (0..5).map(|_| taffy_tree(rows)).min().unwrap_or_default();
    println!("{:<44} {:>12}", "TaffyTree built and laid out (before M4)", ms(taffy));
    let runs: usize = std::env::var("BENCH_COLD_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9);
    report_cold("cold, 1 thread", runs, rows, serial);
    report_cold(&format!("cold, {threads} threads"), runs, rows, parallel);

    let mut t = table(rows);
    lay_out(&mut t, parallel);
    let (d, s) = lay_out(&mut t, parallel);
    report("nothing changed", d, s);

    // One cell's text changes, its size kept.
    let mut flip = false;
    let (d, s) = best(
        20,
        || (),
        |()| {
            flip = !flip;
            change(
                &mut t,
                &CELL,
                if flip {
                    "Customer number 2501"
                } else {
                    "Customer number 2500"
                },
            );
            lay_out(&mut t, parallel)
        },
    );
    report("one cell changed, same size", d, s);

    // One cell wraps: its row 20 px taller, and back.
    let long = "Customer number 2500 with a name long enough to wrap onto another line of the cell";
    let mut grow = false;
    let (d, s) = best(
        20,
        || (),
        |()| {
            grow = !grow;
            change(&mut t, &CELL, if grow { long } else { "Customer number 2500" });
            lay_out(&mut t, parallel)
        },
    );
    report("one row 20 px taller (or back)", d, s);
}

/// Repeat one case `n` times, for a profiler to sample.
fn profile(case: &str, rows: usize, n: usize, schedule: Schedule) {
    let mut total = Duration::ZERO;
    let mut stats = Stats::default();
    match case {
        "cold" | "cold1" => {
            for _ in 0..n {
                let mut t = table(rows);
                let (d, s) = lay_out(&mut t, schedule);
                total += d;
                stats.add(s);
            }
        }
        _ => {
            let long = "Customer number 2500 with a name long enough to wrap onto another line of the cell";
            let mut t = table(rows);
            lay_out(&mut t, schedule);
            for i in 0..n {
                change(&mut t, &CELL, if i % 2 == 0 { long } else { "Customer number 2500" });
                let (d, s) = lay_out(&mut t, schedule);
                total += d;
                stats.add(s);
            }
        }
    }
    report(
        &format!("{case} ×{n}, mean"),
        total / u32::try_from(n.max(1)).unwrap_or(1),
        stats,
    );
}
