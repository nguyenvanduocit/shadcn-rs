//! Stress scenes for `gui` and shadcn, driven by the devtools ([`gui::devtools`]).
//!
//! ```sh
//! cargo run --release -p shadcn --example stress -- --run [scene…] [--cycles=N --monkey=N --budget=MS --frames]
//! cargo run --release -p shadcn --example stress -- --a11y [scene…]   # what a screen reader is sent per frame
//! cargo run --release -p shadcn --example stress -- --inspect <scene> [-e CMD]…   # else stdin
//! cargo run --release -p shadcn --example stress -- --window <scene>              # with HUD and devtools socket
//! cargo run --release -p shadcn --example stress -- --list
//! ```
//!
//! `--run` takes each scene through its script, times its steady frames,
//! repeats its cycle and checks the heap gives back what a cycle takes, then
//! lets the monkey loose on it ([`gui::devtools`]'s `monkey`). A scene
//! `FAIL`s when a command fails, the monkey finds a panic or a failed layout,
//! a cycle keeps memory, or its frames allocate past its budget on the UI
//! thread (ADR-0023: a steady frame's 95th percentile, and the costliest
//! script step's frames together; CI runs `--run --cycles=0 --monkey=0`).
//! Slow frames are reported, not failed: the scenes are built to be slow.
//!
//! `--cold` runs the scripts with every frame checked against one built cold
//! (ADR-0013, `GUI_COLD_CHECK=1`, a build with debug assertions): the check
//! allocates and takes time, so the budgets are printed and not judged, and
//! a scene fails only when a command, a frame or the cold comparison does.

// An app, not a component (ADR-0028, "The boundary for app callbacks"):
// the stress runner reads files and runs threads of its own.
#![expect(
    clippy::disallowed_methods,
    reason = "an app's own I/O and threads; the bans hold component code"
)]

mod scenes;
mod threads;

use std::time::{Duration, Instant};

use gui::devtools::Session;
use gui::heap;
use gui::inspect::{Spread, bytes, ms};
use gui::{Element, Scenario};
use shadcn::{Theme, VEGA, install};

/// The heap counted, for the leak checks and the reports.
#[cfg(not(feature = "dhat-heap"))]
#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// Diagnosis (ADR-0023): `--features dhat-heap` writes `dhat-heap.json` on exit,
/// for <https://nnethercote.github.io/dh_view/dh_view.html>.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static HEAP: dhat::Alloc = dhat::Alloc;

/// A scene's render function: its state is in the signals it made.
pub type Render = Box<dyn Fn() -> Element>;

/// What `--run` judges by.
struct Options {
    cycles: usize,
    monkey: usize,
    budget: Duration,
    /// What one cycle may leave held, on average, past the first.
    growth: u64,
    /// Print every drawing frame of the script (`--frames`).
    frames: bool,
    /// Frames checked against a cold build; allocation budgets not judged (`--cold`).
    cold: bool,
}

fn flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    args.iter().find_map(|a| {
        a.strip_prefix("--")
            .and_then(|f| f.strip_prefix(key))
            .and_then(|v| v.strip_prefix('='))
    })
}

fn number(args: &[String], key: &str, default: usize) -> Result<usize, String> {
    flag(args, key).map_or(Ok(default), |v| v.parse().map_err(|_| format!("--{key} is a number")))
}

fn scene(name: &str) -> Result<&'static scenes::Scene, String> {
    scenes::ALL
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("no scene {name:?}; --list names them"))
}

fn session(scene: &scenes::Scene) -> Result<Session, String> {
    let scenario = Scenario::measuring((scene.render)(), scene.size.0, scene.size.1, 1.0).map_err(|e| e.to_string())?;
    Session::new(scenario).map_err(|e| e.to_string())
}

/// A command run for its effect; the thread's mail (a feed) arrives first.
fn command(s: &mut Session, line: &str) -> Result<String, String> {
    if line == "frame" {
        // What another thread sends lands between frames; give it time to.
        std::thread::sleep(scenes::FEED_SETTLE);
    }
    s.run(line)
}

