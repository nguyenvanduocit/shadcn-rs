//! Time as components see it: the frame clock that drives animation and the
//! timers that run work later (a tooltip's delay, a toast's dismissal).
//!
//! Reading [`now`] asks for another frame, so an animation keeps running for
//! as long as something reads the clock and stops when nothing does.
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
}

thread_local! {
    static START: Instant = Instant::now();
    /// The time frames read, fixed per frame so one frame is consistent.
    static FRAME_TIME: Cell<Option<f32>> = const { Cell::new(None) };
    /// Where a script has set the clock, once one drives it.
    static SCRIPTED: Cell<Option<Duration>> = const { Cell::new(None) };
    static WANTS_FRAME: Cell<bool> = const { Cell::new(false) };
    static TIMERS: RefCell<Vec<Timer>> = const { RefCell::new(Vec::new()) };
    static NEXT_TIMER: Cell<u64> = const { Cell::new(1) };
}

/// The time on the clock: since the app started, or where a script set it.
fn current() -> Duration {
    SCRIPTED.with(Cell::get).unwrap_or_else(|| START.with(Instant::elapsed))
}

/// Seconds since the app started, as of the frame being built; asks for the
/// next frame.
#[must_use]
pub fn now() -> f32 {
    WANTS_FRAME.with(|w| w.set(true));
    frame_time()
}

/// Ask for the next frame without reading the clock (something on screen
/// moves on its own time).
pub fn request_frame() {
    WANTS_FRAME.with(|w| w.set(true));
}

/// The frame's time without asking for another frame (painting reads it).
#[must_use]
pub fn frame_time() -> f32 {
    FRAME_TIME.with(Cell::get).unwrap_or_else(|| current().as_secs_f32())
}

/// Fix the clock for the frame about to be built.
pub fn begin_frame() {
    FRAME_TIME.with(|f| f.set(Some(current().as_secs_f32())));
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
        for timer in t.borrow_mut().iter_mut() {
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
        (timer.run)();
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
        let mut timers = t.borrow_mut();
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

/// Run `f` once after `delay`.
pub fn after(delay: Duration, f: impl Fn() + 'static) -> TimerId {
    let id = NEXT_TIMER.with(|n| {
        let id = n.get();
        n.set(id.wrapping_add(1));
        id
    });
    TIMERS.with(|t| {
        t.borrow_mut().push(Timer {
            due: current().saturating_add(delay),
            id,
            run: Rc::new(f),
        });
    });
    TimerId(id)
}

/// Drop every scheduled call, for a run that starts afresh (a scenario of
/// another page on the same thread).
pub fn cancel_all() {
    TIMERS.with(|t| t.borrow_mut().clear());
}

/// Drop a scheduled call that has not run yet.
pub fn cancel(timer: TimerId) {
    TIMERS.with(|t| t.borrow_mut().retain(|x| x.id != timer.0));
}

/// Run every timer that is due, earliest first; returns whether any ran.
pub fn run_due_timers() -> bool {
    let now = current();
    let mut ran = false;
    while let Some(timer) = take_due(now) {
        (timer.run)();
        ran = true;
    }
    ran
}

/// When the last timer due by `by` on the clock is due, if one is pending.
#[must_use]
pub fn last_due_by(by: Duration) -> Option<Duration> {
    TIMERS.with(|t| t.borrow().iter().map(|x| x.due).filter(|&due| due <= by).max())
}

/// When the next timer is due, in real time.
#[must_use]
pub fn next_deadline() -> Option<Instant> {
    let due = TIMERS.with(|t| t.borrow().iter().map(|x| x.due).min())?;
    START.with(|s| s.checked_add(due))
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
        assert!((now() - 3.2).abs() < 1e-6);
    }
}
