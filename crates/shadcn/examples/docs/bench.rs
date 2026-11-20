//! `docs --bench`: how fast each page draws and whether it gives back what
//! it takes, judged against a frame budget.
//!
//! Each page runs in a measuring scenario at the shots' size and scale:
//! - **script**: every frame its script draws, cold caches included;
//! - **steady**: [`ROUNDS`] rounds of [`Options::frames`] more frames of the
//!   page at rest (before, between and after the walks below), what a
//!   frame costs while something on it moves (every frame builds, lays out
//!   and paints the whole page);
//! - **walk**: the pointer rests on each pointer target in view (up to
//!   [`TARGETS`]) while motion plays, then leaves and the page comes to rest.
//!   The first walk warms what loads once (a fallback font a tooltip's text
//!   needs) and is reported; a second walk shows the page again what it has
//!   already shown, so after it the caches and heap must hold what they held
//!   after the first, within [`GROWTH`]. A page a walk leaves showing other
//!   nodes than the one before it (a hover that changed its state) is not
//!   judged for growth.
//!
//! A page FAILs when its steady 95th percentile is past the budget or the walk
//! leaves more held than it found. `--json=PATH` writes a line per page for
//! tools; `--baseline=PATH` compares with such a file from an earlier run and
//! names each page whose best round's median moved past the noise ([`NOISE`]),
//! a slower one counting as a FAIL: the before and after of a change,
//! headless. The best round, not all frames, because a whole round can land
//! on a slower core (Apple silicon's efficiency cores) and double its times. Run it on a release build: a debug build's
//! times say nothing about the app's.

use std::path::PathBuf;
use std::time::Duration;

use gui::heap::Heap;
use gui::inspect::{Footprint, FrameReport, Spread, Timings, bytes, ms};
use gui::{Bounds, Scenario, ScenarioError};

use crate::verdict::field;
use crate::{Page, selected};

/// The most pointer targets a page's walk rests on; the rest are counted, not walked.
const TARGETS: usize = 24;
/// Frames drawn, 50 ms apart, while the pointer rests on one target: past
/// tw-animate's 150 ms, so what a hover starts plays to its end.
const WALK_FRAMES: u32 = 4;
/// What a second walk may leave held past what the first left.
const GROWTH: u64 = 256 * 1024;
/// Rounds of steady frames; the fastest round's median is what runs compare.
const ROUNDS: usize = 3;
/// More times a page that reads slower than the baseline is measured.
const CONFIRM: usize = 2;
/// How far a best round's median may move from the baseline's before it counts:
/// this share of it and this much time, both. Unchanged code moved up to 35%
/// between runs on an Apple silicon Mac mini (a 3.4ms page), so only a change past both is named.
const NOISE: (f64, Duration) = (0.40, Duration::from_millis(1));

/// What `--bench` judges by and where it writes.
#[derive(Clone, Debug)]
pub struct Options {
    /// The steady 95th percentile a page must stay within.
    pub budget: Duration,
    /// Frames drawn at rest after the script.
    pub frames: u32,
    /// Where to write a JSON line per page.
    pub json: Option<PathBuf>,
    /// An earlier run's JSON to compare with.
    pub baseline: Option<PathBuf>,
}

impl Options {
    /// The options in `args` (`--budget=MS`, `--frames=N`, `--json=PATH`,
    /// `--baseline=PATH`) and the arguments left.
    pub fn parse(args: &[String]) -> Result<(Self, Vec<String>), String> {
        let mut options = Self {
            budget: Duration::from_millis(16),
            frames: 30,
            json: None,
            baseline: None,
        };
        let mut rest = Vec::new();
        for a in args {
            if let Some(v) = a.strip_prefix("--budget=") {
                let budget: f64 = v
                    .parse()
                    .map_err(|_| format!("--budget takes milliseconds, not {v:?}"))?;
                options.budget = Duration::try_from_secs_f64(budget / 1000.0).map_err(|e| format!("--budget: {e}"))?;
            } else if let Some(v) = a.strip_prefix("--json=") {
                options.json = Some(PathBuf::from(v));
            } else if let Some(v) = a.strip_prefix("--baseline=") {
                options.baseline = Some(PathBuf::from(v));
            } else if let Some(v) = a.strip_prefix("--frames=") {
                options.frames = v.parse().map_err(|_| format!("--frames takes a count, not {v:?}"))?;
            } else {
                rest.push(a.clone());
            }
        }
        Ok((options, rest))
    }
}

