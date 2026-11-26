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
//!
//! The calendar's time ([`wall_time`]: what day it is) is the clock's time
//! from a wall epoch: the system's when the app starts (taken again at a
//! frame boundary if the system's clock jumped, as a sleep or a time sync
//! does), a fixed day ([`script_epoch`]) once a script takes the clock, or
//! what a test sets ([`set_wall_time`]). While a frame is built it is the
//! one sample taken as the frame began ([`begin_frame`], until
//! [`end_frame`]), so the live frame and its cold twin never disagree
//! across midnight; outside a frame (an input handler, a timer, an effect
//! between frames) it is sampled when asked, so an app idle across
//! midnight names the new day at once. It is for the calendar alone: it
//! can step back when the system's clock is set back.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The wall time a script starts the clock at: 2026-06-15T12:00:00Z.
#[must_use]
pub fn script_epoch() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_781_524_800))
        .unwrap_or(UNIX_EPOCH)
}

/// How far the system's clock may differ from the frame clock's reckoning
/// before it is taken to have jumped (a sleep, a time sync).
const WALL_JUMP: Duration = Duration::from_secs(1);

struct Timer {
    /// When it runs, on the clock.
    due: Duration,
    id: u64,
    /// Its work, kept in the window current when it was set: it runs in
    /// that window, whichever window's turn runs it, and never once that
    /// window closed (ADR-0015, [`crate::engine::Stored`]).
    run: crate::engine::Stored<Box<dyn Fn()>>,
    /// What owned it when it was made: its call runs as that owner, so a
    /// timer it sets again (an autoplay) is owned, and cancelled, too.
    owner: Option<crate::reactive::OwnerId>,
}

/// Run `timer`'s work, in the window and as the owner it was set in,
/// through the gate, under `all`: one that panics leaves the others to run.
fn run_timer(timer: &Timer, all: &mut crate::engine::Gather) {
    all.run(|| {
        timer.run.call(|f| crate::reactive::run_owned(timer.owner, f));
    });
}