/// The frames of script section `name` (after frame `from`, up to `to`):
/// their CPU and GPU times and the tiles they rastered.
fn section(s: &Session, name: &str, from: u64, to: u64) -> Option<String> {
    let drew: Vec<_> = s
        .log()
        .reports()
        .filter(|r| r.number > from && r.number <= to && r.drawn.submitted)
        .collect();
    if drew.is_empty() {
        return None;
    }
    let cpu = Spread::of(drew.iter().map(|r| {
        r.timings
            .total()
            .saturating_sub(r.drawn.gpu_wait)
            .saturating_sub(r.drawn.raster_wait)
    }));
    let tiles: usize = drew.iter().map(|r| r.drawn.tiles_done).sum();
    let mut line = format!(
        "  {name} frames ×{}  cpu p50 {}  p95 {}  max {}  tiles rastered {tiles}",
        drew.len(),
        ms(cpu.p50),
        ms(cpu.p95),
        ms(cpu.max)
    );
    let gpu: Vec<Duration> = drew.iter().filter_map(|r| r.drawn.gpu).collect();
    if !gpu.is_empty() {
        let g = Spread::of(gpu.iter().copied());
        line.push_str(&format!(
            "  gpu p50 {}  p95 {}  max {}",
            ms(g.p50),
            ms(g.p95),
            ms(g.max)
        ));
    }
    Some(line)
}

/// What one scene's run found.
struct Verdict {
    lines: Vec<String>,
    failed: bool,
}

