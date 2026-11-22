//! The shadcn gallery: one page per component, and the docs that show them.
//!
//! `cargo run -p shadcn --example docs` opens the docs in a window (see
//! [`docs`]); `-- --docs-trace <dir>` walks them headless into `<dir>`;
//! `-- --shots <dir> [page…] [--dark]` renders each page's script headless
//! into `<dir>/<page>/` (PNG per step, overlay and layout dump beside it);
//! `-- --demos [page…]` lists the shadcn-vue demos each page reproduces and
//! `-- --verdict <dir> [page…] [--dark] [--color=N --shift=N --size=N --no-coverage]`
//! judges them against shadcn-vue's own, forgiving what [`verdict::Tolerance`]
//! says (`crates/shadcn/verdict/run.sh` runs both steps around the browser);
//! `-- --clock` prints the instant the docs are at and its time zone
//! ("<ms since the epoch> <IANA zone>", [`verdict::CLOCK_MS`]), which the
//! verdict pins the browser to;
//! `-- --bench [page…] [--budget=MS --frames=N --json=PATH --baseline=PATH]` times each page and checks
//! it gives back what a pointer walk takes ([`bench`]); `-- --hud` opens the
//! docs with the frame HUD (F10 toggles it); `-- --export <file>` writes the
//! docs as JSON for the website (`site/`, [`docs::export`]);
//! `-- --text-sample <dir>` draws the text sample at 1x and 2x and
//! `-- --text-compare <dir>` measures it against Chrome's
//! (`crates/shadcn/verdict/text.sh` runs both around the browser).
//!
//! The devtools ([`gui::devtools`], `help` lists their commands):
//! `-- --inspect <page|docs> [--size=WxH --scale=N] [-e CMD]…` drives a page
//! (or the whole docs) headless, running each `-e` command, or each line of
//! stdin; `-- --serve [--hud]` opens the docs listening for commands, and
//! `-- --attach [--socket=PATH] [CMD…]` sends commands (or stdin's lines) to
//! the newest app listening.

mod bench;
mod docs;
mod pages;
mod text_sample;
mod verdict;

use std::path::PathBuf;

use gui::prelude::*;
use gui::{Element, ImageSource, Scenario, ScenarioError, Signal, div, text, use_context};
use shadcn::{Theme, provide_theme, theme};

/// The heap counted, for `--bench` and the frame HUD.
#[cfg(not(feature = "dhat-heap"))]
#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

/// Diagnosis (ADR-0023): `--features dhat-heap` writes `dhat-heap.json` on exit,
/// for <https://nnethercote.github.io/dh_view/dh_view.html>.
#[cfg(feature = "dhat-heap")]
#[global_allocator]
static HEAP: dhat::Alloc = dhat::Alloc;

/// A page's render function; it holds the page's state.
pub type Render = Box<dyn FnMut() -> Element>;
/// A scripted run of one page.
pub type Shot = Scenario;

/// One gallery page.
pub struct Page {
    pub name: &'static str,
    /// Build the page's state and return its render function.
    pub render: fn() -> Render,
    /// Drive the page and take frames.
    pub script: fn(&mut Shot) -> Result<(), ScenarioError>,
}

/// Where a page renders: on the gallery's own canvas (shots, verdicts), or in
/// the docs, which title the page themselves and lift each demo into a
/// Preview | Code block of its own. Provided once, before any page renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Host {
    #[default]
    Gallery,
    Docs,
}

fn host() -> Host {
    use_context::<Host>().unwrap_or_default()
}

/// The name [`section`] gives its box; the docs drop a section whose content
/// was only demos.
pub const SECTION: &str = "section";

/// A page's canvas: its title above `content` (in the docs, `content` alone).
pub fn page(title: &str, content: impl IntoElement) -> Element {
    if host() == Host::Docs {
        return content.into_element();
    }
    let t = theme();
    div()
        .size_full()
        .bg(t.background)
        .text_color(t.foreground)
        .p(8.0)
        .flex_col()
        .gap(6.0)
        .child(text(title).text_2xl().font_semibold().tracking_tight())
        .child(content)
        .into_element()
}

/// A labelled row of examples.
pub fn section(title: &str, content: impl IntoElement) -> Element {
    let t = theme();
    div()
        .named(SECTION)
        .flex_col()
        .gap(3.0)
        .child(text(title).text_sm().font_medium().text_color(t.muted_foreground))
        .child(content)
        .into_element()
}

