//! Input logs replayed through the compositor and a UI pipeline on a fake
//! display clock (ADR-0015's "tests replay recorded event logs").
//!
//! A log ([`super::input_log`]) is fed, event by event at its own time,
//! through what a window runs: [`Router::route`] (the platform thread),
//! the compositor's wheel and gesture handling (the render thread), and,
//! for the rest, the UI thread's drag resampling ([`PointerResampler`]) and
//! the pipeline's hit testing against what was on screen when the event
//! came. Frames are drawn at the vblanks of a display at 60, 120 or 144 Hz,
//! composed at their present time (one interval after the vblank), with the
//! render thread's schedule: a main frame for the UI when it asked, else a
//! compositor-only frame while something moves, else nothing. The UI
//! answers at once, or only every few vblanks (a busy UI). Everything runs
//! on one thread on the scenario clock, so a replay is the same every time.
//!
//! The scene is a probe: a scroll container of 2 000 rows 48 px tall,
//! opened in its middle, each row clickable, and beside it a pad 100 px
//! wide that takes drags. Every log in `crates/gui/tests/input_logs/` is
//! replayed against the checks of [`check`]; the synthesized ones also
//! against what their sequence is there to show.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use super::input_log::{Event, Header, InputLog, Observation, Platform, Routed, Router, Stamped, Told, WheelDelta};
use super::scroll::{THUMB_FADE, THUMB_SHOWN};
use super::{Shown, WheelPhase};
use crate::app::Scenario;
use crate::clock;
use crate::color::Color;
use crate::element::{IntoElement, div};
use crate::events::DragPhase;
use crate::prelude::*;
use crate::scroll::ScrollHandle;
use crate::window::{Input, PointerResampler};

/// A row's height in the probe.
const ROW: f32 = 48.0;
const ROWS: i32 = 2000;
/// The drag pad's width, at the window's right.
const PAD: f32 = 100.0;
/// Where the probe's list opens: its middle, so no log reaches an edge.
const START: f32 = 48_000.0;
/// When a log's time 0 falls on the scenario clock, seconds.
const T0: f64 = 1.0;
/// How long frames go on after a log's last event, seconds.
const TAIL: f64 = 3.0;
/// ADR-0015's constants, as the checks expect them.
const PREDICT_FOR: f64 = 0.035;
const MAX_PREDICTION: f64 = 0.008;
const RESAMPLE_LATENCY: f64 = 0.005;
const RESAMPLE_MIN_DELTA: f64 = 0.002;
const FLING_TAU: f64 = 0.325;
const FLING_MIN: f64 = 50.0;

/// A frame drawn.
#[derive(Clone, Debug)]
struct Frame {
    vblank: f64,
    present: f64,
    /// The UI ran a main frame for it.
    ui: bool,
    /// The list's offset shown, unsnapped.
    offset: f32,
}

/// A wheel event as the render thread took it.
#[derive(Clone, Debug)]
struct Wheel {
    at: f64,
    dy: f32,
    precise: bool,
    ended: bool,
    /// It found a scroll container under the pointer.
    hit: bool,
}

/// A primary press and release on the list.
#[derive(Clone, Debug)]
struct Press {
    down: (f64, f32, f32, f32),
    up: (f64, f32, f32, f32),
    /// The row the click handler was called for, if one.
    clicked: Option<i32>,
}

/// What a drag handler was told, and when the frame that told it is presented.
#[derive(Clone, Debug)]
struct Dragged {
    phase: DragPhase,
    x: f32,
    y: f32,
    /// The frame's present time; `None` for what arrived with an event
    /// (the press, the release and the move flushed before it).
    present: Option<f64>,
}

/// A replay's record.
#[derive(Debug, Default)]
struct Run {
    hz: f64,
    ui_every: u32,
    frames: Vec<Frame>,
    wheels: Vec<Wheel>,
    presses: Vec<Press>,
    drags: Vec<Dragged>,
    /// The raw pointer samples of each drag: (when, x, y).
    raw: Vec<Vec<(f64, f32, f32)>>,
    /// At the end: the UI's offset, the compositor's, and whether it still owed scroll.
    ui_final: f32,
    shown_final: f32,
    owed: bool,
}

fn scrolled(s: &Scenario) -> (f32, f32) {
    s.presented()
        .scroll
        .first()
        .map_or((f32::NAN, f32::NAN), |x| (x.offset.1, -x.applied.1))
}

/// Replay `log` on a display at `hz`, the UI answering every `ui_every`th
/// vblank it is owed one at.
fn replay(log: &InputLog, hz: f64, ui_every: u32) -> Run {
    replay_probe(log, hz, ui_every, false)
}

/// [`replay`], the probe's list carrying a wheel handler that declines
/// every wheel when `declining`: the compositor hands each to the UI,
/// whose route scrolls the list.
fn replay_probe(log: &InputLog, hz: f64, ui_every: u32, declining: bool) -> Run {
    let Header {
        size, scale, platform, ..
    } = log.header;
    let handle = ScrollHandle::new();
    let clicked: Rc<RefCell<Vec<i32>>> = Rc::default();
    let drags: Rc<RefCell<Vec<(DragPhase, f32, f32)>>> = Rc::default();
    let (h, c, d) = (handle, Rc::clone(&clicked), Rc::clone(&drags));
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            let d = Rc::clone(&d);
            let list = div()
                .height(pct(100.0))
                .flex(Flex::grow(1.0))
                .overflow_y(Overflow::Scroll)
                .overflow_x(Overflow::Hidden)
                .track_scroll(&h)
                .children((0..ROWS).map(move |i| {
                    let c = Rc::clone(&c);
                    div()
                        .height(ROW)
                        .background(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
                        .on_click(move |_| c.borrow_mut().push(i))
                }));
            let list = if declining { list.on_wheel(|_| false) } else { list };
            div()
                .size(pct(100.0))
                .display(Display::Flex)
                .flex_direction(FlexDirection::Row)
                .child(list)
                .child(
                    div()
                        .width(PAD)
                        .height(pct(100.0))
                        .background(Color::hex(0xe4e4e7))
                        .on_drag(move |e| d.borrow_mut().push((e.phase, e.x, e.y))),
                )
                .into_element()
        },
        size.0,
        size.1,
        scale,
    )
    .expect("a scenario");
    s.settle().expect("the first frame");
    s.act("open in the middle", || handle.scroll_to((0.0, START)));
    s.settle().expect("a frame");
    assert!(
        (scrolled(&s).0 - START).abs() < 0.01,
        "the probe opens at {START}: {:?}",
        scrolled(&s)
    );
    s.parts().1.compositor.set_flings(!platform.flings_itself());

    let interval = 1.0 / hz;
    let events: Vec<&Stamped> = log.events.iter().collect();
    let end = events.last().map_or(T0, |e| e.at + T0) + TAIL;
    let mut run = Run {
        hz,
        ui_every,
        ..Run::default()
    };
    let mut router = Router::default();
    let mut resampler = PointerResampler::default();
    let mut pressed = false;
    let mut down: Option<(f64, f32, f32, f32)> = None;
    let mut wants = false;
    let mut dirty = false;
    let mut owed_for = 0_u32;
    let mut next = 0_usize;
    let mut k = 0_u32;
    loop {
        let vblank = T0 + f64::from(k) * interval;
        k = k.saturating_add(1);
        if vblank > end {
            break;
        }
        // What arrived before this vblank, each at its own time.
        while let Some(e) = events.get(next).filter(|e| e.at + T0 <= vblank) {
            next = next.saturating_add(1);
            let at = e.at + T0;
            match router.route(e.event.clone()) {
                Routed::Wheel {
                    point,
                    delta,
                    precise,
                    phase,
                } => {
                    let h = s.parts().1;
                    let seen = h.shown().seen();
                    let c = &mut h.compositor;
                    let ended = matches!(
                        phase,
                        WheelPhase::Ended | WheelPhase::Cancelled | WheelPhase::MomentumEnded
                    );
                    let time = u64::try_from(Duration::from_secs_f64(at.max(0.0)).as_nanos()).unwrap_or(u64::MAX);
                    // The render thread's dispatch, every outcome as the live
                    // threads take it: handed to the UI with what the
                    // compositor scrolled before it (ADR-0033), the UI
                    // thread runs it in the frame shown as it came
                    // (`window::ui_loop`).
                    let hit = match c.dispatch_wheel(super::WheelInput {
                        point,
                        delta,
                        time,
                        precise,
                        phase,
                        seen,
                    }) {
                        super::Dispatched::Ui(input) => {
                            let shown = s.shown();
                            if let Some((x, y)) = resampler.flush() {
                                s.pipeline_mut().input_shown(Input::PointerMoved { x, y }, &shown);
                            }
                            let out = s.send(input);
                            wants |= out.redraw;
                            out.redraw
                        }
                        super::Dispatched::Scrolled => true,
                        super::Dispatched::Nothing => false,
                    };
                    run.wheels.push(Wheel {
                        at,
                        dy: delta.1,
                        precise,
                        ended,
                        hit,
                    });
                    if hit {
                        dirty = true;
                        wants = true;
                    }
                }
                Routed::Told(told) => {
                    s.take_pull();
                    s.told(&told).expect("a size");
                    wants = true;
                }
                Routed::Ui(input) => {
                    // The UI thread's handling (`window::ui_loop`), each
                    // input with what the compositor scrolled before the
                    // render thread handed it on (ADR-0033); a held move
                    // takes that in as it comes.
                    wants |= s.take_pull();
                    let shown = s.shown();
                    let on_screen = scrolled(&s).1;
                    match input {
                        Input::PointerMoved { x, y } if pressed => {
                            resampler.push(f64_f32(at), x, y);
                            if let Some(r) = run.raw.last_mut() {
                                r.push((at, x, y));
                            }
                            wants = true;
                            continue;
                        }
                        Input::MouseDown { x, y } => {
                            pressed = true;
                            down = Some((at, x, y, on_screen));
                            run.raw.push(Vec::new());
                        }
                        Input::MouseUp { .. } | Input::Blur => {
                            pressed = false;
                            if let Some((x, y)) = resampler.flush() {
                                s.pipeline_mut().input_shown(Input::PointerMoved { x, y }, &shown);
                            }
                        }
                        _ => {}
                    }
                    let before = clicked.borrow().len();
                    let out = s.pipeline_mut().input_shown(input.clone(), &shown);
                    wants |= out.redraw;
                    if let Input::MouseUp { x, y } = input
                        && let Some(d) = down.take()
                    {
                        run.presses.push(Press {
                            down: d,
                            up: (at, x, y, on_screen),
                            clicked: clicked.borrow().get(before).copied(),
                        });
                    }
                    for (phase, x, y) in drags.borrow_mut().drain(..) {
                        run.drags.push(Dragged {
                            phase,
                            x,
                            y,
                            present: None,
                        });
                    }
                }
            }
        }
        let present = vblank + interval;
        let at = f64_f32(present);
        let animating = s.parts().1.compositor.animating(f64_f32(vblank));
        if wants {
            owed_for = owed_for.saturating_add(1);
        }
        let ui = wants && owed_for >= ui_every;
        if ui {
            clock::advance_to(Duration::from_secs_f64(present));
            if let Some((x, y)) = resampler.sample(at) {
                let shown = s.shown();
                s.pipeline_mut().input_shown(Input::PointerMoved { x, y }, &shown);
            }
            s.settle().expect("a frame");
            wants = false;
            owed_for = 0;
        } else if dirty || animating {
            // A main frame owed and not answered yet draws nothing by
            // itself: the render thread waits for it asleep.
            s.parts().1.draw(at);
            // Presented: hover converges on it, as the window's UI does
            // when idle.
            let shown = s.shown();
            wants |= s.pipeline_mut().converge(&shown);
        } else {
            continue;
        }
        for (phase, x, y) in drags.borrow_mut().drain(..) {
            run.drags.push(Dragged {
                phase,
                x,
                y,
                present: Some(present),
            });
        }
        dirty = false;
        // A fling or a resampled gesture moved on: the UI takes it in.
        wants |= s.parts().1.compositor.scrolled();
        run.frames.push(Frame {
            vblank,
            present,
            ui,
            offset: scrolled(&s).0,
        });
    }
    run.ui_final = handle.offset().1;
    run.shown_final = scrolled(&s).0;
    run.owed = s.parts().1.compositor.scrolled();
    run
}

fn f64_f32(v: f64) -> f32 {
    crate::convert::f64_to_f32(v)
}

fn u32_f32(v: u32) -> f32 {
    crate::convert::u32_to_f32(v)
}

/// Where the list would be at `t` if every wheel step applied when it came
/// and a gesture moved evenly between its events: the true position.
fn truth(run: &Run, t: f64) -> f64 {
    let mut total = f64::from(START);
    let mut last: Option<(f64, f64)> = None;
    for w in run.wheels.iter().filter(|w| w.hit) {
        let dy = f64::from(w.dy);
        if w.at > t {
            if w.precise
                && let Some((ta, pa)) = last
                && w.at - ta < 0.25
            {
                return pa + dy * (t - ta) / (w.at - ta);
            }
            return total;
        }
        total += dy;
        last = w.precise.then_some((w.at, total));
    }
    total
}

/// What each fling of ours carried the list: (when the fingers lifted,
/// how far), measured at the last frame before the next wheel event.
fn carried(run: &Run) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    for (i, w) in run.wheels.iter().enumerate().filter(|(_, w)| w.precise && w.ended) {
        let next = run.wheels.get(i.saturating_add(1)).map_or(f64::INFINITY, |n| n.at);
        let base = truth(run, w.at) + out.iter().map(|c| c.1).sum::<f64>();
        let end = run
            .frames
            .iter()
            .rev()
            .find(|f| f.vblank > w.at && f.vblank < next)
            .map_or(base, |f| f64::from(f.offset));
        out.push((w.at, end - base));
    }
    out
}

/// How fast the true position moves around `t`, pixels a second: the
/// fastest between two events from `from` to `t`; and the longest gap
/// between those events.
fn speed(run: &Run, from: f64, t: f64) -> (f64, f64) {
    let hits: Vec<&Wheel> = run.wheels.iter().filter(|w| w.hit && w.precise).collect();
    hits.windows(2)
        .filter(|p| p[1].at >= from && p[0].at <= t && p[1].at - p[0].at < 0.25)
        .map(|p| {
            (
                f64::from(p[1].dy.abs()) / (p[1].at - p[0].at).max(1e-3),
                p[1].at - p[0].at,
            )
        })
        .fold((0.0, 0.0), |a, b| (a.0.max(b.0), a.1.max(b.1)))
}