fn run_scene(scene: &scenes::Scene, o: &Options) -> Verdict {
    let mut lines = Vec::new();
    let mut failed = false;
    let started = Instant::now();
    let mut s = match session(scene) {
        Ok(s) => s,
        Err(e) => {
            return Verdict {
                lines: vec![format!("  could not start: {e}")],
                failed: true,
            };
        }
    };
    let first = s.log().last().map(|r| (r.timings.total(), r.footprint.nodes));
    if let Some((took, nodes)) = first {
        lines.push(format!("  first frame {}  {nodes} nodes", ms(took)));
    }

    // The script: each action timed by the frame it caused, and what all
    // the frames it drew allocated on this thread.
    let mut slowest = (Duration::ZERO, String::new());
    let mut costliest: (u64, String) = (0, String::new());
    let script_from = s.log().last().map_or(0, |r| r.number);
    // Named parts of the script (`section NAME` … `section end`), whose
    // frames are reported on their own as each ends (the log keeps only
    // the latest frames).
    let mut sections: Vec<String> = Vec::new();
    let mut open: Option<(String, u64)> = None;
    for line in scene.script {
        let before = s.log().last().map_or(0, |r| r.number);
        if let Some(name) = line.strip_prefix("section ") {
            if let Some((n, from)) = open.take()
                && let Some(l) = section(&s, &n, from, before)
            {
                sections.push(l);
            }
            if name != "end" {
                open = Some((name.to_owned(), before));
            }
            continue;
        }
        match command(&mut s, line) {
            Ok(_) => {
                if let Some(r) = s.log().last()
                    && r.timings.total() > slowest.0
                {
                    slowest = (r.timings.total(), (*line).to_owned());
                }
                let allocated = s
                    .log()
                    .reports()
                    .filter(|r| r.number > before)
                    .filter_map(|r| r.footprint.thread)
                    .fold(0_u64, |n, t| n.saturating_add(t.allocations));
                if allocated >= costliest.0 {
                    costliest = (allocated, (*line).to_owned());
                }
            }
            Err(e) => {
                failed = true;
                lines.push(format!("  FAIL script `{line}`: {e}"));
            }
        }
    }
    if !slowest.1.is_empty() {
        lines.push(format!("  slowest action {} for `{}`", ms(slowest.0), slowest.1));
    }
    lines.extend(sections);
    // The script's frames that drew: their CPU time, what they uploaded,
    // and the GPU's time by its timestamps (a frame or two behind).
    {
        let drew: Vec<_> = s
            .log()
            .reports()
            .filter(|r| r.number > script_from && r.drawn.submitted)
            .collect();
        if !drew.is_empty() {
            // The CPU's part: the frame less its wait for the GPU (and, for a
            // canvas, for the raster workers' plans: their own threads' work).
            let cpu_of = |r: &&gui::inspect::FrameReport| {
                r.timings
                    .total()
                    .saturating_sub(r.drawn.gpu_wait)
                    .saturating_sub(r.drawn.raster_wait)
            };
            let cpu = Spread::of(drew.iter().map(cpu_of));
            let up = Spread::of(drew.iter().map(|r| r.drawn.uploaded));
            let mut line = format!(
                "  drawing frames ×{}  cpu p50 {}  p95 {}  max {}  uploaded p50 {}  max {}",
                drew.len(),
                ms(cpu.p50),
                ms(cpu.p95),
                ms(cpu.max),
                bytes(up.p50),
                bytes(up.max)
            );
            let gpu: Vec<Duration> = drew.iter().filter_map(|r| r.drawn.gpu).collect();
            if !gpu.is_empty() {
                let g = Spread::of(gpu.iter().copied());
                let least = gpu.iter().copied().min().unwrap_or_default();
                line.push_str(&format!(
                    "  gpu least {}  p50 {}  p95 {}  max {}",
                    ms(least),
                    ms(g.p50),
                    ms(g.p95),
                    ms(g.max)
                ));
                let passes: Vec<gui::scene::GpuPasses> = drew.iter().filter_map(|r| r.drawn.gpu_passes).collect();
                if !passes.is_empty() {
                    let p50 = |f: fn(&gui::scene::GpuPasses) -> Duration| ms(Spread::of(passes.iter().map(f)).p50);
                    line.push_str(&format!(
                        "  (p50 paths {}  chunk compute {}  coverage {}  passes {})",
                        p50(|p| p.paths),
                        p50(|p| p.compute),
                        p50(|p| p.coverage),
                        p50(|p| p.main)
                    ));
                }
            }
            lines.push(line);
            // Canvas raster (ADR-0017): frames that rasterized tiles, and
            // frames the compositor drew alone to go on with it.
            let raster: Vec<_> = drew.iter().copied().filter(|r| r.drawn.raster > 0).collect();
            if !raster.is_empty() {
                let alone = drew
                    .iter()
                    .filter(|r| r.causes.iter().any(|c| c == "compositor frame"))
                    .count();
                let cost = Spread::of(raster.iter().map(|r| r.drawn.raster));
                let rcpu = Spread::of(raster.iter().map(cpu_of));
                let tiles: usize = raster.iter().map(|r| r.drawn.tiles_done).sum();
                lines.push(format!(
                    "  raster frames ×{} ({alone} drawn by the compositor alone)  {tiles} tiles  cost p50 {}  max {}  cpu p50 {}  p95 {}  max {}",
                    raster.len(),
                    cost.p50,
                    cost.max,
                    ms(rcpu.p50),
                    ms(rcpu.p95),
                    ms(rcpu.max)
                ));
            }
            if o.frames {
                for r in &drew {
                    let t = r.timings;
                    let p = r.drawn.gpu_passes.unwrap_or_default();
                    lines.push(format!(
                        "    #{} cpu {} (build {} layout {} paint {})  gpu {} (paths {} compute {} coverage {} passes {})  damage {} px  up {}",
                        r.number,
                        ms(cpu_of(r)),
                        ms(t.build),
                        ms(t.layout),
                        ms(t.paint.saturating_sub(r.drawn.gpu_wait).saturating_sub(r.drawn.raster_wait)),
                        r.drawn.gpu.map_or_else(|| "-".to_owned(), ms),
                        ms(p.paths),
                        ms(p.compute),
                        ms(p.coverage),
                        ms(p.main),
                        r.drawn.damage,
                        bytes(r.drawn.uploaded),
                    ));
                }
            }
        }
    }
    if !costliest.1.is_empty() {
        let mut line = format!(
            "  costliest action {} allocations for `{}` (budget {})",
            costliest.0, costliest.1, scene.budget.action
        );
        if costliest.0 > scene.budget.action && !o.cold {
            failed = true;
            line.push_str("  FAIL: over its allocation budget");
        }
        lines.push(line);
    }

    // Steady frames: the page as it is, rebuilt each frame.
    match s.bench(30) {
        Ok(frames) => {
            let total = Spread::of(frames.iter().map(|r| r.timings.total()));
            let over = if total.p95 > o.budget { "  (over budget)" } else { "" };
            lines.push(format!(
                "  steady p50 {}  p95 {}  build {}  layout {}  paint {}{over}",
                ms(total.p50),
                ms(total.p95),
                ms(Spread::of(frames.iter().map(|r| r.timings.build)).p50),
                ms(Spread::of(frames.iter().map(|r| r.timings.layout)).p50),
                ms(Spread::of(frames.iter().map(|r| r.timings.paint)).p50),
            ));
            if let Some(r) = frames.last() {
                lines.push(format!("  holds {}", r.footprint));
            }
            let allocs = Spread::of(frames.iter().filter_map(|r| r.footprint.thread).map(|t| t.allocations));
            let mut line = format!(
                "  steady allocations p50 {}  p95 {}  max {} on the UI thread (budget {})",
                allocs.p50, allocs.p95, allocs.max, scene.budget.steady
            );
            if allocs.p95 > scene.budget.steady && !o.cold {
                failed = true;
                line.push_str("  FAIL: over its allocation budget");
            }
            lines.push(line);
        }
        Err(e) => {
            failed = true;
            lines.push(format!("  FAIL bench: {e}"));
        }
    }

    // Cycles: what one round takes, a later round must not keep.
    if !scene.cycle.is_empty() && o.cycles >= 2 {
        let mut after_first = None;
        let mut cycle_times = Vec::new();
        let warm = usize::from(o.cycles >= 3);
        for round in 0..o.cycles {
            let t0 = Instant::now();
            for line in scene.cycle {
                if let Err(e) = command(&mut s, line) {
                    failed = true;
                    lines.push(format!("  FAIL cycle {round} `{line}`: {e}"));
                }
            }
            cycle_times.push(t0.elapsed());
            if let Err(e) = s.rest() {
                failed = true;
                lines.push(format!("  FAIL rest: {e}"));
            }
            // The first rounds warm what loads once (a font a toast needs,
            // a cache's first entries); the baseline is after the second.
            if round == warm {
                s.forget_log();
                after_first = heap::now();
            }
        }
        s.forget_log();
        let after_last = heap::now();
        let spread = Spread::of(cycle_times.iter().copied());
        let mut line = format!("  cycle ×{} p50 {}  max {}", o.cycles, ms(spread.p50), ms(spread.max));
        if let (Some(a), Some(b)) = (after_first, after_last) {
            let rounds = u64::try_from(o.cycles.saturating_sub(warm.saturating_add(1)))
                .unwrap_or(1)
                .max(1);
            let grew = b.live.saturating_sub(a.live);
            let per = grew.checked_div(rounds).unwrap_or(0);
            line.push_str(&format!(
                "  heap {} → {} ({} per cycle)",
                bytes(a.live),
                bytes(b.live),
                bytes(per)
            ));
            if per > o.growth {
                failed = true;
                line.push_str(&format!("  FAIL: keeps more than {} per cycle", bytes(o.growth)));
            }
        }
        lines.push(line);
    }

    // The monkey: random input, checked at every step.
    if o.monkey > 0 {
        match s.run(&format!(
            "monkey {} --seed=7 --budget={}",
            o.monkey,
            o.budget.as_millis()
        )) {
            Ok(report) => {
                for l in report.lines() {
                    lines.push(format!("  {l}"));
                }
            }
            Err(e) => {
                failed = true;
                for l in e.lines() {
                    lines.push(format!("  FAIL {l}"));
                }
            }
        }
    }
    lines.push(format!("  took {:.1}s", started.elapsed().as_secs_f32()));
    Verdict { lines, failed }
}