/// One shadcn-vue demo reproduced: `file` is its source,
/// `apps/v4/components/demo/<file>` (e.g. `"ButtonDemo.vue"`). The verdict
/// finds it by that name and lays it out at the width shadcn-vue's is shot at;
/// the docs find it by that name too and centre it in its preview.
pub fn demo(file: &'static str, content: impl IntoElement) -> Element {
    let d = div().named(file);
    match host() {
        Host::Gallery => d.w_px(verdict::SPECIMEN_WIDTH).flex_col().items_start(),
        Host::Docs => d.w_full().flex().justify_center().items_center(),
    }
    .child(content)
    .into_element()
}

/// The photo `name` a shadcn-vue demo loads (`github.com/<name>.png`, an
/// Unsplash `photo-<id>` or `avatar.vercel.sh/<name>`). `verdict/fetch.sh` caches them (third-party, never
/// committed); `None` until it has, and a page then shows what its component
/// shows for an image that has not loaded.
pub fn photo(name: &str) -> Option<ImageSource> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("verdict/.photos")
        .join(format!("{name}.png"));
    ImageSource::from_png(&std::fs::read(path).ok()?)
}

/// The default script: one frame of the page as it opens.
pub fn still(s: &mut Shot) -> Result<(), ScenarioError> {
    s.frame("default").map(|_| ())
}

/// Frames `ms` milliseconds after what `act` does sets a transition or an
/// animation off, named `name` and the time: a page's script compares them
/// with the browser's own paused at the same times. What still runs after
/// the last ends then, and the script goes on with frames of states.
pub fn motion_frames(
    s: &mut Shot,
    name: &str,
    ms: &[u16],
    act: impl FnOnce(&mut Shot) -> Result<(), ScenarioError>,
) -> Result<(), ScenarioError> {
    s.play_motion();
    act(s)?;
    // The frame the motion starts on, as a browser starts it on the next one.
    s.settle()?;
    let start = gui::clock::frame_time();
    for &t in ms {
        s.at(start + f32::from(t) / 1000.0);
        s.frame(&format!("{name} {t}ms"))?;
    }
    s.still_motion();
    s.settle()
}

/// [`motion_frames`] at the times every comparison takes: the start, a
/// quarter and a half of tw-animate's 150ms, and well after.
pub const MOTION_MS: [u16; 4] = [0, 75, 150, 300];

fn shots(dir: PathBuf, only: &[String]) -> Result<(), ScenarioError> {
    for p in selected(only) {
        let mut s = Scenario::new((p.render)(), &dir.join(p.name), 960, 640, 2.0)?;
        (p.script)(&mut s)?;
        println!("{}: {}", p.name, dir.join(p.name).display());
    }
    Ok(())
}

fn selected(only: &[String]) -> impl Iterator<Item = &'static Page> + '_ {
    pages::ALL
        .iter()
        .filter(move |p| only.is_empty() || only.iter().any(|o| o == p.name))
}

/// Print `<page> <demo file>` per reproduced demo, for the browser to shoot.
fn demos(only: &[String]) -> Result<(), ScenarioError> {
    let scratch = std::env::temp_dir().join("docs-demos");
    for p in selected(only) {
        let s = verdict::scenario(p, &scratch)?;
        for (demo, _) in verdict::specimens(s.last()) {
            println!("{} {demo}", p.name);
        }
    }
    Ok(())
}

/// Judge each reproduced demo against its shot under `<dir>/<page>/<demo>/`;
/// writes `verdict.txt` and `verdict.json` there. `Ok(false)` when any FAILs.
fn verdicts(dir: &std::path::Path, only: &[String], tolerance: verdict::Tolerance) -> Result<bool, ScenarioError> {
    let (mut report, mut json, mut scores) = (String::new(), Vec::new(), Vec::new());
    let (mut failed, mut unjudged) = (0_u32, 0_u32);
    let better = verdict::better_list();
    for p in selected(only) {
        let s = verdict::scenario(p, &dir.join(p.name))?;
        let found = verdict::specimens(s.last());
        if found.is_empty() {
            unjudged = unjudged.saturating_add(1);
            report.push_str(&format!("NODEMO   -  {:<36} wraps no demo in demo(..)\n", p.name));
        }
        for (demo, bounds) in found {
            let d = dir.join(p.name).join(demo.trim_end_matches(".vue"));
            std::fs::create_dir_all(&d)?;
            let why = better.iter().find(|(b, _)| b == demo).map(|(_, why)| why.clone());
            let line = match verdict::judge(&s, demo, bounds, &d, why, tolerance) {
                Ok(v) => {
                    if v.grade() == verdict::Grade::Fail {
                        failed = failed.saturating_add(1);
                    }
                    scores.push(v.score());
                    json.push(v.json(p.name));
                    v.to_string()
                }
                Err(e) => {
                    unjudged = unjudged.saturating_add(1);
                    e.to_string()
                }
            };
            report.push_str(&format!("{line}  {}\n", d.display()));
        }
    }
    let mean = if scores.is_empty() {
        0.0
    } else {
        scores.iter().sum::<f64>() / f64::from(u32::try_from(scores.len()).unwrap_or(u32::MAX))
    };
    report.push_str(&format!(
        "\nscore {mean:.0}/100 over {} demos: {failed} FAIL (<{:.0}), {unjudged} not judged; PASS is ≥{:.0}; BETTER per verdict/better.tsv\ntolerance {tolerance}\n",
        scores.len(),
        verdict::FAIL,
        verdict::PASS,
    ));
    print!("{report}");
    std::fs::write(dir.join("verdict.txt"), report)?;
    std::fs::write(dir.join("verdict.json"), format!("[\n{}\n]\n", json.join(",\n")))?;
    Ok(failed == 0)
}