/// What the run must show, whatever the log: returns a line of measures.
fn check(name: &str, log: &InputLog, run: &Run) -> String {
    let what = format!("{name} at {} Hz, UI every {}", run.hz, run.ui_every);
    let interval = 1.0 / run.hz;
    let flings = !log.header.platform.flings_itself();
    let total: f32 = run.wheels.iter().filter(|w| w.hit).map(|w| w.dy).sum();

    // Scroll is never lost or applied twice: once everything settles, the
    // UI holds what the compositor shows, and that is every step taken (a
    // fling of ours carries on in the gesture's direction).
    assert!(!run.owed, "{what}: scroll still owed to the UI at the end");
    assert!(
        (run.ui_final - run.shown_final).abs() < 0.01,
        "{what}: the UI took in {} but the compositor shows {}",
        run.ui_final,
        run.shown_final
    );
    let moved = run.shown_final - START;
    let flung = moved - total;
    let flung_any = flings && run.wheels.iter().any(|w| w.ended && w.precise);
    if flung_any {
        assert!(
            flung.abs() < 20_000.0 && (flung == 0.0 || flung.signum() == total.signum() || total == 0.0),
            "{what}: flung {flung} against the gesture"
        );
    } else {
        assert!(
            (moved - total).abs() < 0.05,
            "{what}: moved {moved}, the steps add to {total}"
        );
    }

    // After the last event of a gesture, prediction stops within 35 ms:
    // the list stays where the events put it, and nothing is drawn.
    let mut quiet = 0_usize;
    for (i, w) in run.wheels.iter().enumerate().filter(|(_, w)| w.hit || w.ended) {
        let next = run.wheels.get(i.saturating_add(1)).map_or(f64::INFINITY, |n| n.at);
        if next - w.at < PREDICT_FOR + 3.0 * interval {
            continue;
        }
        if flings && w.precise && (w.ended || !w.hit) {
            // A fling of ours runs; [`check_flings`] follows it.
            continue;
        }
        if flings
            && w.precise
            && w.hit
            && run
                .wheels
                .get(i.saturating_add(1))
                .is_some_and(|n| n.ended && n.at - w.at < 0.15)
        {
            continue;
        }
        quiet = quiet.saturating_add(1);
        let settled: Vec<&Frame> = run
            .frames
            .iter()
            .filter(|f| f.present >= w.at + PREDICT_FOR && f.vblank < next)
            .collect();
        // Where the events put it (and, where we fling, a fling before).
        let expected = if flings {
            settled.first().map_or(f64::NAN, |f| f64::from(f.offset))
        } else {
            truth(run, w.at)
        };
        for f in settled {
            assert!(
                (f64::from(f.offset) - expected).abs() < 0.05,
                "{what}: {:.1} ms after the event at {:.3} s the list shows {} where the events put it at {expected}",
                (f.present - w.at) * 1000.0,
                w.at - T0,
                f.offset
            );
        }
        // Past 35 ms the compositor draws only to fade the scrollbar out:
        // nothing moves, and once it has faded nothing is drawn.
        let late: Vec<&Frame> = run
            .frames
            .iter()
            .filter(|f| !f.ui && f.vblank >= w.at + PREDICT_FOR + interval && f.vblank < next)
            .collect();
        assert!(
            late.iter().all(|f| (f64::from(f.offset) - expected).abs() < 0.05),
            "{what}: the list moved more than 35 ms after the last event at {:.3} s",
            w.at - T0,
        );
        // It was last scrolled as the prediction ran out, then shows and fades.
        let faded = w.at + PREDICT_FOR + f64::from(THUMB_SHOWN + THUMB_FADE) + interval;
        let drawn = late.iter().filter(|f| f.vblank >= faded).count();
        assert_eq!(
            drawn,
            0,
            "{what}: the compositor drew {drawn} frames after the scrollbar faded, past the event at {:.3} s",
            w.at - T0,
        );
    }

    // While a gesture runs, each frame shows about where the content truly
    // is at its present time: never more than the frame's own lag behind,
    // or a prediction ahead (a lost or doubled step is far more).
    let carry = if flings { carried(run) } else { Vec::new() };
    let mut worst: f64 = 0.0;
    let mut jitter: f64 = 0.0;
    let mut last_step: Option<f64> = None;
    for pair in run.frames.windows(2) {
        let [a, f] = pair else { continue };
        let active = run
            .wheels
            .iter()
            .any(|w| w.hit && w.precise && w.at <= f.vblank && f.present <= w.at + PREDICT_FOR);
        // A fling of ours runs from a lift to the next event ([`flings`]).
        let flinging = flings
            && run.wheels.iter().any(|w| w.ended && w.at <= f.vblank)
            && run
                .wheels
                .iter()
                .rev()
                .find(|w| w.at <= f.vblank && (w.hit || w.ended))
                .is_some_and(|w| w.ended);
        if !active || flinging {
            last_step = None;
            continue;
        }
        // The flings of ours before carried it on.
        let truly = truth(run, f.present) + carry.iter().filter(|c| c.0 < f.vblank).map(|c| c.1).sum::<f64>();
        let err = f64::from(f.offset) - truly;
        // The frame shows the events up to its vblank, the last of them up
        // to a gap before it, predicted up to 8 ms on; the truth is at its
        // present, an interval after the vblank.
        let (v, gap) = speed(run, f.present - 0.05 - interval, f.present);
        let bound = v * (interval + gap.max(MAX_PREDICTION)) + 1.0;
        assert!(
            err.abs() <= bound,
            "{what}: at {:.3} s the list shows {} where it truly is {truly} ({err:.1} px off, more than {bound:.1})",
            f.present - T0,
            f.offset,
        );
        worst = worst.max(err.abs());
        let step = f64::from(f.offset - a.offset) / (f.present - a.present) * interval;
        if let Some(l) = last_step {
            jitter = jitter.max((step - l).abs());
        }
        last_step = Some(step);
    }

    // A click lands on the row that was on screen under it, and only when
    // the press and the release were on the same one.
    let list = u32_f32(log.header.size.0) - PAD;
    let row_at = |y: f32, offset: f32| ((y + offset) / ROW).floor();
    let near_edge =
        |y: f32, offset: f32| ((y + offset) / ROW).fract().min(1.0 - ((y + offset) / ROW).fract()) * ROW < 1.0;
    let mut clicks = 0_usize;
    for p in &run.presses {
        let (_, dx, dy, doff) = p.down;
        let (_, ux, uy, uoff) = p.up;
        if dx >= list || ux >= list || near_edge(dy, doff) || near_edge(uy, uoff) {
            continue;
        }
        let (rd, ru) = (row_at(dy, doff), row_at(uy, uoff));
        if rd == ru {
            clicks = clicks.saturating_add(1);
            assert_eq!(
                p.clicked.map(f64::from),
                Some(f64::from(ru)),
                "{what}: a click at {:.3} s hit row {:?}, the screen showed row {ru} there",
                p.up.0 - T0,
                p.clicked
            );
        } else {
            assert_eq!(
                p.clicked, None,
                "{what}: pressed on row {rd}, released on row {ru}: no click"
            );
        }
    }

    // A drag sees one move a frame, where the pointer was 5 ms before the
    // frame is presented, and on release the last raw position.
    let mut moves = 0_usize;
    let mut raw_moves = 0_usize;
    let mut drag = 0_usize;
    let mut by_frame: Option<f64> = None;
    for (i, d) in run.drags.iter().enumerate() {
        match d.phase {
            DragPhase::Start => {
                by_frame = None;
            }
            DragPhase::Move => {
                let samples = run.raw.get(drag).map_or(&[][..], Vec::as_slice);
                if let Some(p) = d.present {
                    assert_ne!(by_frame, Some(p), "{what}: two drag moves in one frame");
                    by_frame = Some(p);
                    moves = moves.saturating_add(1);
                    let seen: Vec<(f64, f32, f32)> = samples.iter().copied().filter(|s| s.0 <= p - interval).collect();
                    let want = resampled(&seen, p - RESAMPLE_LATENCY);
                    assert!(
                        (want.0 - d.x).abs() < 0.01 && (want.1 - d.y).abs() < 0.01,
                        "{what}: the drag move of the frame at {:.4} s is at ({}, {}), the samples put it at {want:?}",
                        p - T0,
                        d.x,
                        d.y
                    );
                } else {
                    // Flushed before the release: the last raw position.
                    let last = samples.last().map(|s| (s.1, s.2));
                    assert_eq!(last, Some((d.x, d.y)), "{what}: the release flushes the last move");
                    assert!(
                        matches!(run.drags.get(i.saturating_add(1)), Some(n) if n.phase == DragPhase::End),
                        "{what}: the flushed move comes just before the release"
                    );
                }
            }
            DragPhase::End | DragPhase::Cancel => {
                raw_moves = raw_moves.saturating_add(run.raw.get(drag).map_or(0, Vec::len));
                drag = drag.saturating_add(1);
            }
        }
    }
    let ui_frames = run.frames.iter().filter(|f| f.ui).count();
    format!(
        "{what}: {} frames ({ui_frames} UI), moved {moved:.1} px (steps {total:.1}, flung {:.1}), {quiet} quiet gaps, worst error {worst:.1} px, step jitter {jitter:.1} px, {clicks} clicks, {moves} drag moves from {raw_moves} samples",
        run.frames.len(),
        if flung_any { flung } else { 0.0 }
    )
}

/// Where a drag's samples put the pointer at `t`: interpolated between the
/// samples around it, or extrapolated from the last two at most
/// `min(8 ms, half their gap)`, not from samples closer than 2 ms.
fn resampled(samples: &[(f64, f32, f32)], t: f64) -> (f32, f32) {
    let Some(&(t1, x1, y1)) = samples.last() else {
        return (f32::NAN, f32::NAN);
    };
    let lerp = |a: f32, b: f32, k: f64| a + (b - a) * f64_f32(k);
    if t <= t1 {
        for w in samples.windows(2) {
            let [(ta, xa, ya), (tb, xb, yb)] = *w else { continue };
            if ta <= t && t <= tb {
                let k = if tb > ta { (t - ta) / (tb - ta) } else { 1.0 };
                return (lerp(xa, xb, k), lerp(ya, yb, k));
            }
        }
        return samples.first().map_or((x1, y1), |s| (s.1, s.2));
    }
    match samples.len().checked_sub(2).and_then(|i| samples.get(i)) {
        Some(&(t0, x0, y0)) if t1 - t0 >= RESAMPLE_MIN_DELTA => {
            let gap = t1 - t0;
            let ahead = (t - t1).min(MAX_PREDICTION).min(gap / 2.0);
            (
                lerp(x1, x1 + (x1 - x0), ahead / gap),
                lerp(y1, y1 + (y1 - y0), ahead / gap),
            )
        }
        _ => (x1, y1),
    }
}

/// A fling of ours (where the platform has none) carries a gesture on in
/// its direction, slower each frame, and ends; and fingers that stop
/// before they lift fling slower. Returns, for each gesture that ended,
/// how far it was flung.
fn flings(run: &Run) -> Vec<f64> {
    let interval = 1.0 / run.hz;
    let carry = carried(run);
    let mut out: Vec<f64> = Vec::new();
    for (i, w) in run.wheels.iter().enumerate().filter(|(_, w)| w.precise && w.ended) {
        let next = run.wheels.get(i.saturating_add(1)).map_or(f64::INFINITY, |n| n.at);
        // Where the events put it, plus what the flings before carried it.
        let base = truth(run, w.at) + carry.iter().filter(|c| c.0 < w.at).map(|c| c.1).sum::<f64>();
        let after: Vec<&Frame> = run
            .frames
            .iter()
            .filter(|f| f.vblank > w.at && f.vblank < next)
            .collect();
        let end = after.last().map_or(base, |f| f64::from(f.offset));
        let flung = end - base;
        out.push(flung);
        if flung.abs() < 1.0 {
            continue;
        }
        // Slower and slower, after the frame or two that still show the
        // gesture's own prediction.
        let steps: Vec<f64> = after
            .windows(2)
            .map(|p| f64::from(p[1].offset - p[0].offset) / (p[1].present - p[0].present) * interval)
            .collect();
        for s in steps.windows(2).skip(2) {
            assert!(
                s[1].abs() <= s[0].abs() + 0.05 && s[1] * flung >= 0.0,
                "a fling at {} Hz speeds up or turns: steps {steps:?}",
                run.hz
            );
        }
        // It ends: by when its speed falls under 50 px/s, and from then
        // the compositor draws nothing.
        let v0 = steps.get(2).map_or(0.0, |s| s.abs() / interval);
        let lasts = FLING_TAU * (v0 / FLING_MIN).max(1.0).ln() + 0.1;
        // Frames past it only fade the scrollbar: the list stays.
        let moving: Vec<&&Frame> = after
            .windows(2)
            .filter(|p| p.get(1).is_some_and(|f| !f.ui) && p.first().map(|f| f.offset) != p.get(1).map(|f| f.offset))
            .filter_map(|p| p.get(1))
            .collect();
        if let Some(last) = moving.last() {
            assert!(
                last.vblank - w.at <= lasts + 2.0 * interval,
                "a fling at {} Hz from {v0:.0} px/s still moves {:.3} s after the lift (it lasts {lasts:.3} s)",
                run.hz,
                last.vblank - w.at
            );
        }
    }
    out
}

fn logs_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/input_logs")
}

/// Every log under `tests/input_logs`, by file name.
fn logs() -> Vec<(String, InputLog)> {
    let mut out: Vec<(String, InputLog)> = std::fs::read_dir(logs_dir())
        .expect("tests/input_logs")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "log"))
        .map(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(&p).expect("a log");
            let log = InputLog::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
            (name, log)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn log(name: &str) -> InputLog {
    logs()
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, l)| l)
        .unwrap_or_else(|| panic!("no log {name}"))
}

#[test]
fn every_input_log_replays_without_losing_scroll_at_60_120_and_144_hz() {
    let logs = logs();
    assert!(logs.len() >= 6, "the synthesized logs are checked in");
    for (name, log) in &logs {
        for (hz, every) in [(60.0, 1), (120.0, 1), (144.0, 1), (120.0, 4)] {
            let run = replay(log, hz, every);
            let line = check(name, log, &run);
            if !log.header.platform.flings_itself() {
                flings(&run);
            }
            eprintln!("{line}");
        }
    }
}

#[test]
fn a_replay_is_the_same_every_time() {
    let log = log("macos-trackpad-flick.log");
    let a = replay(&log, 120.0, 1);
    let b = replay(&log, 120.0, 1);
    let offsets = |r: &Run| r.frames.iter().map(|f| (f.vblank, f.offset, f.ui)).collect::<Vec<_>>();
    assert_eq!(offsets(&a), offsets(&b));
}

#[test]
fn macos_momentum_is_scrolled_as_it_comes_and_ends_with_its_last_event() {
    let log = log("macos-trackpad-flick.log");
    for hz in [60.0, 120.0] {
        let run = replay(&log, hz, 1);
        let momentum = run.wheels.iter().filter(|w| w.precise && w.hit).count();
        assert!(momentum > 60, "finger and momentum events: {momentum}");
        // No fling of ours: the momentum events are the fling.
        let total: f32 = run.wheels.iter().filter(|w| w.hit).map(|w| w.dy).sum();
        assert!((run.shown_final - START - total).abs() < 0.05);
        let last = run.wheels.iter().filter(|w| w.hit).map(|w| w.at).fold(0.0, f64::max);
        let moved_after = run
            .frames
            .iter()
            .filter(|f| f.vblank > last + PREDICT_FOR + 1.0 / hz)
            .filter(|f| (f.offset - run.shown_final).abs() >= 0.05)
            .count();
        assert_eq!(moved_after, 0, "nothing moves 35 ms after the momentum's last event");
        let faded = last + PREDICT_FOR + f64::from(THUMB_SHOWN + THUMB_FADE) + 1.0 / hz;
        let drawn_after = run.frames.iter().filter(|f| f.vblank > faded).count();
        assert_eq!(drawn_after, 0, "nothing drawn once the scrollbar has faded");
        // A click during the momentum hits what was on screen.
        assert!(!run.presses.is_empty());
    }
}

