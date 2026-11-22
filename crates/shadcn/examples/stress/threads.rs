//! ADR-0015's goals, measured on the app's own threads with no window
//! ([`gui::offscreen`]): this thread plays the platform's, sending input
//! stamped as it arrives, and a render thread ticks at 120 Hz.
//!
//! - **Busy UI**: a timer keeps the UI thread busy 50 ms of every 100 ms
//!   while the wheel scrolls a list and a box spins. The render thread
//!   scrolls and spins by itself: it must draw every tick.
//! - **Input to present**: a click on a button that changes its colour,
//!   with the UI idle, is shown by the frame presented one frame later.
//!
//! `STRESS_THREADS_DEBUG=1` prints each frame of the busy run: its gap, its wait
//! for the commit, its draw, and the UI's answer time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gui::compositor::FrameStat;
use gui::inspect::ms;
use gui::prelude::*;
use gui::window::Input;
use gui::{Animation, Color, Signal, div, text};

/// The display's interval: 120 Hz.
const INTERVAL: Duration = Duration::from_micros(8_333);
/// How long the UI is busy at a time, and how often.
const BUSY: Duration = Duration::from_millis(50);
const BUSY_EVERY: Duration = Duration::from_millis(100);
const SIZE: (u32, u32) = (800, 600);
/// How many clicks input to present is measured over, and its goal: two
/// intervals (a click lands somewhere in one) and a little.
const CLICKS: usize = 20;
/// No two frames further apart than two intervals (one tick missed at most).
const GAP_GOAL: Duration = Duration::from_micros(8_333 * 2 + 1_000);
const LATENCY_GOAL: Duration = Duration::from_micros(8_333 * 2 + 1_000);

/// Keep the UI thread busy `BUSY` every `BUSY_EVERY` while `on`.
fn busy(on: Arc<AtomicBool>) {
    let again = Arc::clone(&on);
    gui::clock::after(BUSY_EVERY.saturating_sub(BUSY), move || {
        if again.load(Ordering::Relaxed) {
            std::thread::sleep(BUSY);
        }
        busy(Arc::clone(&again));
    });
}

/// A button, a spinning box and a list of 300 rows under them.
fn app(on: Arc<AtomicBool>) -> impl FnMut() -> gui::Element {
    busy(on);
    let pressed = Signal::new(false);
    move || {
        div()
            .size(pct(100.0))
            .child(
                div()
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Row)
                    .gap(32.0)
                    .child(
                        div()
                            .named("button")
                            .width(120.0)
                            .height(40.0)
                            .background(if pressed.get() {
                                Color::hex(0x18181b)
                            } else {
                                Color::hex(0xe4e4e7)
                            })
                            .on_click(move |_| pressed.update(|p| *p = !*p))
                            .child(text("press")),
                    )
                    .child(
                        div()
                            .width(40.0)
                            .height(40.0)
                            .background(Color::hex(0x2563eb))
                            .animation(Animation::spin()),
                    ),
            )
            .child(
                div()
                    .named("list")
                    .height(500.0)
                    .overflow_y(Overflow::Scroll)
                    .overflow_x(Overflow::Hidden)
                    .children((0..300).map(|i| {
                        div()
                            .height(24.0)
                            .background(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
                            .child(text(format!("row {i}")))
                    })),
            )
            .into_element()
    }
}

/// The frames drawn in `window`, by the vblank they began at.
fn within(frames: &[FrameStat], from: Instant, to: Instant) -> Vec<FrameStat> {
    frames
        .iter()
        .filter(|f| f.vsync >= from && f.vsync < to)
        .copied()
        .collect()
}