/// The commands given as `-e CMD` (each one line), or else stdin's lines.
fn commands(args: &[String]) -> Result<Vec<String>, String> {
    let mut given = Vec::new();
    let mut words = args.iter();
    while let Some(w) = words.next() {
        if w == "-e" {
            given.push(words.next().ok_or("-e needs a command")?.clone());
        }
    }
    if !given.is_empty() {
        return Ok(given);
    }
    std::io::stdin()
        .lines()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())
}

/// `--inspect`: a headless devtools session on a page, or the whole docs;
/// `Ok(false)` when a command failed.
fn inspect(args: &[String]) -> Result<bool, String> {
    let mut rest = args.iter().filter(|a| !a.starts_with("--")).peekable();
    let mut target = None;
    // The first word that is not a flag, nor a command after `-e`.
    while let Some(w) = rest.next() {
        if w == "-e" {
            rest.next();
            continue;
        }
        target = Some(w.clone());
        break;
    }
    let target = target.ok_or("--inspect: name a page (see `--demos`) or `docs`")?;
    let flag = |key: &str| {
        args.iter().find_map(|a| {
            a.strip_prefix("--")
                .and_then(|f| f.strip_prefix(key))
                .and_then(|v| v.strip_prefix('='))
        })
    };
    let whole = target == "docs";
    let (mut w, mut h) = if whole { (1440, 900) } else { (960, 640) };
    if let Some(size) = flag("size") {
        let (sw, sh) = size.split_once('x').ok_or("--size is WxH")?;
        w = sw.parse().map_err(|_| "--size is WxH")?;
        h = sh.parse().map_err(|_| "--size is WxH")?;
    }
    let scale = flag("scale")
        .map_or(Ok(2.0), str::parse::<f32>)
        .map_err(|_| "--scale is a number")?;
    let render = if whole {
        docs::app()
    } else {
        let page = pages::ALL
            .iter()
            .find(|p| p.name == target)
            .ok_or_else(|| format!("no page {target:?}"))?;
        (page.render)()
    };
    let scenario = Scenario::measuring(render, w, h, scale).map_err(|e| e.to_string())?;
    let mut session = gui::devtools::Session::new(scenario).map_err(|e| e.to_string())?;
    let lines = commands(args)?;
    session
        .run_script(lines, &mut std::io::stdout().lock())
        .map_err(|e| e.to_string())
}