#[test]
fn a_libinput_gesture_is_flung_and_fingers_that_stopped_fling_slower() {
    let log = log("wayland-touchpad-fling.log");
    for hz in [60.0, 144.0] {
        let run = replay(&log, hz, 1);
        check("wayland-touchpad-fling.log", &log, &run);
        let flung = flings(&run);
        let [moving, paused, rested, ..] = flung.as_slice() else {
            panic!("three gestures that end: {flung:?}");
        };
        assert!(*moving > 200.0, "lifted while moving: flung {moving}");
        assert!(
            *paused > 0.0 && *paused < moving * 0.75,
            "lifted 60 ms after the fingers stopped: slower ({paused} against {moving})"
        );
        assert!(
            rested.abs() < 0.5,
            "lifted 200 ms after they stopped: no fling ({rested})"
        );
    }
}

#[test]
fn a_notched_wheel_applies_at_once() {
    let log = log("windows-wheel.log");
    let run = replay(&log, 60.0, 1);
    for w in run.wheels.iter().filter(|w| w.hit && !w.precise) {
        let expected = truth(&run, w.at);
        let f = run
            .frames
            .iter()
            .find(|f| f.vblank >= w.at)
            .expect("a frame after the notch");
        let later = run.wheels.iter().any(|n| n.at > w.at && n.at <= f.vblank);
        if !later {
            assert!(
                (f64::from(f.offset) - expected).abs() < 0.05,
                "the frame after a notch shows it whole: {} against {expected}",
                f.offset
            );
        }
    }
}

/// A recorded wheel over a wheel handler that declines it: the replay runs
/// the live dispatch, the compositor handing each turn to the UI, whose
/// route scrolls the list as far as the compositor's does with no handler.
#[test]
fn a_recorded_wheel_over_a_declining_handler_scrolls_the_list_as_live() {
    let log = log("windows-wheel.log");
    let direct = replay(&log, 60.0, 1);
    let handed = replay_probe(&log, 60.0, 1, true);
    assert!(
        (direct.ui_final - START).abs() > 1.0,
        "the wheel scrolled: {}",
        direct.ui_final
    );
    assert!(
        (handed.ui_final - direct.ui_final).abs() < 0.05,
        "the UI's route as far: {} against {}",
        handed.ui_final,
        direct.ui_final
    );
}

#[test]
fn a_drag_from_a_1000_hz_mouse_is_one_move_a_frame() {
    let log = log("drag-and-click.log");
    let run = replay(&log, 60.0, 1);
    check("drag-and-click.log", &log, &run);
    let raw: usize = run.raw.iter().map(Vec::len).sum();
    let moves = run
        .drags
        .iter()
        .filter(|d| d.phase == DragPhase::Move && d.present.is_some())
        .count();
    assert!(raw > 3 * moves, "{raw} samples coalesced into {moves} moves");
    assert!(run.presses.iter().filter(|p| p.clicked.is_some()).count() >= 3);
}

/// The synthesized logs, by file name (see [`synthesize`]).
#[test]
fn the_synthesized_logs_are_as_generated() {
    for (name, log) in synthesize() {
        let path = logs_dir().join(&name);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            text == log.write(),
            "{name} differs from what it is synthesized as: run `cargo test -p gui --lib write_synthesized_input_logs -- --ignored`"
        );
    }
}

#[test]
#[ignore = "writes the synthesized logs into tests/input_logs"]
fn write_synthesized_input_logs() {
    for (name, log) in synthesize() {
        std::fs::write(logs_dir().join(&name), log.write()).expect("written");
    }
}

/// A deterministic jitter in `[-1, 1]`.
struct Jitter(u64);

impl Jitter {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let bits = u32::try_from(self.0 >> 40).unwrap_or(0);
        f64::from(bits) / f64::from(1_u32 << 24) * 2.0 - 1.0
    }
}

/// A log under construction: events at milliseconds.
struct Synth {
    header: Header,
    events: Vec<Stamped>,
}

impl Synth {
    fn new(platform: Platform, size: (u32, u32), scale: f32, source: &str) -> Self {
        Self {
            header: Header {
                platform,
                size,
                scale,
                source: source.to_owned(),
            },
            events: Vec::new(),
        }
    }

    fn at(&mut self, ms: f64, event: Event) {
        // Microseconds, as the log keeps them.
        let ms = (ms * 1000.0).round() / 1000.0;
        self.events.push(Stamped { at: ms / 1000.0, event });
    }

    fn wheel(&mut self, ms: f64, delta: WheelDelta, phase: WheelPhase) {
        self.at(ms, Event::Wheel { delta, phase });
    }

    fn pixels(&mut self, ms: f64, dy: f64, phase: WheelPhase) {
        let dy = f64_f32((dy * 100.0).round() / 100.0);
        self.wheel(ms, WheelDelta::Pixels(0.0, dy), phase);
    }

    fn click(&mut self, ms: f64, x: f32, y: f32, hold: f64) {
        self.at(ms, Event::Moved(x, y));
        self.at(ms + 1.0, Event::Primary(true));
        self.at(ms + 1.0 + hold, Event::Primary(false));
    }

    fn done(mut self) -> InputLog {
        self.events.sort_by(|a, b| a.at.total_cmp(&b.at));
        InputLog {
            header: self.header,
            events: self.events,
            observed: Vec::new(),
        }
    }
}

/// A macOS trackpad gesture's finger phase from `start` ms: `deltas` in
/// points (winit's sign), one every `gap` ms with jitter, the first
/// `started` unless the gesture `began` already; returns when the last came.
fn fingers(s: &mut Synth, j: &mut Jitter, start: f64, gap: f64, deltas: &[f64], began: bool) -> f64 {
    let mut t = start;
    for (i, d) in deltas.iter().enumerate() {
        let phase = if i == 0 && !began {
            WheelPhase::Began
        } else {
            WheelPhase::Changed
        };
        s.pixels(t, *d, phase);
        t += gap + j.next() * gap * 0.12;
    }
    t - gap
}

/// The logs synthesized from the documented event sequences of each
/// platform, by file name.
fn synthesize() -> Vec<(String, InputLog)> {
    let mut out = Vec::new();

    // macOS: a flick and its momentum.
    let mut j = Jitter(1);
    let mut s = Synth::new(
        Platform::MacOs,
        (480, 320),
        2.0,
        "synthesized, not recorded: a two-finger flick on a macOS trackpad and its momentum, as AppKit sends it \
         (NSEvent scrollWheel: phase MayBegin, then Began and Changed with precise scrollingDeltaY in points while \
         the fingers move, Ended with no delta at the lift, then momentumPhase Began, Changed and Ended, with \
         phase None), its phases as the window reads them off the NSEvent (gui-display's scroll phases: MayBegin \
         and Began are began, Ended ended, momentumPhase Began, Changed and Ended are momentum-began, momentum and \
         momentum-ended), its delta as winit 0.30 gives it (PixelDelta in logical points). \
         The finger phase is sampled every 8-9 ms (a trackpad's report rate as AppKit coalesces it), the momentum \
         every 16.7 ms decaying by 0.998 a millisecond (UIScrollView.DecelerationRate.normal, the system's \
         documented deceleration) until under half a point; deltas are made up within those rules. A click \
         lands on the list during the momentum.",
    );
    for i in 0..5 {
        s.at(f64::from(i) * 10.0, Event::Moved(150.0, 160.0 + f64_f32(f64::from(i))));
    }
    s.pixels(100.0, 0.0, WheelPhase::Began);
    let ramp = [
        -2.0, -4.0, -7.0, -11.0, -15.0, -19.0, -22.0, -24.0, -25.0, -25.0, -24.0, -22.0, -20.0,
    ];
    let last = fingers(&mut s, &mut j, 112.0, 8.4, &ramp, false);
    s.pixels(last + 7.9, 0.0, WheelPhase::Ended);
    let mut t = last + 8.0 + 8.3;
    let mut d: f64 = -20.0 * 0.98;
    let mut first = true;
    while d.abs() >= 0.5 {
        let phase = if first {
            WheelPhase::MomentumBegan
        } else {
            WheelPhase::Momentum
        };
        s.pixels(t, d, phase);
        first = false;
        let gap = 16.67 + j.next() * 0.4;
        d *= 0.998_f64.powf(gap);
        t += gap;
    }
    s.pixels(t, 0.0, WheelPhase::MomentumEnded);
    s.click(700.0, 150.0, 160.0, 70.0);
    s.click(1500.0, 150.0, 220.0, 40.0);
    out.push(("macos-trackpad-flick.log".to_owned(), s.done()));

    // macOS: a slow scroll with a rest, lifted without momentum, and a touch
    // that never scrolls.
    let mut j = Jitter(2);
    let mut s = Synth::new(
        Platform::MacOs,
        (480, 320),
        2.0,
        "synthesized, not recorded: a slow two-finger scroll on a macOS trackpad (NSEvent phase Began, Changed \
         every 8-12 ms with a few points each), a rest of 200 ms with the fingers down (no events: AppKit sends \
         none while they do not move), more scrolling the other way, and a lift with no momentum (phase Ended, no \
         momentumPhase follows when the fingers had stopped); then fingers resting and lifting without scrolling \
         (phase MayBegin, then Cancelled: began, then cancelled). Phases as the window reads them off the \
         NSEvent (gui-display's scroll phases).",
    );
    s.at(0.0, Event::Moved(200.0, 120.0));
    s.pixels(50.0, 0.0, WheelPhase::Began);
    let slow: Vec<f64> = (0..20).map(|i| -(2.0 + f64::from(i % 4))).collect();
    let last = fingers(&mut s, &mut j, 60.0, 10.0, &slow, false);
    let back: Vec<f64> = (0..15).map(|i| 1.5 + f64::from(i % 3)).collect();
    let last = fingers(&mut s, &mut j, last + 200.0, 11.0, &back, true);
    s.pixels(last + 20.0, 0.0, WheelPhase::Ended);
    s.pixels(last + 400.0, 0.0, WheelPhase::Began);
    s.pixels(last + 520.0, 0.0, WheelPhase::Cancelled);
    s.click(last + 700.0, 200.0, 120.0, 60.0);
    out.push(("macos-trackpad-slow.log".to_owned(), s.done()));

    // macOS: a mouse with a notched wheel.
    let mut s = Synth::new(
        Platform::MacOs,
        (480, 320),
        2.0,
        "synthesized, not recorded: a notched mouse wheel on macOS. AppKit reports it without precise deltas \
         (hasPreciseScrollingDeltas false), scrollingDeltaY in lines, accelerated when the wheel spins fast, \
         phase and momentumPhase None: LineDelta, no phase. Notches 120 ms apart, then \
         a spin 25 ms apart with the acceleration rising and falling.",
    );
    s.at(0.0, Event::Moved(120.0, 100.0));
    for i in 0..4 {
        s.wheel(
            100.0 + f64::from(i) * 120.0,
            WheelDelta::Lines(0.0, -1.0),
            WheelPhase::None,
        );
    }
    for (i, l) in [1.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0].iter().enumerate() {
        s.wheel(
            700.0 + f64_from(i) * 25.0,
            WheelDelta::Lines(0.0, f64_f32(*l)),
            WheelPhase::None,
        );
    }
    s.click(1100.0, 120.0, 100.0, 50.0);
    out.push(("macos-mouse-wheel.log".to_owned(), s.done()));

    // Windows: notched wheel and a precision touchpad with its inertia.
    let mut j = Jitter(4);
    let mut s = Synth::new(
        Platform::Windows,
        (480, 320),
        1.0,
        "synthesized, not recorded: Windows. A notched wheel sends WM_MOUSEWHEEL with a delta of WHEEL_DELTA \
         (120) a notch; a precision touchpad sends WM_MOUSEWHEEL with deltas under 120 while the fingers move, \
         and Windows goes on sending smaller ones after the lift (its inertia), with no message marking either \
         end. winit 0.30 (platform_impl/windows/event_loop.rs WM_MOUSEWHEEL) makes each a LineDelta of \
         delta / 120 with phase moved. Notches 80 ms apart, a fast spin 12 ms apart, then the touchpad: 24-48 \
         units every 8 ms, then inertia every 16 ms decaying by 0.93 until a delta rounds to 0.",
    );
    s.at(0.0, Event::Moved(150.0, 150.0));
    for i in 0..5 {
        s.wheel(
            100.0 + f64::from(i) * 80.0,
            WheelDelta::Lines(0.0, -1.0),
            WheelPhase::None,
        );
    }
    for i in 0..10 {
        s.wheel(
            600.0 + f64::from(i) * 12.0,
            WheelDelta::Lines(0.0, 1.0),
            WheelPhase::None,
        );
    }
    let mut t = 1000.0;
    for units in [-24.0, -32.0, -40.0, -48.0, -48.0, -40.0, -40.0, -32.0] {
        s.wheel(t, WheelDelta::Lines(0.0, f64_f32(units / 120.0)), WheelPhase::None);
        t += 8.0 + j.next();
    }
    let mut units: f64 = -30.0;
    while units.round() != 0.0 {
        s.wheel(
            t,
            WheelDelta::Lines(0.0, f64_f32(units.round() / 120.0)),
            WheelPhase::None,
        );
        units *= 0.93;
        t += 16.0 + j.next() * 0.5;
    }
    s.click(t + 30.0, 150.0, 170.0, 50.0);
    out.push(("windows-wheel.log".to_owned(), s.done()));

    // Wayland: libinput finger scrolls, lifted moving, after a stop, after a rest; and a wheel.
    let mut j = Jitter(5);
    let mut s = Synth::new(
        Platform::Wayland,
        (480, 320),
        1.0,
        "synthesized, not recorded: a touchpad on Wayland through libinput. wl_pointer.axis events with \
         axis_source finger, in surface pixels, one frame every 7-12 ms while the fingers move, and an \
         axis_stop when they lift (no momentum: kinetic scrolling is the client's, as GTK's \
         gtkeventcontrollerscroll.c does it); winit 0.30 (platform_impl/linux/wayland/seat/pointer/mod.rs) makes \
         the first started (began), the rest moved (changed), the stop ended with a zero PixelDelta. Three gestures: lifted while \
         moving, lifted 60 ms after the fingers stopped, and lifted 200 ms after; then a wheel, whose \
         axis_discrete steps winit makes LineDelta.",
    );
    s.at(0.0, Event::Moved(180.0, 150.0));
    let mut gesture = |s: &mut Synth, start: f64, rest: f64| {
        let deltas: Vec<f64> = (0..18)
            .map(|i| -(4.0 + 9.0 * (f64::from(i) / 6.0).min(1.0)) - 0.37 * f64::from(i % 3))
            .collect();
        let mut t = start;
        for (i, d) in deltas.iter().enumerate() {
            s.pixels(t, *d, if i == 0 { WheelPhase::Began } else { WheelPhase::Changed });
            t += 9.0 + j.next() * 1.5;
        }
        let lift = t - 9.0 + 9.0 + rest;
        s.pixels(lift, 0.0, WheelPhase::Ended);
    };
    gesture(&mut s, 100.0, 0.0);
    gesture(&mut s, 3000.0, 60.0);
    gesture(&mut s, 6000.0, 200.0);
    for i in 0..6 {
        s.wheel(
            9000.0 + f64::from(i) * 40.0,
            WheelDelta::Lines(0.0, -1.0),
            WheelPhase::None,
        );
    }
    s.click(9500.0, 180.0, 150.0, 50.0);
    out.push(("wayland-touchpad-fling.log".to_owned(), s.done()));

    // Drags at 125 Hz and 1000 Hz on the pad, clicks on the list, and a
    // click right after a wheel step the UI has not laid out.
    let mut j = Jitter(6);
    let mut s = Synth::new(
        Platform::X11,
        (480, 320),
        1.0,
        "synthesized, not recorded: a mouse. Pointer motion as winit 0.30 reports CursorMoved, one event a \
         device report: every 8 ms for a 125 Hz mouse, every 1 ms (with 0.2 ms jitter) for a 1000 Hz one; \
         buttons as MouseInput at the last position; a notched wheel step (LineDelta) and clicks on the list \
         right after it.",
    );
    s.at(0.0, Event::Moved(430.0, 60.0));
    s.at(20.0, Event::Primary(true));
    let mut t = 28.0;
    for i in 1..=40 {
        s.at(
            t,
            Event::Moved(430.0 + f64_f32(f64::from(i) * 0.3), 60.0 + f64_f32(f64::from(i) * 4.0)),
        );
        t += 8.0 + j.next() * 0.5;
    }
    s.at(t + 3.0, Event::Primary(false));
    s.at(600.0, Event::Moved(440.0, 250.0));
    s.at(610.0, Event::Primary(true));
    let mut t = 611.0;
    for i in 1..=300 {
        let k = f64::from(i);
        s.at(
            t,
            Event::Moved(
                440.0 - f64_f32(k * 0.05),
                250.0 - f64_f32(k * 0.6 + (k / 20.0).sin() * 3.0),
            ),
        );
        t += 1.0 + j.next() * 0.2;
    }
    s.at(t + 0.5, Event::Primary(false));
    s.click(1200.0, 100.0, 30.0, 60.0);
    s.click(1400.0, 100.0, 200.0, 60.0);
    s.at(1600.0, Event::Moved(100.0, 130.0));
    s.wheel(1610.0, WheelDelta::Lines(0.0, -2.0), WheelPhase::None);
    s.click(1611.0, 100.0, 130.0, 5.0);
    s.wheel(1700.0, WheelDelta::Lines(0.0, 1.0), WheelPhase::None);
    s.click(1702.0, 100.0, 130.0, 3.0);
    out.push(("drag-and-click.log".to_owned(), s.done()));
    out
}