/// `window` closes: the timers set in it are cancelled (ADR-0015, the
/// close order), dropped once out of the queue, each under a catch, in
/// its close turn. How many were.
pub(crate) fn cancel_window(window: &crate::focus::FocusRoot) -> usize {
    let gone: Vec<Timer> = TIMERS
        .try_with(|t| {
            t.try_borrow_mut()
                .map(|mut t| {
                    let (gone, kept) = std::mem::take(&mut t.timers)
                        .into_iter()
                        .partition(|x| x.run.is_of(window));
                    t.timers = kept;
                    gone
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let n = gone.len();
    for timer in gone {
        crate::engine::contained(move || drop(timer));
    }
    n
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

/// `at` as nanoseconds on the frame clock (real time, from any thread):
/// exact at any uptime.
#[must_use]
pub fn nanos(at: Instant) -> u64 {
    u64::try_from(at.saturating_duration_since(start()).as_nanos()).unwrap_or(u64::MAX)
}

/// The time on the clock now, nanoseconds: real time, or where a script
/// set it ([`current_time`]).
#[must_use]
pub fn current_nanos() -> u64 {
    u64::try_from(current().as_nanos()).unwrap_or(u64::MAX)
}

/// A moment on the frame clock as a caller has it. The compositor's
/// physics (resampling, flings, scrollbar fades) keeps every instant as
/// exact nanoseconds and takes only spans between two of them as seconds
/// ([`span`]), as Chromium's `TimeTicks` and `TimeDelta` (int64
/// microseconds) and Flutter's `Duration` do: an f32 of seconds since the
/// clock started steps by 1/64 s at three days of uptime, which no
/// physics survives. Seconds as `f32` remain accepted for scripted times
/// near the clock's start (tests), where they are exact enough.
pub trait Moment: Copy {
    /// Nanoseconds on the frame clock.
    fn nanos(self) -> u64;
}

impl Moment for u64 {
    fn nanos(self) -> u64 {
        self
    }
}

impl Moment for Duration {
    fn nanos(self) -> u64 {
        u64::try_from(self.as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Moment for Instant {
    fn nanos(self) -> u64 {
        nanos(self)
    }
}

/// Seconds as a script gives them (a test's `s.at(1.0)`, a frame at 0.016):
/// an input-only conversion, to nanoseconds as they come in. Nothing
/// stores an f32 instant (frame time, motion, the compositor's physics,
/// the caret's blink, a double click all keep u64 nanoseconds and take
/// spans of them), and nothing outside tests and scenarios passes one: an
/// f32 of seconds since the clock started is exact only near its start.
impl Moment for f32 {
    fn nanos(self) -> u64 {
        nanos_of(self)
    }
}

/// The shortest span a rate is measured over, seconds (Chromium's
/// `kResampleMinDelta`): events closer than input is sampled measure no
/// rate (two events of one instant, a scenario's, measure none). Both
/// times are of one source (ADR-0032: no time is compared across
/// sources).
pub const MIN_RATE_SPAN: f32 = 0.002;

/// `amount` a second, over the span from `from` to `to`: `None` under
/// [`MIN_RATE_SPAN`] (or backwards). Every rate made of times (a speed,
/// a fling's, a prediction's) is made here.
#[must_use]
pub fn rate(amount: (f32, f32), from: u64, to: u64) -> Option<(f32, f32)> {
    let over = span(from, to);
    (over >= MIN_RATE_SPAN).then(|| (amount.0 / over, amount.1 / over))
}

/// The span from `from` to `to` (nanoseconds on the frame clock),
/// seconds; negative when `to` is earlier. Taken from two exact instants,
/// in f64, then narrowed: small, so exact enough as an f32 at any uptime.
#[must_use]
pub fn span(from: u64, to: u64) -> f32 {
    let s = if to >= from {
        Duration::from_nanos(to.saturating_sub(from)).as_secs_f64()
    } else {
        -Duration::from_nanos(from.saturating_sub(to)).as_secs_f64()
    };
    crate::convert::f64_to_f32(s)
}

/// `seconds` (a small span, from [`span`]'s side) as nanoseconds; 0 for
/// a negative one.
#[must_use]
pub fn span_nanos(seconds: f32) -> u64 {
    nanos_of(seconds)
}

/// The instant `seconds` (a span, negative before) from `at`.
#[must_use]
pub fn shifted(at: u64, seconds: f32) -> u64 {
    if seconds >= 0.0 {
        at.saturating_add(nanos_of(seconds))
    } else {
        at.saturating_sub(nanos_of(-seconds))
    }
}

/// `nanos` on the frame clock as seconds (what motion is sampled in).
#[must_use]
pub fn seconds_of(nanos: u64) -> f32 {
    Duration::from_nanos(nanos).as_secs_f32()
}

/// `seconds` on the frame clock as nanoseconds (a time given in seconds).
#[must_use]
pub fn nanos_of(seconds: f32) -> u64 {
    let d = Duration::try_from_secs_f32(seconds.max(0.0)).unwrap_or_default();
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

/// The moment `nanos` on the frame clock is (real time).
#[must_use]
pub fn instant_of(nanos: u64) -> Instant {
    start().checked_add(Duration::from_nanos(nanos)).unwrap_or_else(start)
}

/// The moment `seconds` on the frame clock is (real time).
#[must_use]
pub fn instant(seconds: f32) -> Instant {
    let d = Duration::try_from_secs_f32(seconds.max(0.0)).unwrap_or_default();
    start().checked_add(d).unwrap_or_else(start)
}

crate::engine::engine_local! {
    /// The time frames read, fixed per frame so one frame is consistent.
    /// Nanoseconds on the frame clock ([`Moment`]).
    [Fork] static FRAME_TIME: Cell<Option<u64>> = const { Cell::new(None) };
    /// Where a script has set the clock, once one drives it.
    [Fork] static SCRIPTED: Cell<Option<Duration>> = const { Cell::new(None) };
    /// The wall time at the clock's zero, once taken.
    [Fork] static WALL_EPOCH: Cell<Option<SystemTime>> = const { Cell::new(None) };
    /// The wall time of the frame being built, sampled as it began.
    [Fork] static FRAME_WALL: Cell<Option<SystemTime>> = const { Cell::new(None) };
    [Fork] static WANTS_FRAME: Cell<bool> = const { Cell::new(false) };
    /// A cold frame's timers are its own; the live ones it can only see
    /// ([`Queue`]).
    [Fork(cold_enter, cold_drain, cold_exit)] static TIMERS: RefCell<Queue> = RefCell::new(Queue::default());
}

/// The timers set, numbered.
static NEXT_TIMER: crate::engine::IdKind = crate::engine::IdKind::new("timer");

/// The timers as the live registry holds them, with no id: what V2
/// compares between two engines (review r18, decision 6).
#[cfg(test)]
pub(crate) fn registry_summary() -> String {
    TIMERS.with(|q| {
        let q = q.borrow();
        format!("timers {} set aside {}", q.timers.len(), q.live.is_some())
    })
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
fn cold_drain(_: &dyn std::any::Any) -> Vec<Box<dyn std::any::Any>> {
    // One box per timer (ADR-0034, S9): each its own drop.
    let timers = TIMERS.with(|t| std::mem::take(&mut t.borrow_mut().timers));
    let mut out = Vec::with_capacity(timers.len());
    for t in timers {
        crate::engine::whole(t, &mut out);
    }
    out
}

/// The live timers moved back (ADR-0034, S1); the check's were drained.
#[cfg(debug_assertions)]
fn cold_exit(_: Box<dyn std::any::Any>) {
    TIMERS.switch_with(|t| {
        let mut q = t.borrow_mut();
        let live = q.live.take().map(|(live, _)| live).unwrap_or_default();
        let left = std::mem::replace(&mut q.timers, live);
        if !left.is_empty() {
            crate::engine::quarantined("TIMERS", Box::new(left));
        }
    });
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
    seconds_of(frame_nanos())
}

/// The frame's time, exact nanoseconds on the frame clock: what motion
/// and the compositor sample at, by spans of it ([`span`]).
#[must_use]
pub fn frame_nanos() -> u64 {
    FRAME_TIME.with(Cell::get).unwrap_or_else(current_nanos)
}

/// Fix the clock for the frame about to be built, to be presented at `at`
/// on the clock (a scripted clock's frames are at its time).
pub fn begin_frame(at: impl Moment) {
    FRAME_TIME.with(|f| f.set(Some(at.nanos())));
    if !scripted() {
        // The system's clock jumped (a sleep, a time sync): reckon from it.
        let now = current();
        let real = SystemTime::now();
        let reckoned = wall_at(now);
        let off = real.duration_since(reckoned).unwrap_or_else(|e| e.duration());
        if off > WALL_JUMP {
            WALL_EPOCH.with(|w| w.set(Some(real.checked_sub(now).unwrap_or(real))));
        }
    }
    FRAME_WALL.with(|w| w.set(Some(wall_at(current()))));
}

/// The frame built since [`begin_frame`] is done: the wall time is sampled
/// when asked again.
pub fn end_frame() {
    FRAME_WALL.with(|w| w.set(None));
}

/// The wall time when the clock reads `at`: the epoch, taken from the
/// system's now the first time it is asked for.
fn wall_at(at: Duration) -> SystemTime {
    let epoch = WALL_EPOCH.with(Cell::get).unwrap_or_else(|| {
        let real = SystemTime::now();
        let epoch = real.checked_sub(current()).unwrap_or(real);
        WALL_EPOCH.with(|w| w.set(Some(epoch)));
        epoch
    });
    epoch.checked_add(at).unwrap_or(epoch)
}

/// What day and time it is, as a calendar shows it: the wall time of the
/// frame being built (the same for the live frame and its cold twin), or
/// of now outside a frame. Read it, never the system's clock, so that a
/// scripted run names the same day every time.
#[must_use]
pub fn wall_time() -> SystemTime {
    FRAME_WALL.with(Cell::get).unwrap_or_else(|| wall_at(current()))
}

/// Set the wall time to `at`, now: the clock's time moves it on from here.
/// Under a script that is a fixed day; outside one it is no pin (the
/// clock runs on, and a frame boundary takes the system's time again if
/// the two have parted).
pub fn set_wall_time(at: SystemTime) {
    WALL_EPOCH.with(|w| w.set(Some(at.checked_sub(current()).unwrap_or(at))));
    // A frame being built reads the new time too.
    FRAME_WALL.with(|w| {
        if w.get().is_some() {
            w.set(Some(at));
        }
    });
}

/// Whether something read the clock since the last call; clears the flag.
pub fn take_frame_request() -> bool {
    WANTS_FRAME.with(|w| w.replace(false))
}

/// Let a script drive the clock from here on, starting at `at`: time then
/// moves only by [`advance_to`]. Timers already pending keep the delay they
/// have left. No order of input and writes is read off this clock
/// (ADR-0032), so setting it back reorders nothing.
pub fn script(at: Duration) {
    // The wall time carries over as the clock is moved to `at`; the first
    // script starts it at the fixed day.
    let wall = if scripted() { wall_at(current()) } else { script_epoch() };
    start_script(at, wall);
}

/// [`script`], on the fixed day ([`script_epoch`]) whatever the wall time
/// was: a Scenario's, a scheduler's or a snapshot's clock, which inherits
/// no day an earlier one on the thread left.
pub fn script_fresh(at: Duration) {
    start_script(at, script_epoch());
}

fn start_script(at: Duration, wall: SystemTime) {
    let was = current();
    WALL_EPOCH.with(|w| w.set(Some(wall.checked_sub(at).unwrap_or(wall))));
    FRAME_WALL.with(|w| w.set(None));
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
    // As [`run_due_timers`]: a timer that panics leaves the others due on
    // the way to run, and the clock reaches `to`.
    let mut all = crate::engine::Gather::new();
    let mut ran = false;
    while let Some(timer) = take_due(to) {
        set_scripted(timer.due.max(current()));
        run_timer(&timer, &mut all);
        ran = true;
    }
    set_scripted(to);
    all.finish();
    ran
}

fn set_scripted(at: Duration) {
    SCRIPTED.with(|s| s.set(Some(at)));
    // What a timer's work reads as "now" (an animation it starts) is its own time.
    FRAME_TIME.with(|f| f.set(Some(at.nanos())));
}

/// Remove and return the earliest timer due by `by`; in a fork, none. A
/// timer's callback is the outside world's (it writes what it captured,
/// which no rollback puts back), so a fork runs none, made before it or by
/// it, as a fork's `Sender::send` delivers nothing (ADR-0028, rule 5).
/// The live run serves the live timers as without the fork; the fork's own
/// go with it.
fn take_due(by: Duration) -> Option<Timer> {
    if crate::engine::forking() {
        return None;
    }
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
    // Never minted from state a rollback restores: a fork's timer id kept
    // past it names no live timer (ADR-0028, rule 3).
    let id = crate::engine::unique_id(&NEXT_TIMER);
    TIMERS.with(|t| {
        t.borrow_mut().timers.push(Timer {
            due: current().saturating_add(delay),
            id,
            run: crate::engine::Stored::new(crate::engine::Site::Timer, Box::new(f)),
            owner: crate::reactive::owner(),
        });
    });
    if crate::reactive::has_owner() {
        crate::reactive::on_release(move || cancel(TimerId(id)));
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

/// How many scheduled calls are pending (tests).
#[cfg(test)]
pub(crate) fn pending_timers() -> usize {
    TIMERS.with(|t| t.borrow().timers.len())
}

/// Drop a scheduled call that has not run yet.
pub fn cancel(timer: TimerId) {
    TIMERS.with(|t| t.borrow_mut().cancel(timer.0));
}

/// Run every timer that is due, earliest first; returns whether any ran.
/// Each timer's work is a caller's closure, reached through the one gate
/// ([`crate::engine::call_out`]) in the window it was set in, so one that
/// panics leaves the others due to run: inside a turn (ADR-0015, the turn
/// rule; the window's idle is one), its panic goes to the turn's arbiter;
/// outside one, the first goes on once all have run.
pub fn run_due_timers() -> bool {
    let now = current();
    let mut all = crate::engine::Gather::new();
    let mut ran = false;
    while let Some(timer) = take_due(now) {
        run_timer(&timer, &mut all);
        ran = true;
    }
    all.finish();
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
            *self = Self::reporting(interval);
        }
    }

    /// The window is on a display refreshing every `interval`, which may
    /// be another than before even at the same rate: nothing fitted to the
    /// last one holds.
    pub fn moved(&mut self, interval: Duration) {
        *self = Self::reporting(interval);
    }

    /// A display the platform reports refreshing every `interval`, with
    /// nothing fitted yet.
    #[must_use]
    pub fn reporting(interval: Duration) -> Self {
        let mut v = Self::new(interval);
        v.reported = Some(v.interval);
        v
    }

    /// The refresh the platform reported for the display, if it did.
    #[must_use]
    pub fn reported(&self) -> Option<Duration> {
        self.reported
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

    /// Whether the display itself said its phase within [`TICK_FRESH`] of
    /// `at` (its vsync source ticked, or a Wayland frame callback came): no
    /// acquire need block to sample it (ADR-0033).
    #[must_use]
    pub fn told_lately(&self, at: Instant) -> bool {
        let fresh = |seen: Instant| at.saturating_duration_since(seen) < TICK_FRESH;
        self.told.is_some_and(|(_, _, seen)| fresh(seen)) || self.called.is_some_and(fresh)
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
    use std::rc::Rc;

    /// Times a nanosecond apart, or closer than input is sampled, measure
    /// no rate; 10 ms apart, theirs.
    #[test]
    fn a_rate_is_measured_over_a_span_of_time_never_over_an_instant() {
        let at = 1_000_000_000;
        assert_eq!(rate((10.0, 0.0), at, at + 1), None, "one nanosecond");
        assert_eq!(rate((10.0, 0.0), at, at + 1_999_999), None, "under 2 ms");
        assert_eq!(rate((10.0, 0.0), at + 1, at), None, "backwards");
        let r = rate((10.0, -5.0), at, at + 10_000_000);
        assert!(
            r.is_some_and(|(x, y)| (x - 1_000.0).abs() < 0.1 && (y + 500.0).abs() < 0.1),
            "{r:?}"
        );
    }

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

    /// Codex 3 (F1 review 3): a timer id a fork handed out and kept past
    /// its rollback names no live timer: cancelling it leaves the timer
    /// set live after it.
    #[cfg(debug_assertions)]
    #[test]
    fn a_timer_id_a_fork_kept_cancels_no_live_timer() {
        script(Duration::ZERO);
        let kept = {
            let _cold = crate::engine::ColdFrame::begin();
            after(Duration::from_secs(1), || {})
        };
        let live = after(Duration::from_secs(1), || {});
        assert_ne!(kept, live);
        cancel(kept);
        assert!(is_pending(live), "the live timer stands");
        cancel_all();
    }

    /// Codex 6 (F1 review 4): a check that sets a zero-delay timer and runs
    /// the due ones, or moves the clock past a live one, runs no callback:
    /// what a callback writes is the outside world's, which no rollback
    /// puts back. The live timer runs live after it, once.
    #[cfg(debug_assertions)]
    #[test]
    fn a_check_runs_no_timer_callback() {
        script(Duration::ZERO);
        let ran = Rc::new(Cell::new(0_u32));
        let r = Rc::clone(&ran);
        let _live = after(Duration::from_millis(5), move || r.set(r.get() + 1));
        {
            let _cold = crate::engine::ColdFrame::begin();
            let r = Rc::clone(&ran);
            let _ = after(Duration::ZERO, move || r.set(r.get() + 10));
            assert!(!run_due_timers(), "no callback runs in a fork");
            assert!(!advance_to(Duration::from_secs(1)), "nor as the fork's clock moves");
            assert_eq!(ran.get(), 0);
        }
        assert_eq!(ran.get(), 0, "the cell is unchanged after the rollback");
        assert!(advance_to(Duration::from_millis(5)), "the live timer runs live");
        assert_eq!(ran.get(), 1, "once, and the fork's never");
        cancel_all();
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

    /// A script starts the wall time at the fixed day (the same every
    /// run), the scripted clock moves it on, a test can set it, and moving
    /// the script's start keeps the wall time where it was.
    #[test]
    fn a_script_starts_the_wall_time_at_a_fixed_day_and_moves_it_with_the_clock() {
        script(Duration::ZERO);
        assert_eq!(wall_time(), script_epoch());
        advance_to(Duration::from_secs(3600));
        assert_eq!(wall_time(), script_epoch() + Duration::from_secs(3600));
        script(Duration::from_secs(10));
        assert_eq!(
            wall_time(),
            script_epoch() + Duration::from_secs(3600),
            "re-scripting keeps the wall time"
        );
        let at = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        set_wall_time(at);
        assert_eq!(wall_time(), at);
        advance_to(Duration::from_secs(70));
        assert_eq!(wall_time(), at + Duration::from_secs(60));
    }

    /// The wall time is sampled as a frame begins: it does not move while
    /// the frame is built, a cold build of the frame reads the same, and
    /// the next frame samples again.
    #[cfg(debug_assertions)]
    #[test]
    fn the_wall_time_is_one_sample_per_frame_shared_by_the_cold_build() {
        begin_frame(0.0);
        let sampled = wall_time();
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(wall_time(), sampled, "fixed within the frame");
        let cold = crate::engine::ColdFrame::begin();
        assert_eq!(wall_time(), sampled, "the cold build reads the live frame's");
        drop(cold);
        end_frame();
        std::thread::sleep(Duration::from_millis(5));
        assert!(wall_time() > sampled, "outside a frame it is sampled when asked");
        begin_frame(0.03);
        assert!(wall_time() > sampled, "the next frame samples again");
    }

    /// An app idle across midnight: a handler between frames asks for the
    /// time and gets the new day, not the last frame's.
    #[test]
    fn between_frames_the_wall_time_is_read_when_asked() {
        script_fresh(Duration::ZERO);
        let before_midnight = script_epoch() + Duration::from_secs(12 * 3600 - 1);
        set_wall_time(before_midnight);
        begin_frame(0.0);
        assert_eq!(wall_time(), before_midnight);
        end_frame();
        advance_to(Duration::from_secs(10));
        assert_eq!(
            wall_time(),
            before_midnight + Duration::from_secs(10),
            "a handler after the idle names the new day"
        );
    }

    /// A fresh script goes back to the fixed day whatever the wall time
    /// was; a plain one keeps it.
    #[test]
    fn a_fresh_script_goes_back_to_the_fixed_day() {
        script_fresh(Duration::ZERO);
        advance_to(Duration::from_secs(3 * 86_400));
        script(Duration::ZERO);
        assert_eq!(wall_time(), script_epoch() + Duration::from_secs(3 * 86_400));
        script_fresh(Duration::ZERO);
        assert_eq!(wall_time(), script_epoch());
    }

    /// The system's clock jumping (a sleep, a time sync) is taken up at the
    /// next frame boundary: the wall time follows it again.
    #[test]
    fn a_jump_of_the_system_clock_is_taken_up_at_a_frame_boundary() {
        let day = Duration::from_secs(86_400);
        set_wall_time(SystemTime::now() - day);
        begin_frame(0.0);
        let off = SystemTime::now().duration_since(wall_time()).unwrap_or_default();
        assert!(off < Duration::from_secs(2), "re-synced to the system's: {off:?} off");
    }
}
