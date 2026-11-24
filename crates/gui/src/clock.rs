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
/// (outliers past a quarter interval dropped), and to nothing else: a
/// frame that took no texture, or whose acquire failed, woke on a timer,
/// which is no vblank. On Wayland, where an acquire need not block, frame
/// callbacks are samples too, and their gaps the interval
/// ([`Vsync::called_back`]). Elsewhere the interval is the refresh rate of
/// the monitor the window is on, read again when it may be on another.
///
/// Where the OS has a vsync source of its own (ADR-0029: macOS's display
/// link), its ticks are the phase ([`Vsync::ticked`]) and blocked acquires
/// are not fitted while they come.
#[derive(Clone, Debug)]
pub struct Vsync {
    interval: Duration,
    /// Blocked acquires, newest last.
    samples: std::collections::VecDeque<Instant>,
    /// A vblank, as fitted.
    phase: Option<Instant>,
    /// Wayland: the last frame callback, and the gaps between the last
    /// few that came a refresh apart ([`Vsync::called_back`]).
    called: Option<Instant>,
    gaps: std::collections::VecDeque<Duration>,
    /// The refresh the platform reported for the monitor, if it did.
    reported: Option<Duration>,
    /// The display's last tick taken ([`Vsync::ticked`]): its output, its
    /// number, and when its source saw it.
    told: Option<(gui_display::OutputId, u64, Instant)>,
}

/// How long after a display tick blocked acquires stay out of the phase:
/// past it the display's source has stopped (the window was at rest) or
/// its output is gone, and the acquires are the phase again.
const TICK_FRESH: Duration = Duration::from_secs(1);

