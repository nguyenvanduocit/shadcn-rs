//! Time as components see it: the frame clock that drives animation and the
//! timers that run work later (a tooltip's delay, a toast's dismissal).
//!
//! A frame's time is when it will be presented (ADR-0015): the render
//! thread predicts it from the display's vsync ([`Vsync`]) and hands it to
//! the UI thread with each main frame ([`begin_frame`]), so what moves is
//! sampled where it will be seen. Reading the time asks for nothing; what
//! moves on the UI thread asks for the next frame ([`request_frame`]), and
//! what the compositor moves needs no UI frame at all.
//!
//! The clock runs on real time until a script takes it over ([`script`]):
//! from then on time moves only when the script moves it ([`advance_to`]),
//! and the timers that fall due on the way run at their own times, in order,
//! so a scripted run is the same every time.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

struct Timer {
    /// When it runs, on the clock.
    due: Duration,
    id: u64,
    run: Rc<dyn Fn()>,
    /// What owned it when it was made: its call runs as that owner, so a
    /// timer it sets again (an autoplay) is owned, and cancelled, too.
    owner: Option<crate::reactive::OwnerId>,
}

/// When the process's clock started: every thread's times are from it.
fn start() -> Instant {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// `at` as seconds on the frame clock (real time, from any thread).
#[must_use]
pub fn seconds(at: Instant) -> f32 {
    at.saturating_duration_since(start()).as_secs_f32()
}

/// The moment `seconds` on the frame clock is (real time).
#[must_use]
pub fn instant(seconds: f32) -> Instant {
    let d = Duration::try_from_secs_f32(seconds.max(0.0)).unwrap_or_default();
    start().checked_add(d).unwrap_or_else(start)
}

crate::engine::engine_local! {
    /// The time frames read, fixed per frame so one frame is consistent.
    [Fork] static FRAME_TIME: Cell<Option<f32>> = const { Cell::new(None) };
    /// Where a script has set the clock, once one drives it.
    [Fork] static SCRIPTED: Cell<Option<Duration>> = const { Cell::new(None) };
    [Fork] static WANTS_FRAME: Cell<bool> = const { Cell::new(false) };
    /// A cold frame's timers are its own; the live ones it can only see
    /// ([`Queue`]).
    [Fork(cold_enter, cold_drain, cold_exit)] static TIMERS: RefCell<Queue> = RefCell::new(Queue::default());
    [Fork] static NEXT_TIMER: Cell<u64> = const { Cell::new(1) };
}

/// The timers pending. In a cold frame, `timers` are the frame's (none at
/// first, never run live), and the live ones are set aside where the
/// frame reads, never runs or changes them: whether one is pending
/// ([`is_pending`]) answers as it does live.
#[derive(Default)]
struct Queue {
    timers: Vec<Timer>,
    /// While a cold frame runs: the live timers, and those of them the
    /// frame cancelled (for its own reads only).
    live: Option<(Vec<Timer>, Vec<u64>)>,
}

impl Queue {
    fn is_pending(&self, id: u64) -> bool {
        self.timers.iter().any(|x| x.id == id)
            || self
                .live
                .as_ref()
                .is_some_and(|(live, gone)| !gone.contains(&id) && live.iter().any(|x| x.id == id))
    }

    fn cancel(&mut self, id: u64) {
        self.timers.retain(|x| x.id != id);
        if let Some((live, gone)) = self.live.as_mut()
            && live.iter().any(|x| x.id == id)
        {
            gone.push(id);
        }
    }
}

#[cfg(debug_assertions)]
fn cold_enter() -> Box<dyn std::any::Any> {
    crate::engine::fork_work(1);
    TIMERS.with(|t| {
        let mut q = t.borrow_mut();
        let live = std::mem::take(&mut q.timers);
        q.live = Some((live, Vec::new()));
    });
    Box::new(())
}

#[cfg(debug_assertions)]
fn cold_drain(_: &dyn std::any::Any) -> Box<dyn std::any::Any> {
    Box::new(TIMERS.with(|t| std::mem::take(&mut t.borrow_mut().timers)))
}

#[cfg(debug_assertions)]
fn cold_exit(_: Box<dyn std::any::Any>) -> Box<dyn std::any::Any> {
    TIMERS.with(|t| {
        let mut q = t.borrow_mut();
        let live = q.live.take().map(|(live, _)| live).unwrap_or_default();
        Box::new(std::mem::replace(&mut q.timers, live))
    })
}

/// The time on the clock: since the app started, or where a script set it.
fn current() -> Duration {
    SCRIPTED.with(Cell::get).unwrap_or_else(|| start().elapsed())
}

/// Whether a script drives the clock ([`script`]).
#[must_use]
pub fn scripted() -> bool {
    SCRIPTED.with(Cell::get).is_some()
}

/// The time on the clock now, exact: real time, or where a script set it.
/// For what measures spans (a swipe's speed, a countdown's time left),
/// which `f32` seconds quantize after days of uptime.
#[must_use]
pub fn current_time() -> Duration {
    current()
}

/// The time on the clock now, in seconds: real time, or where a script set it.
#[must_use]
pub fn current_seconds() -> f32 {
    current().as_secs_f32()
}

/// Ask for the next UI frame: something the UI lays out moves on its own
/// time (a transition of a colour, a height).
pub fn request_frame() {
    WANTS_FRAME.with(|w| w.set(true));
}

/// The frame's time without asking for another frame (painting reads it).
#[must_use]
pub fn frame_time() -> f32 {
    FRAME_TIME.with(Cell::get).unwrap_or_else(|| current().as_secs_f32())
}

/// Fix the clock for the frame about to be built, to be presented at `at`
/// seconds on the clock (a scripted clock's frames are at its time).
pub fn begin_frame(at: f32) {
    FRAME_TIME.with(|f| f.set(Some(at)));
}

/// Whether something read the clock since the last call; clears the flag.
pub fn take_frame_request() -> bool {
    WANTS_FRAME.with(|w| w.replace(false))
}

/// Let a script drive the clock from here on, starting at `at`: time then
/// moves only by [`advance_to`]. Timers already pending keep the delay they
/// have left.
pub fn script(at: Duration) {
    let was = current();
    TIMERS.with(|t| {
        for timer in t.borrow_mut().timers.iter_mut() {
            timer.due = at.saturating_add(timer.due.saturating_sub(was));
        }
    });
    set_scripted(at);
}

/// Move the scripted clock to `to`, running each timer that falls due by
/// then at its own time, earliest first (one a timer schedules on the way
/// included); returns whether any ran. A clock set back runs nothing.
pub fn advance_to(to: Duration) -> bool {
    let mut ran = false;
    while let Some(timer) = take_due(to) {
        set_scripted(timer.due.max(current()));
        crate::reactive::run_owned(timer.owner, || (timer.run)());
        ran = true;
    }
    set_scripted(to);
    ran
}

fn set_scripted(at: Duration) {
    SCRIPTED.with(|s| s.set(Some(at)));
    // What a timer's work reads as "now" (an animation it starts) is its own time.
    FRAME_TIME.with(|f| f.set(Some(at.as_secs_f32())));
}

/// Remove and return the earliest timer due by `by`.
fn take_due(by: Duration) -> Option<Timer> {
    TIMERS.with(|t| {
        let timers = &mut t.borrow_mut().timers;
        let first = timers
            .iter()
            .enumerate()
            .filter(|(_, x)| x.due <= by)
            .min_by_key(|(_, x)| (x.due, x.id))
            .map(|(i, _)| i)?;
        Some(timers.remove(first))
    })
}

/// A scheduled call; dropping it does not cancel it, [`cancel`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerId(u64);

/// Run `f` once after `delay`. Made while a scope, effect or component
/// runs, it is owned there: cancelled when that owner is disposed (or the
/// effect runs again), as ADR-0013 has timers owned.
pub fn after(delay: Duration, f: impl Fn() + 'static) -> TimerId {
    let id = NEXT_TIMER.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    TIMERS.with(|t| {
        t.borrow_mut().timers.push(Timer {
            due: current().saturating_add(delay),
            id,
            run: Rc::new(f),
            owner: crate::reactive::owner(),
        });
    });
    if crate::reactive::has_owner() {
        crate::reactive::on_cleanup(move || cancel(TimerId(id)));
    }
    TimerId(id)
}

/// Drop every scheduled call, for a run that starts afresh (a scenario of
/// another page on the same thread).
pub fn cancel_all() {
    TIMERS.with(|t| t.borrow_mut().timers.clear());
}

/// Whether a scheduled call has not run yet (nor been dropped).
#[must_use]
pub fn is_pending(timer: TimerId) -> bool {
    TIMERS.with(|t| t.borrow().is_pending(timer.0))
}

/// Drop a scheduled call that has not run yet.
pub fn cancel(timer: TimerId) {
    TIMERS.with(|t| t.borrow_mut().cancel(timer.0));
}

/// Run every timer that is due, earliest first; returns whether any ran.
pub fn run_due_timers() -> bool {
    let now = current();
    let mut ran = false;
    while let Some(timer) = take_due(now) {
        crate::reactive::run_owned(timer.owner, || (timer.run)());
        ran = true;
    }
    ran
}

/// When the last timer due by `by` on the clock is due, if one is pending.
#[must_use]
pub fn last_due_by(by: Duration) -> Option<Duration> {
    TIMERS.with(|t| t.borrow().timers.iter().map(|x| x.due).filter(|&due| due <= by).max())
}

/// When the next timer is due, in real time.
#[must_use]
pub fn next_deadline() -> Option<Instant> {
    let due = TIMERS.with(|t| t.borrow().timers.iter().map(|x| x.due).min())?;
    start().checked_add(due)
}

/// The display's vsync as the render thread sees it (ADR-0015's
/// provisional source, with no `unsafe`): the moment a blocked
/// `get_current_texture` returns is a vblank, and what is drawn after it
/// shows at the next one. The phase is fitted to the last blocked acquires
/// (outliers past a quarter interval dropped); the interval is the
/// monitor's refresh rate. When winit or wgpu give a frame event or
/// presentation feedback, only this changes.
#[derive(Clone, Debug)]
pub struct Vsync {
    interval: Duration,
    /// Blocked acquires, newest last.
    samples: std::collections::VecDeque<Instant>,
    /// A vblank, as fitted.
    phase: Option<Instant>,
}

/// How many blocked acquires the phase is fitted to.
const VSYNC_SAMPLES: usize = 16;
/// An acquire that waited longer than this waited for a vblank.
const VSYNC_BLOCKED: Duration = Duration::from_micros(500);

impl Vsync {
    /// A display refreshing every `interval`.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval: interval.max(Duration::from_millis(1)),
            samples: std::collections::VecDeque::new(),
            phase: None,
        }
    }

    /// A display refreshing `millihertz` thousandths of times a second
    /// (winit's `refresh_rate_millihertz`).
    #[must_use]
    pub fn from_millihertz(millihertz: u32) -> Self {
        let hz = f64::from(millihertz.max(1)) / 1000.0;
        Self::new(Duration::from_secs_f64(1.0 / hz))
    }

    #[must_use]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// The display refreshes every `interval` now (another monitor).
    pub fn set_interval(&mut self, interval: Duration) {
        if interval != self.interval {
            *self = Self::new(interval);
        }
    }

    /// An acquire returned at `at` after waiting `waited`.
    pub fn acquired(&mut self, at: Instant, waited: Duration) {
        if waited < VSYNC_BLOCKED {
            return;
        }
        self.samples.push_back(at);
        if self.samples.len() > VSYNC_SAMPLES {
            self.samples.pop_front();
        }
        self.fit();
    }

    /// Fit the phase: each sample's offset from the newest, folded into
    /// half an interval either way; what lies more than a quarter interval
    /// from their median (a late wake, the newest one included) is dropped,
    /// the rest averaged.
    fn fit(&mut self) {
        let Some(&base) = self.samples.back() else { return };
        let i = self.interval.as_secs_f64();
        let fold = |s: Instant| -> f64 {
            let d = if s >= base {
                s.duration_since(base).as_secs_f64()
            } else {
                -base.duration_since(s).as_secs_f64()
            };
            d - (d / i).round() * i
        };
        let mut folded: Vec<f64> = self.samples.iter().map(|&s| fold(s)).collect();
        folded.sort_by(f64::total_cmp);
        let median = folded.get(folded.len() / 2).copied().unwrap_or(0.0);
        let kept: Vec<f64> = folded.into_iter().filter(|r| (r - median).abs() <= i / 4.0).collect();
        let mean = if kept.is_empty() {
            0.0
        } else {
            kept.iter().sum::<f64>() / f64::from(u32::try_from(kept.len()).unwrap_or(1))
        };
        self.phase = Some(if mean >= 0.0 {
            base.checked_add(Duration::from_secs_f64(mean)).unwrap_or(base)
        } else {
            base.checked_sub(Duration::from_secs_f64(-mean)).unwrap_or(base)
        });
    }

    /// The first vblank after `after`; with no phase yet, `after` and an interval.
    #[must_use]
    pub fn next_after(&self, after: Instant) -> Instant {
        let Some(phase) = self.phase else {
            return after.checked_add(self.interval).unwrap_or(after);
        };
        let i = self.interval.as_secs_f64();
        let ahead = if after >= phase {
            let n = (after.duration_since(phase).as_secs_f64() / i).floor() + 1.0;
            n * i
        } else {
            let back = phase.duration_since(after).as_secs_f64();
            back - (back / i).floor() * i
        };
        let next = phase
            .checked_add(Duration::from_secs_f64(ahead.max(0.0)))
            .or_else(|| phase.checked_sub(Duration::from_secs_f64(-ahead)))
            .unwrap_or(after);
        if next <= after {
            next.checked_add(self.interval).unwrap_or(next)
        } else {
            next
        }
    }

    /// When a frame begun at `at` (an acquire returned then, or the render
    /// thread woke) will be presented: the vblank after it, with `margin`
    /// to draw in.
    #[must_use]
    pub fn present_time(&self, at: Instant, margin: Duration) -> Instant {
        self.next_after(at.checked_add(margin).unwrap_or(at))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scripted_clock_runs_timers_at_their_own_times_in_order() {
        script(Duration::ZERO);
        let seen = Rc::new(RefCell::new(Vec::new()));
        // Re-arms itself every second, as an autoplay does.
        fn tick(seen: Rc<RefCell<Vec<f32>>>) {
            let again = Rc::clone(&seen);
            after(Duration::from_secs(1), move || {
                again.borrow_mut().push(frame_time());
                tick(Rc::clone(&again));
            });
        }
        tick(Rc::clone(&seen));
        let late = Rc::clone(&seen);
        after(Duration::from_millis(2500), move || {
            late.borrow_mut().push(-frame_time())
        });
        assert!(!run_due_timers(), "nothing is due before the script moves time");
        assert!(advance_to(Duration::from_millis(3200)));
        assert_eq!(*seen.borrow(), [1.0, 2.0, -2.5, 3.0]);
        assert!((frame_time() - 3.2).abs() < 1e-6);
    }
}