/// `--attach`: send commands to a running app; `Ok(false)` when one failed.
fn attach(args: &[String]) -> Result<bool, String> {
    #[cfg(unix)]
    {
        let socket = match args.iter().find_map(|a| a.strip_prefix("--socket=")) {
            Some(path) => PathBuf::from(path),
            None => gui::devtools::listening()
                .into_iter()
                .next()
                .ok_or("no app is listening; start one with `docs --serve` (or GUI_DEVTOOLS=1)")?,
        };
        let words: Vec<String> = args.iter().filter(|a| !a.starts_with("--socket=")).cloned().collect();
        let lines = if words.is_empty() || words.first().is_some_and(|w| w == "-e") {
            commands(&words)?
        } else {
            // `--attach tree --depth=2`: the words are one command.
            vec![words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")]
        };
        let answer = gui::devtools::request(&socket, &lines).map_err(|e| format!("{}: {e}", socket.display()))?;
        print!("{answer}");
        Ok(!answer.lines().any(|l| l.starts_with("error: ")))
    }
    #[cfg(not(unix))]
    {
        let _ = args;
        Err("attaching needs a Unix socket; use --inspect".to_owned())
    }
}

/// A word quoted when it holds a space, so a command rebuilt from the shell's words splits back the same.
fn quote(word: &str) -> String {
    if word.contains(char::is_whitespace) || word.is_empty() {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        word.to_owned()
    }
}

fn main() {
    #[cfg(feature = "dhat-heap")]
    let _profiler = dhat::Profiler::new_heap();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dark = args.iter().any(|a| a == "--dark");
    let theme_signal = move || {
        provide_theme(Signal::labeled(
            "theme",
            if dark { Theme::dark() } else { Theme::light() },
        ));
    };
    let rest: Vec<String> = args.iter().filter(|a| *a != "--dark").cloned().collect();
    let windowed = rest.first().is_none_or(|f| f == "--serve" || f == "--hud");
    if !windowed {
        // Headless paths run here, on this thread.
        theme_signal();
    }
    match rest.split_first() {
        Some((flag, tail)) if flag == "--shots" => {
            let Some((dir, only)) = tail.split_first() else {
                eprintln!("usage: docs --shots <dir> [page…] [--dark]");
                std::process::exit(2);
            };
            if let Err(e) = shots(PathBuf::from(dir), only) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some((flag, _)) if flag == "--clock" => println!("{} {}", verdict::CLOCK_MS, verdict::CLOCK_ZONE),
        Some((flag, only)) if flag == "--demos" => {
            if let Err(e) = demos(only) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some((flag, tail)) if flag == "--verdict" => {
            let (tolerance, tail) = match verdict::Tolerance::parse(tail) {
                Ok(parsed) => parsed,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            };
            let Some((dir, only)) = tail.split_first() else {
                eprintln!("usage: docs --verdict <dir> [page…] [--dark] [--color=N --shift=N --size=N --no-coverage]");
                std::process::exit(2);
            };
            match verdicts(std::path::Path::new(dir), only, tolerance) {
                Ok(true) => {}
                Ok(false) => std::process::exit(1),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            }
        }
        Some((flag, tail)) if flag == "--docs-trace" => {
            let Some(dir) = tail.first() else {
                eprintln!("usage: docs --docs-trace <dir>");
                std::process::exit(2);
            };
            if let Err(e) = docs::trace(std::path::Path::new(dir)) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some((flag, tail)) if flag == "--bench" => {
            let (options, only) = match bench::Options::parse(tail) {
                Ok(parsed) => parsed,
                Err(e) => {
                    eprintln!(
                        "{e}\nusage: docs --bench [page…] [--budget=MS --frames=N --json=PATH --baseline=PATH] [--dark]"
                    );
                    std::process::exit(2);
                }
            };
            match bench::bench(&only, &options) {
                Ok(true) => {}
                Ok(false) => std::process::exit(1),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(2);
                }
            }
        }
        Some((flag, tail)) if flag == "--text-sample" || flag == "--text-compare" => {
            let Some(dir) = tail.first() else {
                eprintln!("usage: docs {flag} <dir>");
                std::process::exit(2);
            };
            let dir = std::path::Path::new(dir);
            let done = if flag == "--text-sample" {
                text_sample::render(dir)
            } else {
                text_sample::compare(dir).map(|report| print!("{report}"))
            };
            if let Err(e) = done {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some((flag, tail)) if flag == "--export" => {
            let Some(path) = tail.first() else {
                eprintln!("usage: docs --export <docs.json>");
                std::process::exit(2);
            };
            if let Err(e) = docs::export(std::path::Path::new(path)) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        Some((flag, tail)) if flag == "--inspect" => match inspect(tail) {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(e) => {
                eprintln!(
                    "{e}\nusage: docs --inspect <page|docs> [--size=WxH --scale=N] [-e CMD]… (else commands from stdin; `help` lists them)"
                );
                std::process::exit(2);
            }
        },
        Some((flag, tail)) if flag == "--attach" => match attach(tail) {
            Ok(true) => {}
            Ok(false) => std::process::exit(1),
            Err(e) => {
                eprintln!("{e}\nusage: docs --attach [--socket=PATH] [CMD]… (else commands from stdin)");
                std::process::exit(2);
            }
        },
        // A window's app runs on its UI thread: the theme is provided there.
        Some((flag, tail)) if flag == "--serve" => docs::run(
            gui::DebugTools {
                attach: true,
                hud: tail.iter().any(|a| a == "--hud"),
                ..gui::DebugTools::default()
            },
            theme_signal,
        ),
        Some((flag, _)) if flag == "--hud" => docs::run(
            gui::DebugTools {
                hud: true,
                ..gui::DebugTools::default()
            },
            theme_signal,
        ),
        _ => docs::run(gui::DebugTools::default(), theme_signal),
    }
}