/// How many callback gaps the interval is the shortest of.
const CALLBACK_GAPS: usize = 8;
/// How many it takes first: one gap may span a refresh the compositor
/// skipped.
const CALLBACK_GAPS_LEAST: usize = 3;
/// The gaps taken for a refresh: 10 Hz to 500 Hz.
const CALLBACK_GAPS_IN: std::ops::RangeInclusive<Duration> = Duration::from_millis(2)..=Duration::from_millis(100);
/// How far off the interval may be before the callbacks set it.
const CALLBACK_TOLERANCE: f64 = 0.03;
/// How many gaps must agree on a refresh, and within how much.
const CALLBACK_AGREE: usize = 2;
const CALLBACK_JITTER: f64 = 0.1;
/// The slowest refresh the callbacks may show against the monitor's own
/// (some compositors slow an output; none halves it for one window).
const CALLBACK_SLOWEST: f64 = 1.5;

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
            called: None,
            gaps: std::collections::VecDeque::new(),
            reported: None,
            told: None,
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

    /// A vblank, as fitted to the acquires; none before the first.
    #[must_use]
    pub fn phase(&self) -> Option<Instant> {
        self.phase
    }

    /// The platform reports the monitor refreshing every `interval` (it is
    /// on another): the fit starts afresh, and callbacks are bounded by it.
    pub fn set_interval(&mut self, interval: Duration) {
        if self.reported != Some(interval) {
            *self = Self::new(interval);
            self.reported = Some(self.interval);
        }
    }

    /// The window is on a display refreshing every `interval`, which may
    /// be another than before even at the same rate: nothing fitted to the
    /// last one holds.
    pub fn moved(&mut self, interval: Duration) {
        *self = Self::new(interval);
        self.reported = Some(self.interval);
    }

    /// The display's own vsync source ticked (ADR-0029). A time the display
    /// is behind, predicted or presented, is a vblank: it is the phase, and
    /// what was fitted to acquires is dropped. A synthesized time is a
    /// timer's and sets nothing, so no timer wake can enter the phase
    /// through here. The interval it reports is taken when it is off by
    /// more than [`CALLBACK_TOLERANCE`], within what the monitor reports as
    /// a callback's is. The same tick given twice counts once.
    pub fn ticked(&mut self, tick: &gui_display::Tick) {
        let Some(at) = tick.target.of_the_display() else { return };
        if self
            .told
            .is_some_and(|(output, number, _)| output == tick.output && number == tick.sequence)
        {
            return;
        }
        if let Some(i) = tick.interval.filter(|i| CALLBACK_GAPS_IN.contains(i))
            && (i.as_secs_f64() / self.interval.as_secs_f64() - 1.0).abs() > CALLBACK_TOLERANCE
            && self
                .reported
                .is_none_or(|r| i >= r.mul_f64(1.0 - CALLBACK_TOLERANCE) && i <= r.mul_f64(CALLBACK_SLOWEST))
        {
            self.interval = i;
        }
        self.samples.clear();
        self.phase = Some(at);
        self.told = Some((tick.output, tick.sequence, tick.seen));
    }

    /// An acquire returned at `at` after waiting `waited`.
    pub fn acquired(&mut self, at: Instant, waited: Duration) {
        if waited < VSYNC_BLOCKED {
            return;
        }
        // The display says its own phase: an acquire's return is a worse
        // sample of it (it comes when the compositor frees a drawable).
        if self
            .told
            .is_some_and(|(_, _, seen)| at.saturating_duration_since(seen) < TICK_FRESH)
        {
            return;
        }
        self.samples.push_back(at);
        if self.samples.len() > VSYNC_SAMPLES {
            self.samples.pop_front();
        }
        self.fit();
    }

    /// A Wayland frame callback came at `at`: the compositor repainted its
    /// output, a vblank as the window sees it, and a sample of the phase.
    /// `chained` when the window presented in the refresh before (from the
    /// callback before this one): the gap between the two is then the
    /// output's refresh, and the shortest of the last few such gaps sets
    /// the interval when it is off by more than [`CALLBACK_TOLERANCE`]
    /// (Wayland tells no window its output's refresh rate otherwise).
    pub fn called_back(&mut self, at: Instant, chained: bool) {
        if chained
            && let Some(last) = self.called
            && let Some(gap) = at.checked_duration_since(last)
            && CALLBACK_GAPS_IN.contains(&gap)
        {
            self.gaps.push_back(gap);
            if self.gaps.len() > CALLBACK_GAPS {
                self.gaps.pop_front();
            }
            if let Some(refresh) = self.refresh()
                && (refresh.as_secs_f64() / self.interval.as_secs_f64() - 1.0).abs() > CALLBACK_TOLERANCE
            {
                self.interval = refresh;
                self.fit();
            }
        }
        self.called = Some(at);
        self.called = Some(at);
        self.samples.push_back(at);
        if self.samples.len() > VSYNC_SAMPLES {
            self.samples.pop_front();
        }
        self.fit();
    }

    /// The refresh the callback gaps show: the shortest gap at least
    /// [`CALLBACK_AGREE`] gaps agree with (within [`CALLBACK_JITTER`]), so a
    /// callback late then one early is no shorter refresh, their mean; and
    /// within what the monitor reports, when it does: never faster, and
    /// never as slow as half its rate (a cadence of presents, not of the
    /// output). None before [`CALLBACK_GAPS_LEAST`] gaps.
    fn refresh(&self) -> Option<Duration> {
        if self.gaps.len() < CALLBACK_GAPS_LEAST {
            return None;
        }
        let mut gaps: Vec<f64> = self.gaps.iter().map(Duration::as_secs_f64).collect();
        gaps.sort_by(f64::total_cmp);
        let fitted = gaps.iter().find_map(|&g| {
            let near: Vec<f64> = gaps
                .iter()
                .copied()
                .filter(|&o| o >= g && o <= g * (1.0 + CALLBACK_JITTER))
                .collect();
            (near.len() >= CALLBACK_AGREE)
                .then(|| near.iter().sum::<f64>() / f64::from(u32::try_from(near.len()).unwrap_or(1)))
        })?;
        let fitted = Duration::from_secs_f64(fitted);
        match self.reported {
            Some(r) if fitted < r.mul_f64(1.0 - CALLBACK_TOLERANCE) || fitted > r.mul_f64(CALLBACK_SLOWEST) => None,
            _ => Some(fitted),
        }
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
        let next = if after >= phase {
            let n = (after.duration_since(phase).as_secs_f64() / i).floor() + 1.0;
            phase.checked_add(Duration::from_secs_f64(n * i))
        } else {
            // The phase is ahead (a display's forecast of a coming vblank,
            // ADR-0029): the vblanks between are whole intervals before it.
            let back = phase.duration_since(after).as_secs_f64();
            let whole = (back / i).floor();
            if whole < 1.0 {
                Some(phase)
            } else {
                phase.checked_sub(Duration::from_secs_f64(whole * i))
            }
        }
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

    #[test]
    fn wayland_frame_callbacks_a_refresh_apart_set_the_interval_and_the_phase() {
        // Told 60 Hz, on a 144 Hz output: callbacks that came a refresh
        // apart give the interval; one after the window presented nothing
        // for a while (not chained) gives none.
        let mut v = Vsync::new(Duration::from_micros(16_667));
        let refresh = Duration::from_micros(6_944);
        let t0 = Instant::now();
        let at = |n: u32| t0.checked_add(refresh.saturating_mul(n)).unwrap_or(t0);
        let told = Duration::from_micros(16_667);
        v.called_back(at(0), false);
        assert!(v.phase().is_some(), "a callback is a vblank");
        // A refresh the compositor skipped: two apart.
        v.called_back(at(2), true);
        v.called_back(at(3), true);
        assert_eq!(v.interval(), told, "too few gaps yet");
        v.called_back(at(9), false);
        assert_eq!(v.interval(), told, "not chained: no gap");
        v.called_back(at(10), true);
        assert_eq!(v.interval(), refresh, "the shortest of the gaps");
    }

    #[test]
    fn a_callback_late_then_early_is_no_faster_refresh_nor_beyond_what_the_monitor_reports() {
        let refresh = Duration::from_micros(16_667);
        let mut v = Vsync::new(refresh);
        let t0 = Instant::now();
        let after = |ms: f64| t0.checked_add(Duration::from_secs_f64(ms / 1000.0)).unwrap_or(t0);
        // 16.7 ms apart, but one callback 3 ms late: gaps 19.7 then 13.7.
        for (i, ms) in [0.0, 16.667, 36.333, 50.0, 66.667].into_iter().enumerate() {
            v.called_back(after(ms), i > 0);
        }
        assert_eq!(v.interval(), refresh, "the late one's short gap is jitter");
        // Reported at 60 Hz: callbacks 33 ms apart (a cadence of presents)
        // do not halve it.
        let mut v = Vsync::new(Duration::from_millis(10));
        v.set_interval(refresh);
        for n in 0..6_u32 {
            v.called_back(t0.checked_add(refresh.saturating_mul(2 * n)).unwrap_or(t0), n > 0);
        }
        assert_eq!(v.interval(), refresh, "bounded by the monitor's rate");
    }

    /// A scripted display with one output, a subscription to its ticks,
    /// and what plays its OS.
    fn display() -> (gui_display::Displays, gui_display::Script, gui_display::Ticks) {
        let (mut displays, script) = gui_display::Displays::scripted(|| {});
        script.plug(gui_display::Script::output(1, "built-in"));
        displays.poll();
        let id = displays.outputs().first().map(|o| o.id).unwrap();
        let ticks = displays.ticks(id).unwrap();
        (displays, script, ticks)
    }

    #[test]
    fn a_display_tick_is_the_phase_and_a_synthesized_one_never_is() {
        let (_displays, script, ticks) = display();
        let hz60 = Duration::from_nanos(16_666_667);
        let mut v = Vsync::new(hz60);
        let now = Instant::now();
        // A timer's time: the right rate, no phase.
        script.tick(1, gui_display::Stamp::Synthesized(now), Some(hz60));
        v.ticked(&ticks.latest().unwrap());
        assert_eq!(v.phase(), None, "a timer's wake is no vblank");
        // The display's forecast of a vblank, 5 ms from now.
        let vblank = now + Duration::from_millis(5);
        script.tick(1, gui_display::Stamp::Predicted(vblank), Some(hz60));
        let tick = ticks.latest().unwrap();
        v.ticked(&tick);
        assert_eq!(v.phase(), Some(vblank));
        assert_eq!(v.next_after(now), vblank);
        assert_eq!(v.next_after(vblank), vblank + hz60);
        // What presentation feedback measured is one too.
        script.tick(1, gui_display::Stamp::Presented(vblank + hz60), None);
        v.ticked(&ticks.latest().unwrap());
        assert_eq!(v.phase(), Some(vblank + hz60));
    }

    #[test]
    fn while_the_display_ticks_blocked_acquires_do_not_move_the_phase() {
        let (_displays, script, ticks) = display();
        let hz60 = Duration::from_nanos(16_666_667);
        let mut v = Vsync::new(hz60);
        let now = Instant::now();
        // Acquires fitted a phase before the first tick came.
        v.acquired(now, Duration::from_millis(2));
        assert_eq!(v.phase(), Some(now));
        let vblank = now + Duration::from_millis(5);
        script.tick(1, gui_display::Stamp::Predicted(vblank), None);
        let tick = ticks.latest().unwrap();
        v.ticked(&tick);
        // An acquire that returned 3 ms off the display's phase.
        v.acquired(tick.seen + Duration::from_millis(8), Duration::from_millis(2));
        assert_eq!(v.phase(), Some(vblank), "the display's own phase stands");
        // The same tick read again at the next frame changes nothing.
        v.ticked(&tick);
        assert_eq!(v.phase(), Some(vblank));
        // The source stopped (the window was at rest): after a second the
        // acquires are the phase again, alone.
        let later = tick.seen + TICK_FRESH + Duration::from_millis(3);
        v.acquired(later, Duration::from_millis(2));
        assert_eq!(v.phase(), Some(later));
    }

    #[test]
    fn a_tick_sets_the_interval_within_what_the_monitor_reports() {
        let (_displays, script, ticks) = display();
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let now = Instant::now();
        // Nothing reported: the display link's own period is taken.
        let mut v = Vsync::new(hz60);
        script.tick(1, gui_display::Stamp::Predicted(now), Some(hz120));
        v.ticked(&ticks.latest().unwrap());
        assert_eq!(v.interval(), hz120);
        // Reported at 60 Hz: a tick claiming twice the rate is not it.
        let mut v = Vsync::new(hz60);
        v.set_interval(hz60);
        script.tick(1, gui_display::Stamp::Predicted(now), Some(hz120));
        v.ticked(&ticks.latest().unwrap());
        assert_eq!(v.interval(), hz60);
        // Moved to another display at the same rate: nothing fitted holds.
        assert!(v.phase().is_some());
        v.moved(hz60);
        assert_eq!(v.phase(), None);
    }
}