fn f64_from(i: usize) -> f64 {
    f64::from(u32::try_from(i).unwrap_or(u32::MAX))
}

/// A line of a recorded log, in its order.
#[derive(Clone, Copy)]
enum Line<'a> {
    Event(&'a Stamped),
    Observed(Observation),
}

/// A recorded log's lines in the one order the render thread wrote them
/// (ADR-0033): its events and what it did between them.
fn lines(log: &InputLog) -> Vec<(Line<'_>, f64)> {
    let mut lines = Vec::with_capacity(log.events.len().saturating_add(log.observed.len()));
    let mut observed = log.observed.iter().peekable();
    for (i, e) in log.events.iter().enumerate() {
        while let Some(o) = observed.next_if(|o| o.after <= i) {
            lines.push((Line::Observed(o.what), o.at));
        }
        lines.push((Line::Event(e), e.at));
    }
    lines.extend(observed.map(|o| (Line::Observed(o.what), o.at)));
    lines
}

/// Replay a recorded `log` through `s` in its sequence (ADR-0033), and
/// settle. The render thread took every line in the log's order, so a
/// replay takes them so too: a wheel is dispatched as it comes, and one
/// the compositor hands on is handled at once with what it scrolled
/// before; other input is handled with what the compositor scrolled
/// since the last hand-off, a held drag's move taking that in as it comes
/// (the live UI's coalescing) and sent before the next input; a grant runs
/// a main frame whose commit the render thread takes at `commit`, or
/// drops at `abort`, wheels between them landing on the compositor first.
///
/// What the live run consumed beyond the events is in the log where it
/// was consumed (Codex r10 3): a frame is drawn where the log says one was
/// (`drawn`), and each input is routed in the frame the log says it was
/// (`seen`), not in whatever the replay drew last; a grant is for the
/// present time the log has, which a held drag is resampled at. A log
/// written by hand, which says none of it, draws as each wheel scrolls and
/// routes in what was drawn last.
fn replay_observed(s: &mut Scenario, log: &InputLog) -> std::collections::HashMap<u64, Shown> {
    // Mail is delivered where the log took it, and nowhere else.
    s.mail_as_logged();
    // The compositor flings as the log's platform does, as live.
    s.parts().1.compositor.set_flings(!log.header.platform.flings_itself());
    // A wheel went to the UI while input was diverted: the fling the
    // compositor handed on stops as input is restored, as live.
    let mut wheeled = false;
    let mut router = Router::default();
    let mut pressed = false;
    // The moves a drag holds (the live UI's own), stamped with the
    // replay's frame each came under.
    let mut held = crate::window::HeldMoves::default();
    let send_move = |s: &mut Scenario, ((x, y), seen): crate::window::HeldMove| {
        let shown = s.parts().1.compositor.timeline().resolve(seen);
        s.pipeline_mut().input_shown(Input::PointerMoved { x, y }, &shown);
    };
    let flush = |s: &mut Scenario, held: &mut crate::window::HeldMoves| {
        if let Some(m) = held.flush() {
            send_move(s, m);
        }
    };
    // The log draws its frames itself.
    let draws = log.observed.iter().any(|o| matches!(o.what, Observation::Drawn(..)));
    // Input went straight to the UI here (the render thread made afresh):
    // the UI scrolls the wheels, as it did live.
    let mut diverted = false;
    // The log began before the live render thread had a commit, and none
    // came since ([`Observation::Uncommitted`]).
    let mut uncommitted = false;
    // The window's decorations and logical size, as the platform told
    // them: what a press on its edge is told by.
    let mut chrome = crate::window::decoration::Chrome::default();
    let mut window = (
        crate::convert::u32_to_f32(log.header.size.0),
        crate::convert::u32_to_f32(log.header.size.1),
    );
    // The latest display time a frame was drawn for.
    let mut drawn_for = 0_u64;
    // The live frames drawn, as the replay drew each.
    let mut frames: std::collections::HashMap<u64, Shown> = std::collections::HashMap::new();
    // The live frames drawn before any commit: empty windows.
    let mut blank: std::collections::HashSet<u64> = std::collections::HashSet::new();
    // The frame the events are routed in, as the log last said.
    let mut routed: Option<Option<u64>> = None;
    let shown_for =
        |s: &Scenario, routed: Option<Option<u64>>, frames: &std::collections::HashMap<u64, Shown>| match routed {
            Some(Some(n)) => frames.get(&n).cloned().unwrap_or_else(|| s.shown()),
            Some(None) => Shown::BeforeFirst,
            None => s.shown(),
        };
    let nanos =
        |seconds: f64| u64::try_from(Duration::from_secs_f64((seconds + T0).max(0.0)).as_nanos()).unwrap_or(u64::MAX);
    for (line, line_at) in lines(log) {
        match line {
            Line::Event(e) => {
                // Each event at its own time, as the live threads stamp it
                // (`at`); the clock never goes back (a frame drawn for a
                // display time after it moved it on).
                let now = nanos(e.at);
                let at = Duration::from_nanos(now);
                if at > Duration::from_nanos(clock::current_nanos()) {
                    clock::advance_to(at);
                }
                match router.route(e.event.clone()) {
                    Routed::Wheel {
                        point,
                        delta,
                        precise,
                        phase,
                    } if diverted => {
                        flush(s, &mut held);
                        wheeled |= phase.stops_a_fling(precise);
                        let at = clock::instant_of(now);
                        let input = Input::diverted_wheel(point, delta, at, precise, phase);
                        let shown = shown_for(s, routed, &frames);
                        s.send_shown(input, shown);
                    }
                    // The live compositor had no scene yet: it found no
                    // container to scroll.
                    Routed::Wheel { .. } if uncommitted => {}
                    Routed::Wheel {
                        point,
                        delta,
                        precise,
                        phase,
                    } => {
                        let shown = shown_for(s, routed, &frames);
                        // A frame drawn live before any commit routes into
                        // nothing (no tables: what its empty ones found).
                        let seen = match routed {
                            Some(Some(n)) if blank.contains(&n) => super::Seen::Frame(u64::MAX),
                            _ => shown.seen(),
                        };
                        let h = s.parts().1;
                        match h.compositor.dispatch_wheel(super::WheelInput {
                            point,
                            delta,
                            time: now,
                            precise,
                            phase,
                            seen,
                        }) {
                            super::Dispatched::Ui(input) => {
                                flush(s, &mut held);
                                s.send_shown(input, shown);
                            }
                            super::Dispatched::Scrolled if !draws => {
                                h.draw(clock::current_nanos());
                                let shown = s.shown();
                                s.pipeline_mut().converge(&shown);
                            }
                            super::Dispatched::Scrolled | super::Dispatched::Nothing => {}
                        }
                    }
                    Routed::Ui(Input::PointerMoved { x, y }) if pressed => {
                        s.take_pull();
                        let seen = shown_for(s, routed, &frames).seen();
                        if let Some(first) = held.push(now, x, y, seen) {
                            send_move(s, first);
                        }
                    }
                    // Taken in order with the input, as the live UI takes it.
                    Routed::Told(told) => {
                        flush(s, &mut held);
                        s.take_pull();
                        match &told {
                            Told::Chrome(c) => chrome = c.clone(),
                            Told::Resized { size, scale } if *scale > 0.0 => {
                                window = (
                                    crate::convert::u32_to_f32(size.0) / scale,
                                    crate::convert::u32_to_f32(size.1) / scale,
                                );
                            }
                            _ => {}
                        }
                        s.told(&told).expect("a size");
                    }
                    // A press on the edge of a window that draws its own
                    // frame resizes the window, and the app never sees it,
                    // by the live UI loop's rule (Codex r12 5b).
                    Routed::Ui(Input::MouseDown { x, y })
                        if crate::window::decoration::edge_at(&chrome, window, x, y).is_some() =>
                    {
                        flush(s, &mut held);
                        s.take_pull();
                    }
                    Routed::Ui(input) => {
                        flush(s, &mut held);
                        match input {
                            Input::MouseDown { .. } => pressed = true,
                            Input::MouseUp { .. } | Input::Blur => pressed = false,
                            _ => {}
                        }
                        let shown = shown_for(s, routed, &frames);
                        s.send_shown(input, shown);
                    }
                }
            }
            Line::Observed(Observation::Seen(n)) => routed = Some(n),
            Line::Observed(Observation::Diverted) => {
                // As the live render thread diverted: its gesture settled
                // into what it hands on, then that and its latch to the UI.
                flush(s, &mut held);
                let before = owners(s);
                let c = &mut s.parts().1.compositor;
                c.settle_gesture(nanos(line_at));
                let gesture = c.hand_gesture();
                s.take_pull();
                if let Some(g) = gesture {
                    s.pipeline_mut().take_gesture(g);
                }
                if !diverted {
                    wheeled = false;
                }
                diverted = true;
                one_owner_across(before, owners(s), "diverted");
            }
            // As live: the UI, past all it took straight, hands its gesture
            // back, and the compositor latches it before the input after;
            // a fling handed on stops if a wheel reached the UI meanwhile.
            Line::Observed(Observation::Restored) => {
                flush(s, &mut held);
                let before = owners(s);
                if std::mem::take(&mut wheeled) {
                    s.parts().1.compositor.stop_fling();
                }
                if let Some(g) = s.pipeline_mut().hand_gesture() {
                    s.parts().1.compositor.take_gesture(g);
                }
                diverted = false;
                one_owner_across(before, owners(s), "restored");
            }
            Line::Observed(Observation::Uncommitted) => uncommitted = true,
            // No frame drew live: the gesture applied, and its fling
            // brought up, to the time the log has, as live.
            Line::Observed(Observation::Settled) => {
                s.parts().1.compositor.settle_gesture(nanos(line_at));
            }
            // The UI took a value from another thread here, in order with
            // the input, as live: what the moves held and the compositor
            // scrolled before it first (Codex r12 7).
            Line::Observed(Observation::Mail(site, nth)) => {
                flush(s, &mut held);
                s.take_pull();
                // Nothing when that mailbox closed before it, as live; the
                // outcome tells a sender that sent the replay less.
                s.mail_logged(crate::mailbox::Address { site, nth }, Duration::from_secs(5));
            }
            // Drawn live before the render thread had any commit: an empty
            // window, which input routed in it finds nothing in, as live
            // (the replay's own compositor already holds the app's first
            // frame).
            Line::Observed(Observation::Drawn(n, present)) if uncommitted => {
                let present = present.map_or_else(clock::current_nanos, nanos);
                drawn_for = drawn_for.max(present);
                let mut empty =
                    super::Presented::of(&crate::scene::Scene::new(log.header.size, log.header.scale, None), 0);
                empty.frame = n;
                empty.present_at = present;
                frames.insert(n, Shown::Frame(std::sync::Arc::new(empty)));
                blank.insert(n);
            }
            Line::Observed(Observation::Drawn(n, present)) => {
                // At the display time the live compositor ticked to (ahead
                // of the clock, as live: the UI's clock runs on meanwhile).
                let present = present.map_or_else(clock::current_nanos, nanos);
                drawn_for = drawn_for.max(present);
                let h = s.parts().1;
                h.draw(present);
                let shown = s.shown();
                s.pipeline_mut().converge(&shown);
                frames.insert(n, shown);
            }
            Line::Observed(Observation::Grant(present)) => {
                // The UI's clock ran on to the grant, and the timers due by
                // then ran first, as live (Codex r12 5a: one due between
                // the last input and the grant went in the frame there).
                let granted = Duration::from_nanos(nanos(line_at));
                if granted > Duration::from_nanos(clock::current_nanos()) {
                    clock::advance_to(granted);
                }
                // The UI samples its held drag for the frame's present
                // time, then runs the frame for it, as live.
                let present = present.map_or_else(clock::current_nanos, nanos);
                if let Some(m) = held.sample(present) {
                    send_move(s, m);
                }
                s.parts().1.hold = true;
                s.frame_for(present).expect("a frame");
            }
            Line::Observed(Observation::Committed) => {
                uncommitted = false;
                let h = s.parts().1;
                // Drawn where the log says it was, if it says.
                if draws {
                    h.release_undrawn();
                } else {
                    h.release();
                }
                h.hold = false;
            }
            Line::Observed(Observation::Aborted) => {
                let h = s.parts().1;
                if let Some((mut commit, _)) = h.held.take() {
                    commit.clear();
                    h.spare = Some(commit);
                }
                h.hold = false;
                s.pipeline_mut().recommit();
            }
        }
        // One side holds a gesture's latch at a time, moving with the
        // route (ADR-0033): none on the compositor while input goes
        // straight to the UI, and a gesture the compositor scrolls a
        // container in is not the UI's too.
        let ui = s.pipeline_mut().holds_gesture();
        let compositor = s.parts().1.compositor.holds_gesture();
        if diverted {
            assert_eq!(compositor, None, "the compositor holds a gesture while the UI routes");
        } else if let Some((g, true)) = compositor {
            assert_ne!(ui, Some(g), "gesture {g} latched on both sides");
        }
    }
    flush(s, &mut held);
    let h = s.parts().1;
    h.release();
    h.hold = false;
    // Past the last display time a frame was drawn for: the timeline goes
    // forward.
    if drawn_for > clock::current_nanos() {
        clock::advance_to(Duration::from_nanos(drawn_for));
    }
    s.settle().expect("a frame");
    frames
}