/// Run both checks: the report's lines, and whether one failed (a goal
/// missed is reported: timing on a loaded machine is not a failure).
pub fn run() -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    // This thread plays the platform's: scheduled as a window's main thread is.
    let platform = gui::priority::set_current(gui::priority::ThreadType::Platform);
    let on = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&on);
    let started = gui::offscreen(
        move || {
            shadcn::install(shadcn::Theme::default(), shadcn::VEGA);
            app(flag)
        },
        gui::RunOptions::default(),
        SIZE,
        1.0,
        INTERVAL,
    );
    let mut off = match started {
        Ok(o) => o,
        Err(e) => return (vec![format!("  could not start: {e}")], true),
    };
    let mut failed = false;
    std::thread::sleep(Duration::from_millis(400));
    let priorities: Vec<String> = off
        .priorities()
        .iter()
        .map(|(name, applied)| format!("{name} {applied}"))
        .collect();
    lines.push(format!("  scheduled  platform {platform}; {}", priorities.join("; ")));

    // Input to present, the UI idle.
    let mut latencies = Vec::new();
    off.input(Input::PointerMoved { x: 60.0, y: 20.0 });
    std::thread::sleep(Duration::from_millis(100));
    drop(off.take_frames());
    for _ in 0..CLICKS {
        let shown = off.presented().commit;
        let at = Instant::now();
        off.input(Input::MouseDown { x: 60.0, y: 20.0 });
        off.input(Input::MouseUp { x: 60.0, y: 20.0 });
        // The first frame showing a commit of a main frame granted after the
        // click (it may be drawn in a frame whose vblank came before it: a
        // main frame is granted mid-frame).
        let mut found = None;
        // Whether it shows, not how soon (the goal below): a software GPU
        // on a two-core runner may take far longer than a frame.
        while found.is_none() && at.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
            found = off
                .take_frames()
                .into_iter()
                .find(|f| f.commit.is_some_and(|c| c > shown) && f.granted.is_some_and(|g| g >= at));
        }
        if let Some(f) = found {
            latencies.push(f.present_at.saturating_duration_since(at));
            if std::env::var_os("STRESS_THREADS_DEBUG").is_some() {
                eprintln!(
                    "click: granted +{} vblank +{} ui {:?} draw {} present +{}",
                    f.granted
                        .map_or_else(|| "-".to_owned(), |g| ms(g.saturating_duration_since(at))),
                    ms(f.vsync.duration_since(at)),
                    f.ui_took,
                    ms(f.submitted.duration_since(f.drawing)),
                    ms(f.present_at.saturating_duration_since(at))
                );
            }
        }
        std::thread::sleep(Duration::from_millis(30));
        drop(off.take_frames());
    }
    latencies.sort();
    let worst = latencies.last().copied().unwrap_or(Duration::MAX);
    let median = latencies.get(latencies.len() / 2).copied().unwrap_or(Duration::MAX);
    let within_goal = latencies.iter().filter(|l| **l <= LATENCY_GOAL).count();
    // Every click shows; how soon is the goal, reported.
    let shown_all = latencies.len() == CLICKS;
    failed |= !shown_all;
    lines.push(format!(
        "  input to present  median {}  worst {}  {within_goal}/{CLICKS} within {} (one frame: the UI is granted its frame at once, shown at the next vblank it makes){}",
        ms(median),
        ms(worst),
        ms(LATENCY_GOAL),
        if shown_all { "" } else { "  FAIL: a click was never shown" }
    ));

    // The UI busy: scroll and spin go on at the display's rate.
    on.store(true, Ordering::Relaxed);
    off.input(Input::PointerMoved { x: 200.0, y: 300.0 });
    std::thread::sleep(Duration::from_millis(200));
    drop(off.take_frames());
    let from = Instant::now();
    let shown_before = off.presented();
    let until = from.checked_add(Duration::from_millis(1500)).unwrap_or(from);
    while Instant::now() < until {
        off.wheel((0.0, 3.0), false);
        std::thread::sleep(Duration::from_millis(4));
    }
    let to = Instant::now();
    on.store(false, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(100));
    let frames = within(&off.take_frames(), from, to);
    if std::env::var_os("STRESS_THREADS_DEBUG").is_some() {
        for (a, b) in frames.iter().zip(frames.iter().skip(1)) {
            eprintln!(
                "vsync +{} wait {} draw {} commit {:?} ui {:?} without_ui {}",
                ms(b.vsync.duration_since(a.vsync)),
                ms(b.drawing.duration_since(b.vsync)),
                ms(b.submitted.duration_since(b.drawing)),
                b.commit,
                b.ui_took,
                b.without_ui
            );
        }
    }
    let shown_after = off.presented();
    let ticks = to.duration_since(from).as_secs_f64() / INTERVAL.as_secs_f64();
    let drawn = frames.len();
    let without_ui = frames.iter().filter(|f| f.without_ui).count();
    let mut gaps: Vec<Duration> = frames
        .iter()
        .zip(frames.iter().skip(1))
        .map(|(a, b)| b.vsync.duration_since(a.vsync))
        .collect();
    gaps.sort();
    let worst_gap = gaps.last().copied().unwrap_or(Duration::MAX);
    let commits: Vec<Instant> = frames.iter().filter(|f| f.commit.is_some()).map(|f| f.vsync).collect();
    let longest_ui = commits
        .iter()
        .zip(commits.iter().skip(1))
        .map(|(a, b)| b.duration_since(*a))
        .max()
        .unwrap_or_default();
    let rate = f64::from(u32::try_from(drawn).unwrap_or(u32::MAX)) / to.duration_since(from).as_secs_f64();
    let scrolled = shown_after
        .scroll
        .iter()
        .zip(&shown_before.scroll)
        .map(|(a, b)| a.offset.1 - b.offset.1)
        .fold(0.0_f32, f32::max);
    let missed: f64 = gaps
        .iter()
        .map(|g| (g.as_secs_f64() / INTERVAL.as_secs_f64()).round() - 1.0)
        .sum();
    // What must hold whatever the machine's load: the compositor drew
    // while the UI owed it a frame, the UI was really late, the wheel
    // scrolled. How close to every tick is the goal, reported.
    // Commits land on vblanks: a UI busy for `BUSY` leaves a gap of whole
    // ticks, which may fall a hair short of it (6 × 8.333 ms = 49.998 ms).
    let works = without_ui > 0 && longest_ui.saturating_add(INTERVAL) > BUSY && scrolled > 0.0;
    failed |= !works;
    let goal = f64::from(u32::try_from(drawn).unwrap_or(0)) >= ticks * 0.95 && worst_gap <= GAP_GOAL;
    lines.push(format!(
        "  UI busy {} of every {}: {drawn} frames in {:.0} ticks ({rate:.0} Hz, {missed:.0} missed, worst gap {}), {without_ui} without the UI, longest UI wait {}, scrolled {scrolled:.0} px; 120 Hz goal {}{}",
        ms(BUSY),
        ms(BUSY_EVERY),
        ticks,
        ms(worst_gap),
        ms(longest_ui),
        if goal { "met" } else { "missed" },
        if works { "" } else { "  FAIL: the compositor did not draw without the UI" }
    ));
    (lines, failed)
}