fn run(args: &[String]) -> Result<bool, String> {
    let names: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let o = Options {
        cycles: number(args, "cycles", 5)?,
        monkey: number(args, "monkey", 150)?,
        budget: Duration::from_millis(u64::try_from(number(args, "budget", 16)?).unwrap_or(16)),
        growth: u64::try_from(number(args, "growth-kb", 64)?)
            .unwrap_or(64)
            .saturating_mul(1024),
        frames: args.iter().any(|a| a == "--frames"),
        cold: args.iter().any(|a| a == "--cold"),
    };
    if o.cold && !cfg!(debug_assertions) {
        return Err(
            "--cold needs a build with debug assertions: the cold check is compiled out without them".to_owned(),
        );
    }
    if o.cold && !gui::app::cold_check_asked() {
        return Err("--cold needs GUI_COLD_CHECK=1".to_owned());
    }
    // `threads` is no scene: ADR-0015's goals, on the app's own threads.
    let named_threads = names.iter().any(|n| *n == "threads");
    // Its frames are the app's own threads', not a `Scenario`'s: no cold check.
    if o.cold && named_threads {
        return Err("`threads` has no cold check; --cold runs the scenes".to_owned());
    }
    let with_threads = !o.cold && (names.is_empty() || named_threads);
    if o.cold && names.is_empty() {
        println!("threads skipped (no cold check)\n");
    }
    let names: Vec<&String> = names.into_iter().filter(|n| *n != "threads").collect();
    let chosen: Vec<&scenes::Scene> = if named_threads && names.is_empty() {
        Vec::new()
    } else if names.is_empty() {
        scenes::ALL.iter().collect()
    } else {
        names.iter().map(|n| scene(n)).collect::<Result<_, _>>()?
    };
    let mut fails = Vec::new();
    if with_threads {
        println!("threads: the UI busy, the render thread scrolls and spins at 120 Hz; input to present");
        let (lines, failed) = threads::run();
        for l in &lines {
            println!("{l}");
        }
        println!("  {}\n", if failed { "FAIL" } else { "PASS" });
        if failed {
            fails.push("threads");
        }
    }
    for sc in chosen {
        println!("{}: {}", sc.name, sc.about);
        let v = run_scene(sc, &o);
        for l in &v.lines {
            println!("{l}");
        }
        println!("  {}\n", if v.failed { "FAIL" } else { "PASS" });
        if v.failed {
            fails.push(sc.name);
        }
    }
    if fails.is_empty() {
        println!("all scenes PASS");
    } else {
        println!("FAIL: {}", fails.join(", "));
    }
    Ok(fails.is_empty())
}