/// The wheel gesture each side latched, by identity: the UI's, and the
/// compositor's with whether it scrolls a container in it itself (one
/// it routes to the UI is the UI's to own).
type Owners = (Option<u64>, Option<(u64, bool)>);

fn owners(s: &mut Scenario) -> Owners {
    (s.pipeline_mut().holds_gesture(), s.parts().1.compositor.holds_gesture())
}

/// One owner across a transition of the route (ADR-0033): every gesture
/// owned before it is owned after it (none dropped, none appears from
/// nowhere), and has exactly one owner then: the UI, or the compositor
/// scrolling a container in it. A gesture the compositor routes to the
/// UI is not one it owns: the UI decides where its events go, and holds
/// a latch for it only if one hit a container (one over nothing
/// scrollable has none on either side, and loses nothing by it).
fn one_owner_across(before: Owners, after: Owners, what: &str) {
    let held = |o: Owners| {
        let scrolls = o.1.filter(|c| c.1).map(|c| c.0);
        let mut ids: Vec<u64> = [o.0, scrolls].into_iter().flatten().collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let (was, is) = (held(before), held(after));
    assert_eq!(
        was, is,
        "{what}: the gestures latched before {before:?} and after {after:?} differ"
    );
    for g in is {
        let sides = usize::from(after.0 == Some(g)).saturating_add(usize::from(after.1 == Some((g, true))));
        assert_eq!(sides, 1, "{what}: gesture {g} has {sides} owners after it ({after:?})");
    }
}

/// A "Top" button over a list, the list scrolled by the wheel: the
/// scenario a recorded log is replayed through, and the list's handle.
fn top_and_list() -> (Scenario, ScrollHandle) {
    let h = ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(300.0)
                .child(
                    div()
                        .height(40.0)
                        .on_click(move |_| h.scroll_to((0.0, 0.0)))
                        .child(crate::element::text("Top")),
                )
                .child(
                    div()
                        .height(200.0)
                        .overflow_y(Overflow::Scroll)
                        .track_scroll(&h)
                        .child(div().height(5_000.0)),
                )
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    (s, h)
}

/// Codex r8 3, under ADR-0033: a wheel turned over the list, then "Top"
/// clicked, a main frame granted and committed after. The render thread
/// took the wheel before it handed the click on (one producer), so the
/// click's jump comes after the wheel's motion and swallows it: 0, live
/// and replayed alike. The order where the wheel's motion lands after the
/// click (ADR-0032's residual race) cannot be written: the log is in the
/// render thread's order. The log reads back as written.
#[test]
fn a_replay_takes_in_what_the_live_threads_took_in() {
    let text = "gui-input-log 2\nplatform other\nwindow 300 300 1\nsource a test\n\
                0.000 move 150 100\n\
                10.000 wheel lines 0 -1 none\n\
                11.000 move 150 20\n\
                12.000 down\n\
                13.000 up\n\
                16.000 obs grant\n16.300 obs commit\n";
    let log = InputLog::parse(text).expect("a log");
    assert_eq!(InputLog::parse(&log.write()).as_ref(), Ok(&log), "read back");
    let (mut s, h) = top_and_list();
    replay_observed(&mut s, &log);
    assert_eq!(h.offset(), (0.0, 0.0));
}

/// A wheel the render thread takes while the UI builds the frame it was
/// granted lands on the compositor before that frame's commit, and is
/// handed the UI with the next input, not lost to the commit: the list
/// ends a wheel's step down, as the one wheel left it.
#[test]
fn a_wheel_between_a_grant_and_its_commit_is_kept() {
    let text = "gui-input-log 2\nplatform other\nwindow 300 300 1\n\
                0 move 150 100\n\
                1 obs grant\n\
                2 wheel lines 0 -1 none\n\
                3 obs commit\n\
                4 move 150 101\n";
    let (mut s, h) = top_and_list();
    replay_observed(&mut s, &InputLog::parse(text).expect("a log"));
    assert!(h.offset().1 > 0.0, "{:?}", h.offset());
}

/// Replay `text` through [`top_and_list`]: where the list ends.
fn replayed(text: &str) -> f32 {
    let log = InputLog::parse(text).expect("a log");
    let (mut s, h) = top_and_list();
    replay_observed(&mut s, &log);
    h.offset().1
}

/// Codex r9 I3: a wheel over the list, then "Top" pressed, the pointer
/// moved twice while held (the live window holds them and sends one, the
/// last, before the release) and released. Live, the click jumped to Top
/// after the wheel was taken: 0. The replay holds the moves as the UI
/// does, each taking in what was scrolled as it comes, and handles the
/// release after the last.
#[test]
fn top_release_after_two_coalesced_moves_is_replayed() {
    let log = "gui-input-log 2\nplatform other\nwindow 300 300 1\n\
               0 move 150 100\n\
               1 wheel lines 0 -1 none\n\
               2 move 150 20\n\
               3 down\n\
               4 move 149 20\n5 move 150 20\n\
               6 up\n\
               7 obs grant\n7.3 obs commit\n";
    assert_eq!(replayed(log), 0.0);
}

/// Opus r9 I1: as above with three held moves: the click is not lost.
#[test]
fn a_click_after_three_coalesced_drag_moves_is_replayed() {
    let log = "gui-input-log 2\nplatform other\nwindow 300 300 1\n\
               10 move 150 100\n\
               11 wheel lines 0 -1 none\n\
               12 move 150 20\n\
               12.1 down\n\
               12.2 move 150 21\n12.3 move 150 22\n12.4 move 150 20\n\
               12.5 up\n\
               13 obs grant\n13.3 obs commit\n";
    assert_eq!(replayed(log), 0.0);
}

/// Codex r10 3a: a list of 20 px rows, a wheel turned 40 px over it and a
/// click right after, before any frame showing the scroll was drawn. Live,
/// the click was routed in the frame on screen (the log's `seen 5`), where
/// row 0 is under it. The replay draws only where the log drew, and routes
/// the click in the frame the log names: row 0, not the row a frame drawn
/// at the wheel would put there (2).
#[test]
fn a_click_before_the_scroll_is_drawn_is_routed_in_the_frame_on_screen() {
    let row = Rc::new(std::cell::Cell::new(usize::MAX));
    let r = Rc::clone(&row);
    let mut s = Scenario::measuring(
        move || {
            let r = Rc::clone(&r);
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .children((0..20).map(move |i| {
                    let r = Rc::clone(&r);
                    div().height(20.0).on_click(move |_| r.set(i))
                }))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 obs drawn 5\n0 obs seen 5\n\
               1 move 50 10\n\
               2 wheel lines 0 -1 none\n\
               3 down\n4 up\n";
    replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    assert_eq!(row.get(), 0, "the row frame 5 showed under the click");
}

/// Codex r10 3b: two moves of a drag, at 10 ms (x 0) and 20 ms (x 100),
/// then a main frame granted for 21 ms. Live, the UI resampled the drag
/// Codex r11 3: the window widened from 100 to 200 px before a main frame,
/// then a click at x 75: on the child that takes the left half, laid out
/// at the new width. The log has the resize where the UI took it, and the
/// replay lays the frame out at that size, so the click lands where it
/// did live.
#[test]
fn a_click_after_a_resize_is_replayed_at_the_new_size() {
    let hit = Rc::new(std::cell::Cell::new(false));
    let h = Rc::clone(&hit);
    let mut s = Scenario::measuring(
        move || {
            let h = Rc::clone(&h);
            div()
                .width(pct(100.0))
                .height(100.0)
                .child(div().width(pct(50.0)).height(100.0).on_click(move |_| h.set(true)))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 resize 200 100 1\n\
               1 obs grant 16\n2 obs commit\n3 obs drawn 1 16\n\
               17 obs seen 1\n17 move 75 40\n18 down\n19 up\n";
    replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    assert!(hit.get(), "the click lands on the child at the new width");
}

/// A list 100 px tall over 5000 px, the compositor flinging a trackpad's
/// lift (as where the platform has no momentum).
fn a_flinging_list() -> Scenario {
    let mut s = Scenario::measuring(
        || {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(5_000.0))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    s.compositor().set_flings(true);
    s
}

/// Codex r11 2: a trackpad's gesture lifted at 20 ms, then a compositor
/// frame drawn for its display at 40 ms: the fling has carried the list on
/// for 20 ms by then. The log has that frame's display time, and the
/// replay draws the frame at it, so the frame it keeps (and routes input
/// in) shows the list where the live one did, not where the events left
/// it.
#[test]
fn a_frame_is_replayed_at_its_display_time() {
    let at = |ms: f64| u64::try_from(Duration::from_secs_f64(ms / 1000.0 + T0).as_nanos()).unwrap_or(0);
    let live = {
        let mut s = a_flinging_list();
        let seen = s.shown().seen();
        let c = s.compositor();
        for (ms, phase, dy) in [
            (0.0, WheelPhase::Began, 20.0),
            (10.0, WheelPhase::Changed, 20.0),
            (20.0, WheelPhase::Ended, 0.0),
        ] {
            c.dispatch_wheel(super::WheelInput {
                point: (50.0, 50.0),
                delta: (0.0, dy),
                time: at(ms),
                precise: true,
                phase,
                seen,
            });
        }
        c.tick(at(40.0));
        c.compose(at(40.0));
        c.presented().scroll[0].offset.1
    };
    let mut s = a_flinging_list();
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 move 50 50\n\
               0 wheel pixels 0 -20 began\n10 wheel pixels 0 -20 changed\n20 wheel pixels 0 0 ended\n\
               21 obs drawn 1 40\n";
    let frames = replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    let replayed = match frames.get(&1) {
        Some(Shown::Frame(p)) => p.scroll.first().map(|s| s.offset.1),
        _ => None,
    };
    assert!(live > 40.0, "flung on past the events' 40 px: {live}");
    assert_eq!(replayed, Some(live), "the frame drawn for 40 ms");
}

/// for that present (5 ms before it: x 60). The log has the grant's
/// present time, and the replay samples there too.
#[test]
fn a_held_drag_is_resampled_at_the_grants_present_time() {
    let xs: Rc<RefCell<Vec<f32>>> = Rc::default();
    let d = Rc::clone(&xs);
    let mut s = Scenario::measuring(
        move || {
            let d = Rc::clone(&d);
            div()
                .width(300.0)
                .height(100.0)
                .on_drag(move |e| {
                    if e.phase != DragPhase::End {
                        d.borrow_mut().push(e.x);
                    }
                })
                .into_element()
        },
        300,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let log = "gui-input-log 2\nplatform other\nwindow 300 100 1\n\
               0 move 150 50\n1 down\n\
               10 move 0 50\n20 move 100 50\n\
               20.5 obs grant 21\n20.9 obs commit\n";
    replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    // The drag starts where it was pressed; its first move is the frame's.
    let first = xs.borrow().get(1).copied();
    assert!(first.is_some_and(|x| (x - 60.0).abs() < 0.01), "{:?}", xs.borrow());
}

/// Found by the property test below: wheels the platform sends before
/// the UI's first commit (here a first build that takes 300 ms) are taken
/// by a compositor with no scene, which routes them in nothing, as input
/// before the first frame is routed in what was committed (none): the
/// live list stays at 0. Its replay began from a committed scenario and
/// scrolled them (120). The log says it began uncommitted, and the replay
/// routes no wheel until the first commit: live and replayed alike.
#[test]
fn wheels_before_the_first_commit_replay_as_live() {
    use std::time::Instant;

    let page = move || {
        std::thread::sleep(Duration::from_millis(300));
        move || {
            div()
                .width(300.0)
                .height(200.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(5_000.0))
                .into_element()
        }
    };
    let Ok(mut w) = crate::app::offscreen(
        page,
        crate::app::RunOptions::default(),
        (300, 300),
        1.0,
        Duration::from_millis(8),
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    let path = std::env::temp_dir().join(format!("gui-uncommitted-{}.txt", std::process::id()));
    if w.record_input(&path, (300, 300), 1.0).is_err() {
        eprintln!("no temp file: skipped");
        return;
    }
    w.send(Event::Moved(100.0, 100.0));
    for _ in 0..3 {
        w.send(Event::Wheel {
            delta: WheelDelta::Lines(0.0, -1.0),
            phase: WheelPhase::None,
        });
    }
    // Then one after the first frame shows the list.
    let until = Instant::now() + Duration::from_secs(3);
    while w.presented().scroll.is_empty() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    w.send(Event::Wheel {
        delta: WheelDelta::Lines(0.0, -1.0),
        phase: WheelPhase::None,
    });
    std::thread::sleep(Duration::from_millis(300));
    let live = w.presented().scroll.first().map(|s| s.offset.1);
    drop(w);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::remove_file(&path).ok();
    let log = InputLog::parse(&text).expect("the live log reads back");
    let mut s = Scenario::measuring(
        || {
            div()
                .width(300.0)
                .height(200.0)
                .overflow_y(Overflow::Scroll)
                .child(div().height(5_000.0))
                .into_element()
        },
        300,
        300,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    replay_observed(&mut s, &log);
    let replayed = match s.shown() {
        Shown::Frame(p) => p.scroll.first().map(|s| s.offset.1),
        Shown::BeforeFirst | Shown::Gone(_) => None,
    };
    assert_eq!(
        live,
        Some(crate::interact::LINE_SCROLL),
        "the one wheel after the commit: {text}"
    );
    assert_eq!(replayed, live, "{text}");
}

/// A list 100 tall over 5000 whose click scrolls it to 100 after 50 ms
/// (a timer), and its handle.
fn a_list_a_timer_scrolls() -> (Scenario, ScrollHandle) {
    let h = ScrollHandle::new();
    let mut s = Scenario::measuring(
        move || {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .on_click(move |_| {
                    clock::after(Duration::from_millis(50), move || h.scroll_to((0.0, 100.0)));
                })
                .child(div().height(5_000.0))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    (s, h)
}

/// Codex r12 5a: a timer due between the last input and a grant ran on
/// the live UI before the frame it was granted (it asked for that frame):
/// the replay runs its clock on to the grant's time first, so the frame
/// drawn there shows what the timer did.
#[test]
fn a_timer_due_before_a_grant_is_in_its_frame() {
    let (mut s, _h) = a_list_a_timer_scrolls();
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 move 50 50\n1 down\n2 up\n\
               60 obs grant 76\n61 obs commit\n70 obs drawn 1 76\n";
    let frames = replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    let drawn = match frames.get(&1) {
        Some(Shown::Frame(p)) => p.scroll.first().map(|s| s.offset.1),
        _ => None,
    };
    assert_eq!(drawn, Some(100.0), "the frame granted after the timer shows it");
}

/// A window-sized box counting its clicks.
fn a_clickable_window() -> (Scenario, Rc<std::cell::Cell<u32>>) {
    let clicks = Rc::new(std::cell::Cell::new(0_u32));
    let c = Rc::clone(&clicks);
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            div()
                .width(pct(100.0))
                .height(pct(100.0))
                .on_click(move |_| c.set(c.get().saturating_add(1)))
                .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    (s, clicks)
}

/// Codex r12 5b: a window that draws its own frame, resizable, takes a
/// press on its edge to resize itself, before the app sees it (live,
/// `ui_loop`). The replay takes it so too, by the same rule
/// ([`crate::window::decoration::edge_at`]), so the app's handler hears
/// only the press inside: one click, live and replayed.
#[test]
fn a_press_on_a_resize_edge_is_the_windows_in_a_replay_too() {
    let (mut s, clicks) = a_clickable_window();
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 chrome client+active+resizable \"t\"\n\
               1 move 1 50\n2 down\n3 up\n\
               4 move 50 50\n5 down\n6 up\n";
    replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    assert_eq!(
        clicks.get(),
        1,
        "the edge's press resized the window; the one inside clicked"
    );
}

/// Codex r12 5c: a window resized to nothing (minimized on some
/// platforms) and back, as live takes it: the replay goes on.
#[test]
fn a_window_resized_to_nothing_and_back_replays() {
    let (mut s, clicks) = a_clickable_window();
    let log = "gui-input-log 2\nplatform other\nwindow 100 100 1\n\
               0 resize 0 0 1\n1 resize 100 100 1\n\
               2 move 50 50\n3 down\n4 up\n";
    replay_observed(&mut s, &InputLog::parse(log).expect("a log"));
    assert_eq!(clicks.get(), 1, "the click after it lands");
}

/// A page of a focusable button that hears keys, a drag pad and a list
/// 200 tall over 5000, each telling `calls` what it heard.
fn a_mixed_page(calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> impl Fn() -> crate::Element + Clone {
    move || {
        let (click, key, drag) = (calls.clone(), calls.clone(), calls.clone());
        let note = |c: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, s: String| {
            c.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(s);
        };
        div()
            .width(300.0)
            .child(
                div()
                    .display(Display::Flex)
                    .height(40.0)
                    .child(
                        div()
                            .width(150.0)
                            .height(40.0)
                            .focusable()
                            .on_click(move |_| note(&click, "click".to_owned()))
                            .on_key_down(move |k| {
                                note(&key, format!("key {:?} {:?}", k.key, k.text));
                                true
                            }),
                    )
                    .child(div().width(150.0).height(40.0).on_drag(move |d| {
                        note(&drag, format!("drag {:?} {} {}", d.phase, d.x, d.y));
                    })),
            )
            .child(
                div()
                    .height(200.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(5_000.0)),
            )
            .into_element()
    }
}

/// r11 B, the log's property: a live run on the app's threads, recorded,
/// of wheels, a trackpad's fling, a click, keys (a named key no short
/// list holds among them), a drag, a resize and a render thread that
/// panics with a wheel after it, replays to what the live run showed:
/// every frame the live run was seen to show has the list where the
/// replay's frame of that number has it, and every handler heard the
/// same, in the same order.
#[test]
fn a_recorded_live_run_replays_to_the_same_frames_and_handler_calls() {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Instant;

    use winit::keyboard::{Key, NamedKey};

    use super::input_log::Told;

    let live_calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let page = a_mixed_page(Arc::clone(&live_calls));
    let interval = Duration::from_millis(8);
    let Ok(mut w) = crate::app::offscreen(
        move || page,
        crate::app::RunOptions::default(),
        (300, 300),
        1.0,
        interval,
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    let path = std::env::temp_dir().join(format!("gui-mixed-run-{}.txt", std::process::id()));
    if w.record_input(&path, (300, 300), 1.0).is_err() {
        eprintln!("no temp file: skipped");
        return;
    }
    // Each frame seen on screen: its number and where the list showed.
    let mut seen: std::collections::BTreeMap<u64, f32> = std::collections::BTreeMap::new();
    let mut wait = |w: &crate::window::Offscreen, ms: u64| {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            let p = w.presented();
            if let Some(s) = p.scroll.first() {
                seen.insert(p.frame, s.offset.1);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let pixels = |dy: f32, phase: WheelPhase| Event::Wheel {
        delta: WheelDelta::Pixels(0.0, dy),
        phase,
    };
    let key = |key: Key, text: Option<&str>| Event::Key {
        key,
        text: text.map(str::to_owned),
        repeat: false,
    };
    wait(&w, 200);
    // Notched wheels over the list.
    w.send(Event::Moved(100.0, 150.0));
    for _ in 0..3 {
        w.send(Event::Wheel {
            delta: WheelDelta::Lines(0.0, -1.0),
            phase: WheelPhase::None,
        });
        wait(&w, 12);
    }
    // A trackpad's gesture, lifted fast: the compositor flings.
    w.send(pixels(-20.0, WheelPhase::Began));
    for _ in 0..4 {
        wait(&w, 8);
        w.send(pixels(-30.0, WheelPhase::Changed));
    }
    w.send(pixels(0.0, WheelPhase::Ended));
    wait(&w, 150);
    // A click on the button (it takes the focus), then keys.
    w.send(Event::Moved(50.0, 20.0));
    w.send(Event::Primary(true));
    w.send(Event::Primary(false));
    wait(&w, 30);
    w.send(key(Key::Character("a".into()), Some("a")));
    w.send(key(Key::Named(NamedKey::Redo), None));
    w.send(key(Key::Named(NamedKey::ArrowDown), None));
    wait(&w, 30);
    // A drag on the pad.
    w.send(Event::Moved(200.0, 20.0));
    w.send(Event::Primary(true));
    for x in [210.0, 220.0, 230.0] {
        wait(&w, 10);
        w.send(Event::Moved(x, 25.0));
    }
    w.send(Event::Primary(false));
    wait(&w, 30);
    // The window grows.
    w.send(Event::Told(Told::Resized {
        size: (300, 400),
        scale: 1.0,
    }));
    wait(&w, 60);
    // The render thread panics; a wheel straight after goes to the UI.
    w.panic_render();
    w.send(Event::Moved(100.0, 150.0));
    w.send(Event::Wheel {
        delta: WheelDelta::Lines(0.0, -2.0),
        phase: WheelPhase::None,
    });
    wait(&w, 400);
    w.send(Event::Wheel {
        delta: WheelDelta::Lines(0.0, 1.0),
        phase: WheelPhase::None,
    });
    wait(&w, 400);
    drop(w);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::remove_file(&path).ok();
    let log = InputLog::parse(&text).expect("the live log reads back");
    assert!(
        log.observed.iter().any(|o| matches!(o.what, Observation::Diverted)),
        "the panic diverted: {text}"
    );
    let live_calls = live_calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(live_calls.iter().any(|c| c.contains("Redo")), "{live_calls:?}");

    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut s = Scenario::measuring(a_mixed_page(Arc::clone(&calls)), 300, 300, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    let frames = replay_observed(&mut s, &log);
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(calls, live_calls, "the handlers heard the same");
    let mut compared = 0;
    let mut differ = Vec::new();
    for (n, live) in &seen {
        let Some(Shown::Frame(p)) = frames.get(n) else {
            continue;
        };
        let replayed = p.scroll.first().map(|s| s.offset.1);
        if replayed != Some(*live) {
            differ.push((*n, *live, replayed));
        }
        compared += 1;
    }
    assert!(differ.is_empty(), "frames (live, replayed) differ: {differ:?}\n{text}");
    eprintln!(
        "{compared} frames compared of {} seen; {} handler calls: {live_calls:?}",
        seen.len(),
        live_calls.len()
    );
    assert!(compared > 10, "{compared} frames compared of {} seen", seen.len());
}

/// Who a page's mailbox is: its sender, as the page made it.
type MailSlot = std::sync::Arc<std::sync::Mutex<Option<crate::mailbox::Sender<u32>>>>;

/// A list over a clickable bar, and a mailbox another thread sends
/// numbers to: each scrolls the list to seven times it, and every handler
/// notes what it heard with where the list was then, so the order mail
/// and input were taken in shows.
///
/// The handle and the mailbox are the page's state, made once on its UI
/// thread by the setup, not by the render: a render that made them would
/// make them anew each time it ran, and a list scrolled before would show
/// at 0 (a live re-render does so too; `GUI_COLD_CHECK` caught it).
fn a_mail_page(
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    slot: MailSlot,
) -> impl FnOnce() -> Box<dyn Fn() -> crate::Element> + Send + 'static {
    fn note(c: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, s: String) {
        c.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(s);
    }
    move || {
        let h = ScrollHandle::new();
        let mail = calls.clone();
        let tx = crate::mailbox(move |n: u32| {
            note(&mail, format!("mail {n} at {:?}", h.offset()));
            h.scroll_to((0.0, 7.0 * crate::convert::u32_to_f32(n)));
        });
        *slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
        Box::new(move || {
            let click = calls.clone();
            div()
                .width(300.0)
                .child(
                    div()
                        .height(40.0)
                        .on_click(move |_| note(&click, format!("click at {:?}", h.offset()))),
                )
                .child(
                    div()
                        .height(200.0)
                        .overflow_y(Overflow::Scroll)
                        .track_scroll(&h)
                        .child(div().height(5_000.0)),
                )
                .into_element()
        })
    }
}

/// Codex r12 7: mail another thread sends reaches the UI through the one
/// producer, logged where the UI takes it in order with the input, on
/// both routes (through the render thread, and straight while it is made
/// afresh after a panic); a replay delivers each value at its log place,
/// so the handlers hear mail and input in the live order, and the frames
/// show the list where live did.
#[test]
fn mail_from_another_thread_replays_in_its_place_among_the_input() {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Instant;

    let live_calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let live_slot: MailSlot = Arc::default();
    let page = a_mail_page(Arc::clone(&live_calls), Arc::clone(&live_slot));
    let Ok(mut w) = crate::app::offscreen(
        page,
        crate::app::RunOptions::default(),
        (300, 300),
        1.0,
        Duration::from_millis(8),
    ) else {
        eprintln!("no GPU: skipped");
        return;
    };
    let path = std::env::temp_dir().join(format!("gui-mail-run-{}.txt", std::process::id()));
    if w.record_input(&path, (300, 300), 1.0).is_err() {
        eprintln!("no temp file: skipped");
        return;
    }
    let mut seen: std::collections::BTreeMap<u64, f32> = std::collections::BTreeMap::new();
    let mut wait = |w: &crate::window::Offscreen, ms: u64| {
        let until = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < until {
            let p = w.presented();
            if let Some(s) = p.scroll.first() {
                seen.insert(p.frame, s.offset.1);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    wait(&w, 200);
    let made = Instant::now() + Duration::from_secs(10);
    while live_slot.lock().unwrap_or_else(PoisonError::into_inner).is_none() && Instant::now() < made {
        wait(&w, 10);
    }
    let sender = live_slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("the page made its mailbox");
    // Another thread mails while the platform's input comes.
    let mailer = std::thread::spawn(move || {
        for n in 1..=24_u32 {
            sender.send(n).ok();
            std::thread::sleep(Duration::from_millis(3));
        }
    });
    for round in 0..8 {
        w.send(Event::Moved(100.0, 150.0));
        w.send(Event::Wheel {
            delta: WheelDelta::Lines(0.0, -1.0),
            phase: WheelPhase::None,
        });
        w.send(Event::Moved(50.0, 20.0));
        w.send(Event::Primary(true));
        w.send(Event::Primary(false));
        // The render thread made afresh: input and mail go straight to
        // the UI a while.
        if round == 3 {
            w.panic_render();
        }
        wait(&w, 9);
    }
    mailer.join().expect("the mailer");
    wait(&w, 400);
    drop(w);
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    std::fs::remove_file(&path).ok();
    let log = InputLog::parse(&text).expect("the live log reads back");
    let mails = log
        .observed
        .iter()
        .filter(|o| matches!(o.what, Observation::Mail(..)))
        .count();
    assert_eq!(mails, 24, "each value logged where it was taken: {text}");
    let live_calls = live_calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        live_calls.iter().filter(|c| c.starts_with("mail")).count(),
        24,
        "{live_calls:?}"
    );

    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let slot: MailSlot = Arc::default();
    // The page's setup, run here at the test's top level (no window's code
    // runs: ADR-0015, the thread-window rule), in a scope of the test's.
    let page = crate::engine::owned(a_mail_page(Arc::clone(&calls), Arc::clone(&slot)));
    let mut s = Scenario::measuring(page, 300, 300, 1.0).expect("a scenario");
    s.settle().expect("a frame");
    // The outside world sends the replay what it sent live, on its own
    // thread and time; the log says where each is taken.
    let sender = slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .expect("the page made its mailbox");
    let mailer = std::thread::spawn(move || {
        for n in 1..=24_u32 {
            sender.send(n).ok();
        }
    });
    let frames = replay_observed(&mut s, &log);
    mailer.join().expect("the mailer");
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(calls, live_calls, "mail and input heard in the live order\n{text}");
    let mut differ = Vec::new();
    for (n, live) in &seen {
        let Some(Shown::Frame(p)) = frames.get(n) else {
            continue;
        };
        let replayed = p.scroll.first().map(|s| s.offset.1);
        if replayed != Some(*live) {
            differ.push((*n, *live, replayed));
        }
    }
    assert!(differ.is_empty(), "frames (live, replayed) differ: {differ:?}\n{text}");
}

/// The fault matrix's page: a bar over an outer list holding a pad that
/// takes drags, an inner list beside a strip only the outer list is under,
/// and a box whose wheel handler takes every wheel. Each handler notes
/// what it heard; a click on the bar notes where both lists are.
fn a_faulted_page(
    calls: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    mail: std::sync::Arc<()>,
) -> impl FnOnce() -> Box<dyn Fn() -> crate::Element> + Send + 'static {
    move || {
        let (inner, outer, fine) = (ScrollHandle::new(), ScrollHandle::new(), ScrollHandle::new());
        // Each click builds the page again, and makes a scope of its own,
        // disposing the one before: the mailbox that one made is let go
        // of, its handler (holding `mail`) with it, at once, with no
        // delivery to it (decision 5; `mail_kept` counts what is not).
        // The click does it, not the build: a build changes app heap only
        // through signals (ADR-0034, the render-purity contract), as a
        // cold check builds it again in a world of its own, which a scope
        // kept here and swapped by the build would leak out of.
        let clicks = crate::reactive::Signal::new(0_u32);
        let scope: std::rc::Rc<std::cell::RefCell<Option<crate::Scope>>> = std::rc::Rc::default();
        Box::new(move || {
            let note = |c: &std::sync::Arc<std::sync::Mutex<Vec<String>>>, s: String| {
                c.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(s);
            };
            let built = clicks.get();
            // Weak: only a mailbox's handler holds `mail`, not the builds.
            let (scope, mail) = (std::rc::Rc::clone(&scope), std::sync::Arc::downgrade(&mail));
            let (click, drag, wheel) = (calls.clone(), calls.clone(), calls.clone());
            div()
                .width(300.0)
                .child(div().height(20.0).on_click(move |_| {
                    note(
                        &click,
                        format!(
                            "click inner {:?} outer {:?} fine {:?}",
                            inner.offset(),
                            outer.offset(),
                            fine.offset()
                        ),
                    );
                    if let Some(before) = scope.borrow_mut().take() {
                        before.dispose();
                    }
                    let now = crate::Scope::new();
                    let held = mail.upgrade();
                    let sender = now.run(|| {
                        crate::mailbox::mailbox(move |(): ()| {
                            std::hint::black_box(&held);
                        })
                    });
                    *scope.borrow_mut() = Some(now);
                    drop(sender);
                    clicks.set(built.saturating_add(1));
                }))
                // Steps finer than motion's unit, whose exact sum the
                // matrix knows (decision 4, [`FINE_STEP`]).
                .child(
                    div()
                        .height(20.0)
                        .overflow_y(Overflow::Scroll)
                        .track_scroll(&fine)
                        .child(div().height(400.0)),
                )
                .child(
                    div()
                        .height(260.0)
                        .overflow_y(Overflow::Scroll)
                        .track_scroll(&outer)
                        .child(div().height(40.0).on_drag(move |d| {
                            if d.phase != DragPhase::Move {
                                note(&drag, format!("drag {:?} {} {}", d.phase, d.x, d.y));
                            }
                        }))
                        .child(
                            div()
                                .display(Display::Flex)
                                .height(150.0)
                                .child(
                                    div()
                                        .width(200.0)
                                        .height(150.0)
                                        .overflow_y(Overflow::Scroll)
                                        .track_scroll(&inner)
                                        .child(div().height(4_000.0)),
                                )
                                .child(div().width(100.0).height(150.0)),
                        )
                        .child(div().height(60.0).on_wheel(move |w| {
                            note(&wheel, format!("wheel {} {}", w.dx, w.dy));
                            true
                        }))
                        .child(div().height(2_000.0)),
                )
                .into_element()
        })
    }
}

/// What a fault plan does before an event: lose the device, lose it with
/// none answering a while, panic the render thread, or have it panic when
/// it next diverts, while its device is made on the helper thread, or as
/// it returns the UI's latch (between that and the restore).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fault {
    Lose,
    Hold,
    Panic,
    At(super::thread::PanicStage),
}

/// One live run of the fault matrix's script on the app's threads,
/// recorded: `faults` injected before the events so numbered.
struct Faulted {
    calls: Vec<String>,
    log: InputLog,
    text: String,
    /// Each frame seen on screen: its number and where its lists showed.
    seen: std::collections::BTreeMap<u64, Vec<(f32, f32)>>,
    /// The hops of the input handed on while recovering, each from the
    /// platform's send to the render thread handing it on (ADR-0033).
    hops: Vec<Duration>,
    /// How long after its stamp the UI took each input (the step the
    /// script's input was in when a fault came: recovering).
    takes: Vec<Duration>,
    /// How long after its stamp each event of those steps was sent: this
    /// thread's own wake, before the hop.
    late_sends: Vec<Duration>,
    /// How many mailboxes of the page's builds before the last still held
    /// their handlers at the end (decision 5: none).
    mail_kept: usize,
}

/// The fault matrix's script: each step's events, a step beginning where
/// the screen shows everything before it (a gesture begun, a click, a
/// drag, which land where the frame on screen shows things).
fn fault_script() -> Vec<Vec<Event>> {
    let lines = |dy: f32| Event::Wheel {
        delta: WheelDelta::Lines(0.0, dy),
        phase: WheelPhase::None,
    };
    let pixels = |dy: f32, phase: WheelPhase| Event::Wheel {
        delta: WheelDelta::Pixels(0.0, dy),
        phase,
    };
    vec![
        // Notched wheels over the inner list.
        vec![Event::Moved(100.0, 150.0), lines(-1.0), lines(-1.0)],
        // A trackpad's gesture there, and the platform's momentum after.
        vec![
            pixels(-10.0, WheelPhase::Began),
            pixels(-20.0, WheelPhase::Changed),
            pixels(-20.0, WheelPhase::Changed),
            pixels(-15.0, WheelPhase::Changed),
            pixels(0.0, WheelPhase::Ended),
            pixels(-8.0, WheelPhase::MomentumBegan),
            pixels(-6.0, WheelPhase::Momentum),
            pixels(-4.0, WheelPhase::Momentum),
            pixels(0.0, WheelPhase::MomentumEnded),
        ],
        // Over the wheel handler: a notched wheel and a short gesture.
        vec![
            Event::Moved(100.0, 260.0),
            lines(-1.0),
            pixels(-5.0, WheelPhase::Began),
            pixels(-5.0, WheelPhase::Changed),
            pixels(0.0, WheelPhase::Ended),
        ],
        // A click on the bar.
        vec![Event::Moved(50.0, 10.0), Event::Primary(true), Event::Primary(false)],
        // A drag on the pad.
        vec![
            Event::Moved(50.0, 60.0),
            Event::Primary(true),
            Event::Moved(70.0, 60.0),
            Event::Moved(90.0, 62.0),
            Event::Primary(false),
        ],
        // Off the inner list onto the strip only the outer is under, and
        // a gesture there.
        vec![
            Event::Moved(250.0, 150.0),
            pixels(-12.0, WheelPhase::Began),
            pixels(-12.0, WheelPhase::Changed),
            pixels(0.0, WheelPhase::Ended),
        ],
        // A notched wheel there once its fling is over. (One that cuts a
        // fling short stops it where the frame shown then put it, ahead of
        // the wheel's own time: where depends on when frames fell, faulted
        // or not, so no two live runs would agree; each run's own replay
        // checks that case frame by frame.)
        vec![lines(-1.0)],
        // Steps finer than motion's unit over the fine list.
        std::iter::once(Event::Moved(50.0, 30.0))
            .chain((0..FINE_STEPS).map(|_| pixels(-FINE_STEP, WheelPhase::None)))
            .collect(),
        // Where everything ended up, by the bar's click.
        vec![Event::Moved(50.0, 10.0), Event::Primary(true), Event::Primary(false)],
    ]
}

/// The fault matrix's fine steps: this many of [`FINE_STEP`] px, a delta
/// off motion's grid (an `f32` of 0.4 is 419,430.406 units), so each step
/// leaves a remainder, carried to the next (decision 4).
const FINE_STEPS: u8 = 6;
const FINE_STEP: f32 = 0.4;

/// Where the fine list ends, exact: the steps' sum in whole units, every
/// remainder carried, 2,516,582 units (dropped, 2,516,580). The other
/// oracles compare runs with runs, and a run with its replay: all of them
/// route through the one carry, so only an exact sum sees one dropped.
fn fine_end() -> f32 {
    // One step in units, unrounded (an f32 of 0.4 times a power of two),
    // then the sum rounded once.
    let step = f64::from(FINE_STEP) * crate::convert::i64_to_f64(super::motion::units(1.0));
    super::motion::px(crate::convert::f64_to_i64((step * f64::from(FINE_STEPS)).round()))
}

/// Run the fault matrix's script live, `faults` injected before the
/// events numbered so (in the script's order, from 0).
fn run_faulted(faults: &[(usize, Fault)], tag: usize) -> Option<Faulted> {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Instant;

    // This thread stamps and sends the input as the platform thread does,
    // and is scheduled as that one is (ADR-0015). Its events are stamped
    // on a script's clock, so a busy machine can wake it past a stamp: that
    // wait is before the send, which the hop is timed from, and is
    // reported apart (`late_sends`).
    crate::priority::set_current(crate::priority::ThreadType::Platform);
    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let mail: Arc<()> = Arc::default();
    let mut w = crate::app::offscreen(
        a_faulted_page(Arc::clone(&calls), Arc::clone(&mail)),
        crate::app::RunOptions::default(),
        (300, 300),
        1.0,
        Duration::from_millis(8),
    )
    .ok()?;
    let path = std::env::temp_dir().join(format!("gui-fault-{}-{tag}.txt", std::process::id()));
    w.record_input(&path, (300, 300), 1.0).ok()?;
    type Seen = std::collections::BTreeMap<u64, Vec<(f32, f32)>>;
    let mut seen = Seen::new();
    fn look(w: &crate::window::Offscreen, seen: &mut Seen) -> u64 {
        let p = w.presented();
        if !p.scroll.is_empty() {
            let at: Vec<(f32, f32)> = (&p.scroll).into_iter().map(|s| s.offset).collect();
            seen.insert(p.frame, at);
        }
        p.frame
    }
    // Settled: input goes through the render thread, a frame was drawn
    // after frame `since` (one before the last fault), and none for a
    // while: what was sent is on screen. Past the last recovery
    // (`failed`), nothing draws and input stays with the UI: a while
    // with nothing new is settled.
    let failed = faults.iter().filter(|(_, f)| *f == Fault::Panic).count() > super::thread::MAX_RECOVERIES;
    let settle = |w: &crate::window::Offscreen, seen: &mut Seen, since: u64| -> u64 {
        let start = Instant::now();
        let (mut last, mut still) = (look(w, seen), Instant::now());
        while start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(1));
            let f = look(w, seen);
            if f != last {
                (last, still) = (f, Instant::now());
            }
            let ready = if failed { true } else { w.through() && f > since };
            if ready && still.elapsed() >= Duration::from_millis(if failed { 200 } else { 40 }) {
                return f;
            }
        }
        last
    };
    // The app shown.
    let start = Instant::now();
    while w.presented().scroll.is_empty() && start.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(2));
    }
    let mut frame = settle(&w, &mut seen, 0);
    drop(w.take_forward_latencies());
    drop(w.take_input_latencies());
    let mut hops = Vec::new();
    let mut takes = Vec::new();
    let mut late_sends = Vec::new();
    let mut faults_seen = false;
    let mut index = 0;
    // The step before lifted a trackpad's fingers, and no frame was drawn
    // since its last event (no device yet): its fling goes on unseen, so
    // the next step waits out the longest a fling lasts, as frames drawn
    // would have shown it ending.
    let mut lifted_unseen = false;
    for step in fault_script() {
        frame = settle(&w, &mut seen, frame.saturating_sub(1));
        if std::mem::take(&mut lifted_unseen) {
            std::thread::sleep(Duration::from_millis(1_600));
            frame = settle(&w, &mut seen, frame.saturating_sub(1));
        }
        let lifts = step.iter().any(|e| {
            matches!(
                e,
                Event::Wheel {
                    phase: WheelPhase::Ended,
                    ..
                }
            )
        });
        let mut faulted = None;
        // How late after its stamp each event was sent.
        let mut sends = Vec::new();
        // Each event stamped 4 ms after the one before, however late it
        // is sent (four runs share the machine): a gesture's speed, and
        // so its fling, the same in every run, faulted or not.
        let begun = Instant::now();
        for (k, event) in step.into_iter().enumerate() {
            let after = Duration::from_millis(4).saturating_mul(u32::try_from(k).unwrap_or(u32::MAX));
            let stamp = begun.checked_add(after).unwrap_or(begun);
            // Sent at its stamp, not after: a sleep's overshoot would count
            // in the input's hop, which is timed from the stamp. Slept to a
            // millisecond before, then spun.
            if let Some(wait) = stamp.checked_duration_since(Instant::now())
                && let Some(nap) = wait.checked_sub(Duration::from_millis(1))
            {
                std::thread::sleep(nap);
            }
            while Instant::now() < stamp {
                std::hint::spin_loop();
            }
            for (_, fault) in faults.iter().filter(|(at, _)| *at == index) {
                faulted.get_or_insert(look(&w, &mut seen));
                match fault {
                    Fault::Lose => w.lose_device(),
                    Fault::Hold => w.lose_device_for(Duration::from_millis(60)),
                    Fault::Panic => w.panic_render(),
                    Fault::At(stage) => w.panic_render_at(*stage),
                }
            }
            let now = Instant::now();
            sends.push(now.saturating_duration_since(stamp));
            w.send_at(event, stamp.min(now));
            index = index.saturating_add(1);
            look(&w, &mut seen);
        }
        let last_sent = look(&w, &mut seen);
        if let Some(before) = faulted {
            // Recovered, a frame drawn since: the input handed on
            // meanwhile took its hop, and the UI took it.
            frame = settle(&w, &mut seen, before);
            late_sends.append(&mut sends);
        }
        if faulted.is_some() || faults_seen {
            // Timed from the first fault to the end, every step: a fault
            // a busy machine has the render thread see late takes effect
            // after its step settled, and the input it held is handed on
            // in a later one (the matrix under load: diverted 140 ms after
            // its step, those takes were dropped and none timed).
            faults_seen = true;
            takes.extend(w.take_input_latencies().into_iter().map(|(_, d)| d));
            hops.extend(w.take_forward_latencies().into_iter().map(|(_, d)| d));
        } else {
            // No fault yet: no recovery to time.
            drop(w.take_input_latencies());
            drop(w.take_forward_latencies());
        }
        lifted_unseen = lifts && settle(&w, &mut seen, frame.saturating_sub(1)) <= last_sent.saturating_add(1);
    }
    settle(&w, &mut seen, frame.saturating_sub(1));
    if faults_seen {
        takes.extend(w.take_input_latencies().into_iter().map(|(_, d)| d));
        hops.extend(w.take_forward_latencies().into_iter().map(|(_, d)| d));
    }
    // A scope made by each click: only the last click's mailbox holds
    // `mail`, beside this and the page's own closure, once the UI has let
    // the others go.
    let start = Instant::now();
    while Arc::strong_count(&mail) > 3 && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let mail_kept = Arc::strong_count(&mail).saturating_sub(3);
    drop(w);
    // The log is written on a thread of its own, which a busy machine
    // runs late: read once every event sent is in it (the matrix under
    // load: a run's last click missing from its log, read too soon).
    let start = Instant::now();
    let (text, log) = loop {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let log = InputLog::parse(&text).ok();
        let whole = log.as_ref().is_some_and(|l| l.events.len() >= index);
        if whole || start.elapsed() > Duration::from_secs(10) {
            break (text, log?);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    std::fs::remove_file(&path).ok();
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    Some(Faulted {
        calls,
        log,
        text,
        seen,
        hops,
        takes,
        late_sends,
        mail_kept,
    })
}

/// Whether any event of `log` went straight to the UI, input diverted.
fn diverted_input(log: &InputLog) -> bool {
    let mut diverted = false;
    lines(log).into_iter().any(|(line, _)| match line {
        Line::Observed(Observation::Diverted) => {
            diverted = true;
            false
        }
        Line::Observed(Observation::Restored) => {
            diverted = false;
            false
        }
        Line::Event(_) => diverted,
        Line::Observed(_) => false,
    })
}

/// Replay `run`'s log: what its handlers heard and where each frame it
/// was seen to show had its lists, against the live run's.
fn replay_faulted(run: &Faulted) -> Result<(), String> {
    use std::sync::{Arc, Mutex, PoisonError};

    let calls: Arc<Mutex<Vec<String>>> = Arc::default();
    let page = a_faulted_page(Arc::clone(&calls), Arc::default())();
    let mut s = Scenario::measuring(page, 300, 300, 1.0).map_err(|e| format!("{e:?}"))?;
    s.settle().map_err(|e| format!("{e:?}"))?;
    let frames =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| replay_observed(&mut s, &run.log))).map_err(|p| {
            p.downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default()
        })?;
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if calls != run.calls {
        return Err(format!("handlers: live {:?}, replayed {calls:?}", run.calls));
    }
    let mut differ = Vec::new();
    // Frames drawn before the log began (the app's first, as recording
    // started) are its starting state, not its to account for.
    let first = run
        .log
        .observed
        .iter()
        .find_map(|o| match o.what {
            Observation::Drawn(n, _) => Some(n),
            _ => None,
        })
        .unwrap_or(u64::MAX);
    for (n, live) in run.seen.range(first..) {
        // Every frame seen live is in the replay: one missing is a frame
        // the log does not account for (Codex r13 6).
        let Some(Shown::Frame(p)) = frames.get(n) else {
            differ.push((*n, live.clone(), Vec::new()));
            continue;
        };
        let replayed: Vec<(f32, f32)> = (&p.scroll).into_iter().map(|s| s.offset).collect();
        if &replayed != live {
            differ.push((*n, live.clone(), replayed));
        }
    }
    if differ.is_empty() {
        Ok(())
    } else {
        Err(format!("frames (live, replayed) differ: {differ:?}"))
    }
}

/// The fault matrix, case [(6, panic as it diverts), (6, panic)] under
/// load: the render thread diverted 140 ms late, so the rest of a trackpad
/// gesture (its lift and the platform's momentum, to its end) all went to
/// the UI before input was restored. The momentum's end ended the UI's
/// latch, and its lifted motion went with the latch, so nothing was handed
/// back and the gesture flung nothing (inner list 163 for 1,467.06). A
/// lifted motion goes back on its own, to the container it moved.
#[test]
fn a_gesture_whose_momentum_ended_while_diverted_still_flings() {
    use std::fmt::Write as _;

    let replay = |diverted: bool| {
        let mut text = String::from("gui-input-log 2\nplatform other\nwindow 300 300 1\nsource test\n");
        let mut line = |t: f64, what: &str| {
            writeln!(text, "{t:.6} {what}").ok();
        };
        line(1.0, "obs drawn 1 8.000000");
        line(2.0, "move 100 150");
        let gesture = [
            (849.0, "-10 began"),
            (853.0, "-20 changed"),
            (857.0, "-20 changed"),
            (861.0, "-15 changed"),
            (865.0, "0 ended"),
            (869.0, "-8 momentum-began"),
            (873.0, "-6 momentum"),
            (877.0, "-4 momentum"),
            (881.0, "0 momentum-ended"),
        ];
        let mut frame = 1_u32;
        let mut next_frame = 16.0;
        for (k, (t, step)) in gesture.iter().enumerate() {
            if diverted && k == 3 {
                line(860.0, "obs diverted");
            }
            line(*t, &format!("wheel pixels 0 {step}"));
        }
        if diverted {
            line(1003.0, "obs restored");
        }
        // Frames every 16 ms until the fling is over.
        while next_frame < 3_000.0 {
            frame = frame.saturating_add(1);
            if next_frame > 849.0 {
                line(next_frame, &format!("obs drawn {frame} {:.6}", next_frame + 8.0));
            }
            next_frame += 16.0;
        }
        line(3_010.0, "move 50 10");
        line(3_014.0, "down");
        line(3_018.0, "up");
        let log = InputLog::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let calls: std::sync::Arc<std::sync::Mutex<Vec<String>>> = std::sync::Arc::default();
        let page = a_faulted_page(std::sync::Arc::clone(&calls), std::sync::Arc::default())();
        let mut s = Scenario::measuring(page, 300, 300, 1.0).expect("a scenario");
        s.settle().expect("a frame");
        replay_observed(&mut s, &log);
        let calls = calls.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        calls.last().cloned().unwrap_or_default()
    };
    let (free, diverted) = (replay(false), replay(true));
    // Its events move 83 px; flung, 1,387.06.
    assert!(free.contains("inner (0.0, 1387"), "it flings: {free}");
    assert_eq!(diverted, free);
}

/// The user's ask (2026-11-25, the fault matrix): a render thread that
/// panics or a device lost, before each event of a script of notched
/// wheels, a trackpad's gesture with the platform's momentum, wheels over
/// a handler, a click, a drag and a move between nested lists, and a
/// second fault while the first recovers, loses no wheel and moves none
/// to another target: the handlers hear what a run with no fault heard
/// (the lists where they ended included), the latch has one owner at
/// every step of the log (checked in its replay), the input handed on
/// while recovering takes under a millisecond at the 99th percentile,
/// and every faulty run's log replays to what it showed and heard.
#[test]
fn a_fault_before_any_input_loses_and_moves_no_wheel_and_replays() {
    let Some(clean) = run_faulted(&[], usize::MAX) else {
        eprintln!("no GPU: skipped");
        return;
    };
    replay_faulted(&clean).unwrap_or_else(|e| panic!("the run with no fault: {e}\n{}", clean.text));
    assert_eq!(
        clean.mail_kept, 0,
        "the run with no fault kept disposed scopes' mailboxes"
    );
    // The fine list at the steps' exact sum: the other runs must hear
    // what this one heard, so they are held to it too.
    let fine = format!("fine {:?}", (0.0_f32, fine_end()));
    assert!(
        clean.calls.last().is_some_and(|c| c.ends_with(&fine)),
        "the run with no fault: {:?}, not {fine}",
        clean.calls.last()
    );
    use super::thread::PanicStage;
    let events: usize = fault_script().iter().map(Vec::len).sum();
    let mut plans: Vec<Vec<(usize, Fault)>> = Vec::new();
    for i in 0..events {
        for f in [Fault::Lose, Fault::Hold, Fault::Panic] {
            plans.push(vec![(i, f)]);
        }
        // A panic as it diverts after a panic; one while its device is
        // made on the helper thread (held lost, so the making takes a
        // while).
        plans.push(vec![(i, Fault::At(PanicStage::Divert)), (i, Fault::Panic)]);
        plans.push(vec![(i, Fault::At(PanicStage::Remake)), (i, Fault::Hold)]);
        // A second fault while the first recovers.
        if i + 1 < events {
            for (a, b) in [
                (Fault::Panic, Fault::Panic),
                (Fault::Panic, Fault::Hold),
                (Fault::Hold, Fault::Panic),
                (Fault::Lose, Fault::Lose),
            ] {
                plans.push(vec![(i, a), (i + 1, b)]);
            }
            // The second panic as the UI's latch, on its way when the
            // first diverted again, is returned: between that and the
            // restore.
            plans.push(vec![
                (i, Fault::At(PanicStage::Returned)),
                (i, Fault::Panic),
                (i + 1, Fault::Panic),
            ]);
        }
    }
    // Past the last recovery: the UI keeps taking the input, as the one
    // owner, for the rest of the script.
    for i in [0, 3, events / 2] {
        plans.push(vec![(i, Fault::Panic); super::thread::MAX_RECOVERIES + 1]);
    }
    // A subset of the plans, run by fewer workers, for measuring a hop
    // with less contention (`GUI_MATRIX_PLANS`, `GUI_MATRIX_WORKERS`).
    let only: usize = std::env::var("GUI_MATRIX_PLANS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let workers: usize = std::env::var("GUI_MATRIX_WORKERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4);
    let plans = std::sync::Mutex::new(plans.into_iter().enumerate().take(only).collect::<Vec<_>>());
    let cases = std::sync::atomic::AtomicUsize::new(0);
    let found: std::sync::Mutex<Vec<String>> = std::sync::Mutex::default();
    let hops: std::sync::Mutex<Vec<Duration>> = std::sync::Mutex::default();
    let takes: std::sync::Mutex<Vec<Duration>> = std::sync::Mutex::default();
    let late_sends: std::sync::Mutex<Vec<Duration>> = std::sync::Mutex::default();
    // Each run's slowest hop or take, and its case.
    let worst: std::sync::Mutex<Vec<(Duration, String)>> = std::sync::Mutex::default();
    // How late the OS wakes a thread of this process meanwhile: canaries
    // sleeping 1 ms at a time, their oversleep the run queue's delay, one
    // as a test's thread is scheduled and one as the platform thread is.
    let running = std::sync::atomic::AtomicBool::new(true);
    let late: std::sync::Mutex<Vec<(&str, Vec<Duration>)>> = std::sync::Mutex::default();
    std::thread::scope(|scope| {
        for (name, raised) in [("a test's", false), ("the platform's", true)] {
            let (running, late) = (&running, &late);
            scope.spawn(move || {
                if raised {
                    crate::priority::set_current(crate::priority::ThreadType::Platform);
                }
                let mut mine = Vec::new();
                while running.load(std::sync::atomic::Ordering::Relaxed) {
                    let asked = std::time::Instant::now();
                    std::thread::sleep(Duration::from_millis(1));
                    mine.push(asked.elapsed().saturating_sub(Duration::from_millis(1)));
                }
                late.lock().unwrap().push((name, mine));
            });
        }
        let mut runs = Vec::new();
        for _ in 0..workers {
            runs.push(scope.spawn(|| {
                loop {
                    let Some((n, plan)) = plans.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pop() else {
                        return;
                    };
                    let Some(run) = run_faulted(&plan, n) else {
                        found.lock().unwrap().push(format!("case {n} {plan:?}: did not run"));
                        continue;
                    };
                    cases.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    hops.lock().unwrap().extend(run.hops.iter().copied());
                    takes.lock().unwrap().extend(run.takes.iter().copied());
                    late_sends.lock().unwrap().extend(run.late_sends.iter().copied());
                    if let Some(t) = run.takes.iter().chain(&run.hops).max() {
                        worst.lock().unwrap().push((*t, format!("case {n} {plan:?}")));
                    }
                    let mut why = Vec::new();
                    // Past the last recovery the UI scrolls alone and
                    // flings nothing (the compositor's own motion): what
                    // its handlers heard is checked against its replay.
                    let terminal =
                        plan.iter().filter(|(_, f)| *f == Fault::Panic).count() > super::thread::MAX_RECOVERIES;
                    // Input handed on while diverted was taken, and timed
                    // (a device lost while no event came diverts none).
                    if diverted_input(&run.log) && run.takes.is_empty() {
                        why.push("the UI took input handed on while it recovered, untimed".to_owned());
                    }
                    if !terminal && run.calls != clean.calls {
                        why.push(format!("handlers: {:?}, with no fault {:?}", run.calls, clean.calls));
                    }
                    if terminal && run.calls.is_empty() {
                        why.push("past the last recovery, the handlers heard nothing".to_owned());
                    }
                    if run.mail_kept > 0 {
                        why.push(format!("{} mailboxes of disposed scopes kept", run.mail_kept));
                    }
                    if let Err(e) = replay_faulted(&run) {
                        why.push(format!("replay: {e}"));
                    }
                    if !why.is_empty() {
                        found
                            .lock()
                            .unwrap()
                            .push(format!("case {n} {plan:?}: {}\n{}", why.join("; "), run.text));
                    }
                }
            }));
        }
        let ended: Vec<_> = runs.into_iter().map(|run| run.join()).collect();
        running.store(false, std::sync::atomic::Ordering::Relaxed);
        for end in ended {
            if let Err(panic) = end {
                std::panic::resume_unwind(panic);
            }
        }
    });
    let found = found.into_inner().unwrap();
    let p99 = |mut d: Vec<Duration>| {
        d.sort();
        let p = d.get((d.len() * 99).div_ceil(100).saturating_sub(1)).copied();
        (d.len(), p)
    };
    let hops = hops.into_inner().unwrap();
    let hop_max = hops.iter().max().copied();
    let (hop_count, hop_p99) = p99(hops);
    let (take_count, take_p99) = p99(takes.into_inner().unwrap());
    let late_sends = late_sends.into_inner().unwrap();
    let late_max = late_sends.iter().max().copied();
    let (sent_count, late_p99) = p99(late_sends);
    let late: Vec<String> = late
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|(name, d)| {
            let max = d.iter().max().copied();
            let (n, p) = p99(d);
            format!("{name} 1 ms sleep overslept p99 {p:?}, max {max:?} of {n}")
        })
        .collect();
    eprintln!(
        "{} cases of {events} events; {hop_count} hops while recovering, p99 {hop_p99:?}, max {hop_max:?}; \
         {take_count} UI takes then, p99 {take_p99:?}; {sent_count} sent after their stamps by p99 {late_p99:?}, \
         max {late_max:?}; {}; {} found",
        cases.into_inner(),
        late.join("; "),
        found.len()
    );
    let mut worst = worst.into_inner().unwrap();
    worst.sort_by_key(|w| std::cmp::Reverse(w.0));
    eprintln!("slowest runs (hop or take): {:?}", worst.get(..3).unwrap_or(&worst));
    assert!(
        found.is_empty(),
        "{} cases differ:\n{}",
        found.len(),
        found.join("\n\n")
    );
    // Measured, or failed: an empty sample proves nothing (Codex r13 6).
    let hop_p99 = hop_p99.expect("hops measured while recovering");
    let take_p99 = take_p99.expect("UI takes measured while recovering");
    assert!(
        hop_p99 < Duration::from_millis(1),
        "hop p99 while recovering {hop_p99:?}"
    );
    assert!(
        take_p99 < Duration::from_millis(16),
        "the UI's take p99 while recovering {take_p99:?}"
    );
}