/// One page measured.
struct Bench {
    page: &'static str,
    script: Spread,
    script_frames: usize,
    steady: Spread,
    /// The fastest round's median frame.
    best: Duration,
    /// The steady frames' median per stage.
    stages: Timings,
    /// The steady frames' median allocations and bytes allocated.
    allocations: Option<Heap>,
    rest: Footprint,
    walked: usize,
    targets: usize,
    /// What the first walk left held past the page at rest before it: heap.
    warmed: Option<i128>,
    /// What the second walk left held past the first: images, heap; `None`
    /// when the walks left the page showing different nodes.
    growth: Option<(i128, Option<i128>)>,
}

impl Bench {
    fn over_budget(&self, options: &Options) -> bool {
        self.steady.p95 > options.budget
    }

    fn grew(&self) -> bool {
        let past = |n: i128| n > i128::from(GROWTH);
        self.growth
            .is_some_and(|(images, heap)| past(images) || heap.is_some_and(past))
    }

    fn grade(&self, options: &Options) -> &'static str {
        if self.over_budget(options) || self.grew() {
            "FAIL"
        } else {
            "PASS"
        }
    }

    /// One JSON object on one line, for tools and `--baseline`.
    fn json(&self, options: &Options) -> String {
        let millis = |d: Duration| format!("{:.3}", d.as_secs_f64() * 1000.0);
        let mut line = format!(
            r#"{{"page":"{}","grade":"{}","steady_p50_ms":{},"steady_p95_ms":{},"best_p50_ms":{},"build_ms":{},"layout_ms":{},"paint_ms":{},"script_p95_ms":{},"nodes":{},"image_bytes":{}"#,
            self.page,
            self.grade(options),
            millis(self.steady.p50),
            millis(self.steady.p95),
            millis(self.best),
            millis(self.stages.build),
            millis(self.stages.layout),
            millis(self.stages.paint),
            millis(self.script.p95),
            self.rest.nodes,
            self.rest.raster_bytes(),
        );
        if let (Some(a), Some(h)) = (self.allocations, self.rest.heap) {
            line.push_str(&format!(
                r#","allocations":{},"allocated_bytes":{},"heap_bytes":{}"#,
                a.allocations, a.allocated, h.live
            ));
        }
        if let Some((images, heap)) = self.growth {
            line.push_str(&format!(r#","growth_image_bytes":{images}"#));
            if let Some(heap) = heap {
                line.push_str(&format!(r#","growth_heap_bytes":{heap}"#));
            }
        }
        line.push('}');
        line
    }

    fn line(&self, options: &Options) -> String {
        let grade = self.grade(options);
        let allocations = self.allocations.map_or_else(
            || "heap not counted".to_owned(),
            |h| format!("{} allocations {}/frame", h.allocations, bytes(h.allocated)),
        );
        let heap = self
            .rest
            .heap
            .map_or_else(String::new, |h| format!(", heap {}", bytes(h.live)));
        let again = self.growth.map_or_else(
            || "not judged, the walks left different nodes".to_owned(),
            |(images, heap)| {
                let heap = heap.map_or_else(String::new, |g| format!(", heap {}", signed(g)));
                format!("images {}{heap}", signed(images))
            },
        );
        let warmed = self
            .warmed
            .map_or_else(String::new, |g| format!(" (the first warmed heap {})", signed(g)));
        format!(
            "{grade:<5}{:<22} steady p50 {} p95 {} best round {} (build {} layout {} paint {})  script p95 {} over {}  {allocations}  holds {} nodes, images {}{heap}  walk {}/{}{warmed}, again: {again}",
            self.page,
            ms(self.steady.p50),
            ms(self.steady.p95),
            ms(self.best),
            ms(self.stages.build),
            ms(self.stages.layout),
            ms(self.stages.paint),
            ms(self.script.p95),
            self.script_frames,
            self.rest.nodes,
            bytes(u64::try_from(self.rest.raster_bytes()).unwrap_or(u64::MAX)),
            self.walked,
            self.targets,
        )
    }
}

fn signed(n: i128) -> String {
    let size = bytes(u64::try_from(n.unsigned_abs()).unwrap_or(u64::MAX));
    if n < 0 { format!("-{size}") } else { format!("+{size}") }
}

/// The median of `f` over `reports`.
fn median(reports: &[FrameReport], f: impl Fn(&FrameReport) -> Duration) -> Duration {
    Spread::of(reports.iter().map(f)).p50
}

/// Measure every page in `only` (all when empty) and print a line per page
/// and the summary; `Ok(false)` when any FAILs.
pub fn bench(only: &[String], options: &Options) -> Result<bool, ScenarioError> {
    if cfg!(debug_assertions) {
        eprintln!("bench: a debug build; its times say nothing about a release build's");
    }
    let before = match &options.baseline {
        Some(path) => baseline(path)?,
        None => Vec::new(),
    };
    let mut benches = Vec::new();
    for p in selected(only) {
        let mut b = page(p, options)?;
        // A page that reads slower is measured again: a whole run can land on a slower core.
        let was = before.iter().find(|(page, _)| page == p.name).map(|&(_, d)| d);
        for _ in 0..CONFIRM {
            if was.is_none_or(|was| moved(was, b.best) != Some(true)) {
                break;
            }
            let again = page(p, options)?;
            if again.best < b.best {
                b = again;
            }
        }
        println!("{}", b.line(options));
        benches.push(b);
    }
    if let Some(path) = &options.json {
        let lines: Vec<String> = benches.iter().map(|b| b.json(options)).collect();
        std::fs::write(path, format!("[\n{}\n]\n", lines.join(",\n")))?;
    }
    let slower = match &options.baseline {
        Some(path) => compare(&benches, &before, path),
        None => 0,
    };
    let failed = benches.iter().filter(|b| b.over_budget(options) || b.grew()).count();
    let p95 = Spread::of(benches.iter().map(|b| b.steady.p95));
    let worst = benches.iter().max_by_key(|b| b.steady.p95).map_or("-", |b| b.page);
    println!(
        "\nbench over {} pages: steady p95 median {}, worst {} ({worst}); {failed} FAIL (budget {} p95, growth past {} after a second walk); walks rest on at most {TARGETS} targets in view each",
        benches.len(),
        ms(p95.p50),
        ms(p95.max),
        ms(options.budget),
        bytes(GROWTH),
    );
    Ok(failed == 0 && slower == 0)
}

/// The best-round medians a `--json` file at `path` holds, by page.
fn baseline(path: &std::path::Path) -> Result<Vec<(String, Duration)>, ScenarioError> {
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .filter_map(|l| {
            let p50: f64 = field(l, "best_p50_ms")?.parse().ok()?;
            Some((
                field(l, "page")?.to_owned(),
                Duration::try_from_secs_f64(p50 / 1000.0).ok()?,
            ))
        })
        .collect())
}

/// Whether `now` is slower (`Some(true)`) or faster than `was` past [`NOISE`].
fn moved(was: Duration, now: Duration) -> Option<bool> {
    let by = now.abs_diff(was);
    let share = by.as_secs_f64() / was.as_secs_f64().max(1e-9);
    (share > NOISE.0 && by > NOISE.1).then_some(now > was)
}

/// Print each page whose best round moved past [`NOISE`] from `before` (read
/// from `path`), and the count; returns how many got slower.
fn compare(benches: &[Bench], before: &[(String, Duration)], path: &std::path::Path) -> usize {
    let (mut slower, mut faster, mut compared) = (0_usize, 0_usize, 0_usize);
    println!("\nagainst {}:", path.display());
    for b in benches {
        let Some(&(_, was)) = before.iter().find(|(page, _)| page == b.page) else {
            continue;
        };
        compared = compared.saturating_add(1);
        let word = match moved(was, b.best) {
            None => continue,
            Some(true) => {
                slower = slower.saturating_add(1);
                "SLOWER"
            }
            Some(false) => {
                faster = faster.saturating_add(1);
                "FASTER"
            }
        };
        let share = b.best.abs_diff(was).as_secs_f64() / was.as_secs_f64().max(1e-9);
        println!(
            "{word:<7}{:<22} best round p50 {} -> {} ({}{:.0}%)",
            b.page,
            ms(was),
            ms(b.best),
            if b.best > was { "+" } else { "-" },
            share * 100.0
        );
    }
    println!(
        "{compared} pages compared: {slower} SLOWER (each measured up to {CONFIRM} more times, the fastest kept), {faster} FASTER, the rest within {:.0}% or {} of the baseline",
        NOISE.0 * 100.0,
        ms(NOISE.1)
    );
    slower
}

fn page(p: &'static Page, options: &Options) -> Result<Bench, ScenarioError> {
    let mut s = Scenario::measuring((p.render)(), 960, 640, 2.0)?;
    (p.script)(&mut s)?;
    let script = s.take_reports();

    s.rest(Duration::from_secs(2))?;
    s.take_reports();
    let mut rounds = vec![steady_round(&mut s, options.frames)?];
    let rest = s.footprint();

    let viewport = s.last().viewport;
    let all = s.last().pointer_targets();
    let targets: Vec<Bounds> = all
        .iter()
        .filter(|b| viewport.contains(b.x + b.width / 2.0, b.y + b.height / 2.0))
        .take(TARGETS)
        .copied()
        .collect();
    walk(&mut s, &targets)?;
    let warm = s.footprint();
    rounds.push(steady_round(&mut s, options.frames)?);
    walk(&mut s, &targets)?;
    let after = s.footprint();
    rounds.push(steady_round(&mut s, options.frames)?);
    debug_assert_eq!(rounds.len(), ROUNDS);
    let best = rounds
        .iter()
        .map(|r| median(r, |f| f.timings.total()))
        .min()
        .unwrap_or_default();
    let steady = rounds.concat();

    let heaps: Vec<Heap> = steady.iter().filter_map(|r| r.footprint.heap).collect();
    let allocations = (!heaps.is_empty()).then(|| Heap {
        allocations: Spread::of(heaps.iter().map(|h| h.allocations)).p50,
        allocated: Spread::of(heaps.iter().map(|h| h.allocated)).p50,
        live: 0,
    });
    let raster = |f: &Footprint| i128::try_from(f.raster_bytes()).unwrap_or(i128::MAX);
    let heap_growth = |after: &Footprint, before: &Footprint| {
        after
            .heap
            .zip(before.heap)
            .map(|(a, b)| i128::from(a.live).saturating_sub(i128::from(b.live)))
    };
    Ok(Bench {
        page: p.name,
        script: Spread::of(script.iter().map(|r| r.timings.total())),
        script_frames: script.len(),
        steady: Spread::of(steady.iter().map(|r| r.timings.total())),
        best,
        stages: Timings {
            build: median(&steady, |r| r.timings.build),
            layout: median(&steady, |r| r.timings.layout),
            paint: median(&steady, |r| r.timings.paint),
            vsync: median(&steady, |r| r.timings.vsync),
        },
        allocations,
        rest,
        walked: targets.len(),
        targets: all.len(),
        warmed: heap_growth(&warm, &rest),
        growth: (after.nodes == warm.nodes)
            .then(|| (raster(&after).saturating_sub(raster(&warm)), heap_growth(&after, &warm))),
    })
}

/// `frames` frames of the page as it stands, their reports.
fn steady_round(s: &mut Scenario<crate::Render>, frames: u32) -> Result<Vec<FrameReport>, ScenarioError> {
    for _ in 0..frames {
        s.settle()?;
    }
    Ok(s.take_reports())
}

/// Rest the pointer on each of `targets` while motion plays, then leave and
/// bring the page back to rest; the reports drawn on the way are dropped.
fn walk(s: &mut Scenario<crate::Render>, targets: &[Bounds]) -> Result<(), ScenarioError> {
    s.play_motion();
    for b in targets {
        s.hover(b.x + b.width / 2.0, b.y + b.height / 2.0);
        s.settle()?;
        for _ in 0..WALK_FRAMES {
            s.wait(Duration::from_millis(50));
            s.settle()?;
        }
        s.take_reports();
    }
    s.leave();
    s.still_motion();
    s.rest(Duration::from_secs(2))?;
    // The caches keep what the last two frames drew: two more frames of the
    // page at rest leave only what it shows.
    s.settle()?;
    s.settle()?;
    s.take_reports();
    Ok(())
}