/// Takes the adapter's change events and drops them.
struct Quiet;

impl accesskit_consumer::TreeChangeHandler for Quiet {
    fn node_added(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
    fn node_updated(&mut self, _: &accesskit_consumer::NodeRef<'_>, _: &accesskit_consumer::NodeRef<'_>) {}
    fn focus_moved(
        &mut self,
        _: Option<&accesskit_consumer::NodeRef<'_>>,
        _: Option<&accesskit_consumer::NodeRef<'_>>,
    ) {
    }
    fn node_removed(&mut self, _: &accesskit_consumer::NodeRef<'_>) {}
}

/// What a screen reader is sent, per scene (ADR-0021): the whole tree
/// (what every frame sent before), then what each script step and each
/// steady frame sends now. Each update is also applied to the tree a
/// platform adapter keeps (`accesskit_consumer`), timed: the receiving
/// side's cost, which grows with the nodes sent.
fn a11y(args: &[String]) -> Result<bool, String> {
    let names: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let chosen: Vec<&scenes::Scene> = if names.is_empty() {
        scenes::ALL.iter().collect()
    } else {
        names.iter().map(|n| scene(n)).collect::<Result<_, _>>()?
    };
    let line = |what: &str, sent: &[usize], took: &[Duration], applied: &[Duration]| {
        let mut sorted = sent.to_vec();
        sorted.sort_unstable();
        let at = |q: usize| {
            sorted
                .get(sorted.len().saturating_sub(1).saturating_mul(q) / 100)
                .copied()
                .unwrap_or(0)
        };
        let t = Spread::of(took.iter().copied());
        let a = Spread::of(applied.iter().copied());
        format!(
            "  {what}: sent p50 {} max {} nodes; built p50 {} max {}; adapter p50 {} max {}",
            at(50),
            at(100),
            ms(t.p50),
            ms(t.max),
            ms(a.p50),
            ms(a.max)
        )
    };
    for sc in chosen {
        println!("{}: {}", sc.name, sc.about);
        let mut s = session(sc)?;
        let (mut took, mut applied) = (Vec::new(), Vec::new());
        let mut nodes = 0;
        let mut tree = None;
        for _ in 0..5 {
            let (u, stats) = s.access_probe(true);
            nodes = stats.nodes;
            took.push(stats.took);
            let t0 = Instant::now();
            tree = Some(accesskit_consumer::Tree::new(u, true));
            applied.push(t0.elapsed());
        }
        let mut tree = tree.ok_or("no tree")?;
        println!(
            "  whole tree: {nodes} nodes; built p50 {}; adapter p50 {}",
            ms(Spread::of(took.into_iter()).p50),
            ms(Spread::of(applied.into_iter()).p50)
        );
        let mut step =
            |s: &mut Session, sent: &mut Vec<usize>, took: &mut Vec<Duration>, applied: &mut Vec<Duration>| {
                let (u, p) = s.access_probe(false);
                sent.push(p.sent);
                took.push(p.took);
                let t0 = Instant::now();
                tree.update_and_process_changes(u, &mut Quiet);
                applied.push(t0.elapsed());
            };
        let (mut sent, mut took, mut applied) = (Vec::new(), Vec::new(), Vec::new());
        for l in sc.script {
            command(&mut s, l)?;
            step(&mut s, &mut sent, &mut took, &mut applied);
        }
        println!("{}", line("script steps", &sent, &took, &applied));
        let (mut sent, mut took, mut applied) = (Vec::new(), Vec::new(), Vec::new());
        for _ in 0..30 {
            s.bench(1)?;
            step(&mut s, &mut sent, &mut took, &mut applied);
        }
        println!("{}", line("steady frames", &sent, &took, &applied));
    }
    Ok(true)
}

fn inspect(args: &[String]) -> Result<bool, String> {
    let name = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("--inspect needs a scene")?;
    let mut s = session(scene(name)?)?;
    let mut given = Vec::new();
    let mut words = args.iter();
    while let Some(w) = words.next() {
        if w == "-e" {
            given.push(words.next().ok_or("-e needs a command")?.clone());
        }
    }
    let lines: Vec<String> = if given.is_empty() {
        std::io::stdin()
            .lines()
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?
    } else {
        given
    };
    s.run_script(lines, &mut std::io::stdout().lock())
        .map_err(|e| e.to_string())
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let profiler = dhat::Profiler::new_heap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    install(Theme::default(), VEGA);
    let result = match args.split_first() {
        Some((flag, tail)) if flag == "--run" => run(tail),
        Some((flag, tail)) if flag == "--a11y" => a11y(tail),
        Some((flag, tail)) if flag == "--inspect" => inspect(tail),
        Some((flag, tail)) if flag == "--window" => match tail.first().map(|n| scene(n)) {
            Some(Ok(sc)) => gui::run(
                &format!("stress: {}", sc.name),
                // On the UI thread, where the app's state lives (ADR-0015).
                move || {
                    install(Theme::default(), VEGA);
                    (sc.render)()
                },
                gui::RunOptions {
                    debug: gui::DebugTools {
                        hud: true,
                        attach: true,
                        ..gui::DebugTools::default()
                    },
                    ..gui::RunOptions::default()
                },
            ),
            Some(Err(e)) => Err(e),
            None => Err("--window needs a scene".to_owned()),
        },
        Some((flag, _)) if flag == "--list" => {
            for sc in scenes::ALL {
                println!("{:<10} {}", sc.name, sc.about);
            }
            Ok(true)
        }
        _ => Err("usage: stress --run [scene…] [--cycles=N --monkey=N --budget=MS --growth-kb=N] | --a11y [scene…] | --inspect <scene> [-e CMD]… | --window <scene> | --list".to_owned()),
    };
    // The profile is written when the profiler drops, before any exit.
    #[cfg(feature = "dhat-heap")]
    drop(profiler);
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
