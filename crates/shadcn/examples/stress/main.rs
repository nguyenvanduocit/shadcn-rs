//! Stress scenes for `gui` and shadcn, driven by the devtools ([`gui::devtools`]).
//!
//! ```sh
//! cargo run --release -p shadcn --example stress -- --run [scene…] [--cycles=N --monkey=N --budget=MS]
//! cargo run --release -p shadcn --example stress -- --inspect <scene> [-e CMD]…   # else stdin
//! cargo run --release -p shadcn --example stress -- --window <scene>              # with HUD and devtools socket
//! cargo run --release -p shadcn --example stress -- --list
//! ```
//!
//! `--run` takes each scene through its script, times its steady frames,
//! repeats its cycle and checks the heap gives back what a cycle takes, then
//! lets the monkey loose on it ([`gui::devtools`]'s `monkey`). A scene
//! `FAIL`s when a command fails, the monkey finds a panic or a failed layout,
//! or a cycle keeps memory. Slow frames are reported, not failed: the scenes
//! are built to be slow.

mod scenes;

use std::time::{Duration, Instant};

use gui::devtools::Session;
use gui::heap;
use gui::inspect::{Spread, bytes, ms};
use gui::{Element, Scenario, Signal};
use shadcn::{Theme, provide_theme};

/// The heap counted, for the leak checks and the reports.
#[global_allocator]
static HEAP: &gui::heap::Counting = gui::heap::COUNTING;

/// A scene's render function; it holds the scene's state.
pub type Render = Box<dyn FnMut() -> Element>;

/// What `--run` judges by.
struct Options {
    cycles: usize,
    monkey: usize,
    budget: Duration,
    /// What one cycle may leave held, on average, past the first.
    growth: u64,
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

fn session(scene: &scenes::Scene) -> Result<Session<Render>, String> {
    let scenario = Scenario::measuring((scene.render)(), scene.size.0, scene.size.1, 1.0).map_err(|e| e.to_string())?;
    Session::new(scenario).map_err(|e| e.to_string())
}

/// A command run for its effect; the thread's mail (a feed) arrives first.
fn command(s: &mut Session<Render>, line: &str) -> Result<String, String> {
    if line == "frame" {
        // What another thread sends lands between frames; give it time to.
        std::thread::sleep(scenes::FEED_SETTLE);
    }
    s.run(line)
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

    // The script: each action timed by the frame it caused.
    let mut slowest = (Duration::ZERO, String::new());
    for line in scene.script {
        match command(&mut s, line) {
            Ok(_) => {
                if let Some(r) = s.log().last()
                    && r.timings.total() > slowest.0
                {
                    slowest = (r.timings.total(), (*line).to_owned());
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
    };
    let chosen: Vec<&scenes::Scene> = if names.is_empty() {
        scenes::ALL.iter().collect()
    } else {
        names.iter().map(|n| scene(n)).collect::<Result<_, _>>()?
    };
    let mut fails = Vec::new();
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    provide_theme(Signal::labeled("theme", Theme::light()));
    let result = match args.split_first() {
        Some((flag, tail)) if flag == "--run" => run(tail),
        Some((flag, tail)) if flag == "--inspect" => inspect(tail),
        Some((flag, tail)) if flag == "--window" => match tail.first().map(|n| scene(n)) {
            Some(Ok(sc)) => gui::run(
                &format!("stress: {}", sc.name),
                (sc.render)(),
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
        _ => Err("usage: stress --run [scene…] [--cycles=N --monkey=N --budget=MS --growth-kb=N] | --inspect <scene> [-e CMD]… | --window <scene> | --list".to_owned()),
    };
    match result {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
