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

use super::input_log::{Event, Header, InputLog, Phase, Platform, Routed, Router, Stamped, WheelDelta};
use crate::app::Scenario;
use crate::clock;
use crate::color::Color;
use crate::element::{IntoElement, div};
use crate::events::DragPhase;
use crate::scroll::ScrollHandle;
use crate::styled::Styled;
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
    let Header {
        size, scale, platform, ..
    } = log.header;
    let handle = ScrollHandle::new();
    let clicked: Rc<RefCell<Vec<i32>>> = Rc::default();
    let drags: Rc<RefCell<Vec<(DragPhase, f32, f32)>>> = Rc::default();
    let (h, c, d) = (handle.clone(), Rc::clone(&clicked), Rc::clone(&drags));
    let mut s = Scenario::measuring(
        move || {
            let c = Rc::clone(&c);
            let d = Rc::clone(&d);
            div()
                .size_full()
                .flex_row()
                .child(
                    div()
                        .h_full()
                        .flex_1()
                        .overflow_y_scroll()
                        .track_scroll(&h)
                        .children((0..ROWS).map(move |i| {
                            let c = Rc::clone(&c);
                            div()
                                .h_px(ROW)
                                .bg(if i % 2 == 0 { Color::hex(0xf4f4f5) } else { Color::WHITE })
                                .on_click(move |_| c.borrow_mut().push(i))
                        })),
                )
                .child(
                    div()
                        .w_px(PAD)
                        .h_full()
                        .bg(Color::hex(0xe4e4e7))
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
                    ended,
                } => {
                    let c = &mut s.parts().1.compositor;
                    let t = f64_f32(at);
                    let hit = if precise {
                        c.gesture(point, delta, t, ended)
                    } else {
                        c.wheel(point, delta, t)
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
                Routed::Ui(input) => {
                    // The UI thread's handling (`window::ui_loop`).
                    let shown = s.presented();
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
                        Input::MouseUp { .. } => {
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
                let shown = s.presented();
                s.pipeline_mut().input_shown(Input::PointerMoved { x, y }, &shown);
            }
            s.settle().expect("a frame");
            wants = false;
            owed_for = 0;
        } else if dirty || animating {
            // A main frame owed and not answered yet draws nothing by
            // itself: the render thread waits for it asleep.
            s.parts().1.draw(at);
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
        let late: Vec<&Frame> = run
            .frames
            .iter()
            .filter(|f| !f.ui && f.vblank >= w.at + PREDICT_FOR + interval && f.vblank < next)
            .collect();
        assert!(
            late.is_empty(),
            "{what}: the compositor drew {} frames more than 35 ms after the last event at {:.3} s (first at {:.3} s)",
            late.len(),
            w.at - T0,
            late.first().map_or(0.0, |f| f.vblank - T0)
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
            DragPhase::End => {
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
        let moving: Vec<&&Frame> = after.iter().filter(|f| !f.ui).collect();
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
        let drawn_after = run
            .frames
            .iter()
            .filter(|f| f.vblank > last + PREDICT_FOR + 1.0 / hz)
            .count();
        assert_eq!(drawn_after, 0, "nothing drawn 35 ms after the momentum's last event");
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

    fn wheel(&mut self, ms: f64, delta: WheelDelta, phase: Phase) {
        self.at(ms, Event::Wheel { delta, phase });
    }

    fn pixels(&mut self, ms: f64, dy: f64, phase: Phase) {
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
        }
    }
}

/// A macOS trackpad gesture's finger phase from `start` ms: `deltas` in
/// points (winit's sign), one every `gap` ms with jitter, the first
/// `started` unless the gesture `began` already; returns when the last came.
fn fingers(s: &mut Synth, j: &mut Jitter, start: f64, gap: f64, deltas: &[f64], began: bool) -> f64 {
    let mut t = start;
    for (i, d) in deltas.iter().enumerate() {
        let phase = if i == 0 && !began { Phase::Started } else { Phase::Moved };
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
         phase None) and winit 0.30 maps it (src/platform_impl/macos/view.rs scroll_wheel: momentum phase first, \
         MayBegin and Began are started, Ended and Cancelled ended, the rest moved; PixelDelta in logical points). \
         The finger phase is sampled every 8-9 ms (a trackpad's report rate as AppKit coalesces it), the momentum \
         every 16.7 ms decaying by 0.998 a millisecond (UIScrollView.DecelerationRate.normal, the system's \
         documented deceleration) until under half a point; deltas are made up within those rules. A click \
         lands on the list during the momentum.",
    );
    for i in 0..5 {
        s.at(f64::from(i) * 10.0, Event::Moved(150.0, 160.0 + f64_f32(f64::from(i))));
    }
    s.pixels(100.0, 0.0, Phase::Started);
    let ramp = [
        -2.0, -4.0, -7.0, -11.0, -15.0, -19.0, -22.0, -24.0, -25.0, -25.0, -24.0, -22.0, -20.0,
    ];
    let last = fingers(&mut s, &mut j, 112.0, 8.4, &ramp, false);
    s.pixels(last + 7.9, 0.0, Phase::Ended);
    let mut t = last + 8.0 + 8.3;
    let mut d: f64 = -20.0 * 0.98;
    let mut first = true;
    while d.abs() >= 0.5 {
        s.pixels(t, d, if first { Phase::Started } else { Phase::Moved });
        first = false;
        let gap = 16.67 + j.next() * 0.4;
        d *= 0.998_f64.powf(gap);
        t += gap;
    }
    s.pixels(t, 0.0, Phase::Ended);
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
         (phase MayBegin, then Cancelled, which winit 0.30 maps to started and ended). Phase mapping as in \
         winit's macos/view.rs scroll_wheel.",
    );
    s.at(0.0, Event::Moved(200.0, 120.0));
    s.pixels(50.0, 0.0, Phase::Started);
    let slow: Vec<f64> = (0..20).map(|i| -(2.0 + f64::from(i % 4))).collect();
    let last = fingers(&mut s, &mut j, 60.0, 10.0, &slow, false);
    let back: Vec<f64> = (0..15).map(|i| 1.5 + f64::from(i % 3)).collect();
    let last = fingers(&mut s, &mut j, last + 200.0, 11.0, &back, true);
    s.pixels(last + 20.0, 0.0, Phase::Ended);
    s.pixels(last + 400.0, 0.0, Phase::Started);
    s.pixels(last + 520.0, 0.0, Phase::Cancelled);
    s.click(last + 700.0, 200.0, 120.0, 60.0);
    out.push(("macos-trackpad-slow.log".to_owned(), s.done()));

    // macOS: a mouse with a notched wheel.
    let mut s = Synth::new(
        Platform::MacOs,
        (480, 320),
        2.0,
        "synthesized, not recorded: a notched mouse wheel on macOS. AppKit reports it without precise deltas \
         (hasPreciseScrollingDeltas false), scrollingDeltaY in lines, accelerated when the wheel spins fast, \
         phase and momentumPhase None, which winit 0.30 maps to LineDelta and moved. Notches 120 ms apart, then \
         a spin 25 ms apart with the acceleration rising and falling.",
    );
    s.at(0.0, Event::Moved(120.0, 100.0));
    for i in 0..4 {
        s.wheel(100.0 + f64::from(i) * 120.0, WheelDelta::Lines(0.0, -1.0), Phase::Moved);
    }
    for (i, l) in [1.0, 1.0, 2.0, 3.0, 4.0, 4.0, 3.0, 2.0, 1.0].iter().enumerate() {
        s.wheel(
            700.0 + f64_from(i) * 25.0,
            WheelDelta::Lines(0.0, f64_f32(*l)),
            Phase::Moved,
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
        s.wheel(100.0 + f64::from(i) * 80.0, WheelDelta::Lines(0.0, -1.0), Phase::Moved);
    }
    for i in 0..10 {
        s.wheel(600.0 + f64::from(i) * 12.0, WheelDelta::Lines(0.0, 1.0), Phase::Moved);
    }
    let mut t = 1000.0;
    for units in [-24.0, -32.0, -40.0, -48.0, -48.0, -40.0, -40.0, -32.0] {
        s.wheel(t, WheelDelta::Lines(0.0, f64_f32(units / 120.0)), Phase::Moved);
        t += 8.0 + j.next();
    }
    let mut units: f64 = -30.0;
    while units.round() != 0.0 {
        s.wheel(t, WheelDelta::Lines(0.0, f64_f32(units.round() / 120.0)), Phase::Moved);
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
         the first started, the rest moved, the stop ended with a zero PixelDelta. Three gestures: lifted while \
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
            s.pixels(t, *d, if i == 0 { Phase::Started } else { Phase::Moved });
            t += 9.0 + j.next() * 1.5;
        }
        let lift = t - 9.0 + 9.0 + rest;
        s.pixels(lift, 0.0, Phase::Ended);
    };
    gesture(&mut s, 100.0, 0.0);
    gesture(&mut s, 3000.0, 60.0);
    gesture(&mut s, 6000.0, 200.0);
    for i in 0..6 {
        s.wheel(9000.0 + f64::from(i) * 40.0, WheelDelta::Lines(0.0, -1.0), Phase::Moved);
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
    s.wheel(1610.0, WheelDelta::Lines(0.0, -2.0), Phase::Moved);
    s.click(1611.0, 100.0, 130.0, 5.0);
    s.wheel(1700.0, WheelDelta::Lines(0.0, 1.0), Phase::Moved);
    s.click(1702.0, 100.0, 130.0, 3.0);
    out.push(("drag-and-click.log".to_owned(), s.done()));
    out
}

fn f64_from(i: usize) -> f64 {
    f64::from(u32::try_from(i).unwrap_or(u32::MAX))
}
