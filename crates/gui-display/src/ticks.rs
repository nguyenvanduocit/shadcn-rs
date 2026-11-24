//! Vsync ticks: what an output's vsync source last said, held where any
//! thread can read it or wait for the next.
//!
//! The OS calls back on a thread of its own, at display priority. The
//! callback converts the OS's time, stores one [`Tick`] under a short lock
//! and wakes whoever waits. A reader takes the latest ([`Ticks::latest`])
//! or blocks for the next ([`Ticks::wait`]). The one piece of the caller's
//! code that can run there is the owner's wake, and only when a source
//! asks to be stopped; it is never called under a lock of this crate's
//! ([`Asked`]), so a wake that blocks on a lock of the caller's cannot
//! close a cycle with one of ours.
//!
//! Starting and stopping the OS source belongs to the thread that owns the
//! [`crate::Displays`]. A reader says only whether it wants ticks
//! ([`Ticks::run`]); a source nobody wanted for [`IDLE`] is stopped at the
//! owner's next [`crate::Displays::poll`], and started again there when it
//! is wanted, so a window at rest wakes no thread at any vblank.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::{Error, OutputId};

/// A moment on the display's timeline, with how it is known. The three are
/// kept apart by type so that a consumer chooses what each may drive: a
/// phase is fitted to [`Stamp::Presented`] and [`Stamp::Predicted`] only,
/// never to a timer's wake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// The OS says a frame will reach the screen then: its own forecast of
    /// a coming vblank (macOS's display link output time).
    Predicted(Instant),
    /// The OS says a frame reached the screen then: presentation feedback,
    /// measured after the fact.
    Presented(Instant),
    /// No OS source: a time made up from a timer and the nominal refresh.
    /// It has the right rate and no phase.
    Synthesized(Instant),
}

impl Stamp {
    /// The moment, whatever its basis.
    #[must_use]
    pub fn at(self) -> Instant {
        match self {
            Self::Predicted(at) | Self::Presented(at) | Self::Synthesized(at) => at,
        }
    }

    /// The moment when the display itself is behind it (predicted or
    /// presented); none for a synthesized one.
    #[must_use]
    pub fn of_the_display(self) -> Option<Instant> {
        match self {
            Self::Predicted(at) | Self::Presented(at) => Some(at),
            Self::Synthesized(_) => None,
        }
    }
}

/// One vsync of one output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// The output it is of.
    pub output: OutputId,
    /// Counts this subscription's ticks from 1; a gap is ticks nobody read.
    pub sequence: u64,
    /// When the source's callback ran.
    pub seen: Instant,
    /// The vblank it tells of.
    pub target: Stamp,
    /// The refresh interval the source reported with it, when it did.
    pub interval: Option<Duration>,
    /// The output's mode it was taken in: it counts the changes of the
    /// output's refresh this subscription saw ([`Ticks::generation`]). A
    /// consumer that reset its timing for a new mode takes only ticks of
    /// that mode, not one that came before the change.
    pub generation: u64,
}

/// How long a source nobody wants keeps running before it is stopped: a
/// reader that sleeps between bursts of input does not pay a stop and a
/// start for each.
pub const IDLE: Duration = Duration::from_millis(500);

/// What the owner calls to have [`crate::Displays::poll`] run on its thread.
pub(crate) type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug)]
struct State {
    latest: Option<Tick>,
    sequence: u64,
    /// The output's nominal interval, as of the owner's last poll.
    interval: Duration,
    /// How many times the owner saw that change ([`Tick::generation`]).
    generation: u64,
    /// A reader wants ticks.
    wanted: bool,
    /// Since when nobody does.
    unwanted_since: Option<Instant>,
    /// The owner has the source started.
    started: bool,
    /// The owner was asked to stop it, and has not polled yet.
    asked: bool,
    /// The reader is gone.
    dropped: bool,
    /// The output is gone: no more ticks will come.
    ended: bool,
}

pub(crate) struct Shared {
    output: OutputId,
    state: Mutex<State>,
    ticked: Condvar,
    wake: Wake,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What a source writes its ticks into. Cheap to clone; the source may call
/// it from any thread.
#[derive(Clone)]
pub(crate) struct Feed(Arc<Shared>);

/// The owner's wake, owed by a source that asked to be stopped, and not
/// sent yet. It is sent by whoever holds it once that one holds no lock of
/// this crate's: the wake is the caller's code, which may block on a lock
/// of the caller's that its own thread holds while it stops a source,
/// which takes ours.
#[must_use = "a source asked to be stopped: send the wake once no lock is held"]
pub(crate) struct Asked(Option<Wake>);

impl Asked {
    /// Call the owner's wake, if it is owed.
    pub(crate) fn send(self) {
        if let Some(wake) = self.0 {
            wake();
        }
    }

    /// Whether a wake is owed (what a callback that may not allocate at
    /// every vblank keeps).
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn owed(&self) -> bool {
        self.0.is_some()
    }
}

impl Feed {
    /// A vsync: `target`, with the interval the source reported, read in
    /// mode `generation` ([`Self::mode`], read before the time was). A time
    /// read before the owner moved the output to another mode is of the
    /// old grid, and is dropped. What it returns is sent by the caller once
    /// it holds no lock ([`Asked`]).
    pub(crate) fn tick(&self, target: Stamp, interval: Option<Duration>, generation: u64) -> Asked {
        let now = Instant::now();
        let ask = {
            let mut s = self.0.lock();
            // A callback already on its way when the source was stopped, or
            // a time read in the mode before the last change.
            if !s.started || s.ended || generation != s.generation {
                return Asked(None);
            }
            s.sequence = s.sequence.saturating_add(1);
            s.latest = Some(Tick {
                output: self.0.output,
                sequence: s.sequence,
                seen: now,
                target,
                interval,
                generation: s.generation,
            });
            let idle = !s.wanted
                && s.unwanted_since
                    .is_some_and(|t| now.saturating_duration_since(t) >= IDLE);
            let ask = idle && !s.asked;
            s.asked |= ask;
            ask
        };
        self.0.ticked.notify_all();
        Asked(ask.then(|| Arc::clone(&self.0.wake)))
    }

    /// The output's nominal interval and its mode's generation as of the
    /// owner's last poll, read together: a source reads this before it
    /// reads or makes a time, and ticks with the generation it read.
    pub(crate) fn mode(&self) -> (Duration, u64) {
        let s = self.0.lock();
        (s.interval, s.generation)
    }

    /// A vsync whose time is known now, in the mode current now (a
    /// scripted source's).
    pub(crate) fn tick_now(&self, target: Stamp, interval: Option<Duration>) -> Asked {
        let (_, generation) = self.mode();
        self.tick(target, interval, generation)
    }
}

/// What the owner is to do with a source ([`Link::settle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settle {
    Keep,
    Start,
    Stop,
    /// The reader is gone: stop it and forget it.
    Close,
}

/// The owner's side of a subscription.
pub(crate) struct Link(Arc<Shared>);

impl Link {
    pub(crate) fn output(&self) -> OutputId {
        self.0.output
    }

    pub(crate) fn feed(&self) -> Feed {
        Feed(Arc::clone(&self.0))
    }

    /// Decide what the source is to do now, and record it as done. The
    /// caller acts after this returns, so no lock is held while the OS
    /// starts or stops a source: the OS's callback takes this lock, maybe
    /// under locks of the OS's own, which a start or a stop may take too.
    pub(crate) fn settle(&self, now: Instant) -> Settle {
        let mut s = self.0.lock();
        s.asked = false;
        if s.dropped || s.ended {
            let was = s.started;
            s.started = false;
            return if was || s.dropped { Settle::Close } else { Settle::Keep };
        }
        let idle = !s.wanted
            && s.unwanted_since
                .is_some_and(|t| now.saturating_duration_since(t) >= IDLE);
        if s.wanted && !s.started {
            s.started = true;
            Settle::Start
        } else if idle && s.started {
            s.started = false;
            Settle::Stop
        } else {
            Settle::Keep
        }
    }

    /// The output is gone, or its source failed: no more ticks will come.
    pub(crate) fn end(&self) {
        self.0.lock().ended = true;
        self.0.ticked.notify_all();
    }

    /// The output's refresh changed to `interval` (a poll reported it):
    /// ticks from now on are of the next generation, and a timer source
    /// ticks at the new interval.
    pub(crate) fn remode(&self, interval: Duration) {
        let mut s = self.0.lock();
        s.interval = interval;
        s.generation = s.generation.saturating_add(1);
    }

    /// Make the reader look unwanted since `at` (tests of the idle stop).
    #[cfg(test)]
    pub(crate) fn unwanted_since(&self, at: Instant) {
        let mut s = self.0.lock();
        s.wanted = false;
        s.unwanted_since = Some(at);
    }
}

/// A subscription to one output's vsync. Any thread may hold and read it.
/// Dropping it has its source stopped at the owner's next poll.
pub struct Ticks(Arc<Shared>);

impl std::fmt::Debug for Ticks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ticks")
            .field("output", &self.0.output)
            .finish_non_exhaustive()
    }
}

/// A subscription to an output refreshing every `interval`, and its
/// owner's side, wanted from the start.
pub(crate) fn subscription(output: OutputId, interval: Duration, wake: Wake) -> (Ticks, Link) {
    let shared = Arc::new(Shared {
        output,
        state: Mutex::new(State {
            latest: None,
            sequence: 0,
            interval,
            generation: 0,
            wanted: true,
            unwanted_since: None,
            started: false,
            asked: false,
            dropped: false,
            ended: false,
        }),
        ticked: Condvar::new(),
        wake,
    });
    (Ticks(Arc::clone(&shared)), Link(shared))
}

impl Ticks {
    /// The output whose vsync this is.
    #[must_use]
    pub fn output(&self) -> OutputId {
        self.0.output
    }

    /// The last tick, if one came.
    #[must_use]
    pub fn latest(&self) -> Option<Tick> {
        self.0.lock().latest
    }

    /// The first tick after number `after`, waiting up to `timeout` for it;
    /// none when the time passes or the output is gone.
    #[must_use]
    pub fn wait(&self, after: u64, timeout: Duration) -> Option<Tick> {
        let until = Instant::now().checked_add(timeout);
        let mut s = self.0.lock();
        loop {
            if let Some(t) = s.latest.filter(|t| t.sequence > after) {
                return Some(t);
            }
            if s.ended {
                return None;
            }
            let left = until.map_or(timeout, |u| u.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return None;
            }
            s = self
                .0
                .ticked
                .wait_timeout(s, left)
                .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
        }
    }

    /// Say whether ticks are wanted. Unwanted, the source keeps running for
    /// [`IDLE`] and is then stopped; wanted again after that, the owner is
    /// woken to start it, and until it has, [`Self::latest`] is the tick
    /// from before the stop.
    pub fn run(&self, wanted: bool) {
        let wake = {
            let mut s = self.0.lock();
            if s.wanted == wanted {
                return;
            }
            s.wanted = wanted;
            s.unwanted_since = (!wanted).then(Instant::now);
            wanted && !s.started && !s.ended
        };
        if wake {
            (self.0.wake)();
        }
    }

    /// Whether the output is gone, or its source failed to start: no more
    /// ticks will come. A new subscription tries the source again.
    #[must_use]
    pub fn ended(&self) -> bool {
        self.0.lock().ended
    }

    /// The output's mode as of the owner's last poll: what the ticks of
    /// that mode carry ([`Tick::generation`]).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.lock().generation
    }
}

impl Drop for Ticks {
    fn drop(&mut self) {
        self.0.lock().dropped = true;
        (self.0.wake)();
    }
}

/// A vsync source, owned by the thread that owns the displays.
pub(crate) trait Source {
    /// Start it. On an error nothing of it runs and nothing is counted: a
    /// later start tries afresh.
    fn start(&mut self) -> Result<(), Error>;
    /// Stop it: its feed gets no more ticks once this returns. A backend
    /// whose OS refuses the stop keeps what the OS says is running, and
    /// tries again ([`crate::Backend::tidy`]).
    fn stop(&mut self);
}

/// A source with no display behind it: a thread that ticks with
/// [`Stamp::Synthesized`] times at the output's nominal interval, read
/// afresh before each tick ([`Link::remode`]). For an output whose OS gives
/// no vsync, and for tests.
pub(crate) struct Timer {
    running: Arc<(Mutex<Option<bool>>, Condvar)>,
}

impl Timer {
    pub(crate) fn spawn(feed: Feed) -> std::io::Result<Self> {
        let running = Arc::new((Mutex::new(Some(false)), Condvar::new()));
        let theirs = Arc::clone(&running);
        std::thread::Builder::new()
            .name("gui-display timer".to_owned())
            .spawn(move || {
                let (lock, changed) = &*theirs;
                let mut next = Instant::now();
                loop {
                    {
                        let mut on = lock.lock().unwrap_or_else(PoisonError::into_inner);
                        loop {
                            match *on {
                                None => return,
                                Some(true) => break,
                                Some(false) => on = changed.wait(on).unwrap_or_else(PoisonError::into_inner),
                            }
                        }
                    }
                    // The interval and its mode, read together before the
                    // time is made: a tick made from an interval the owner
                    // has since replaced carries the old mode, and is dropped.
                    let (interval, generation) = feed.mode();
                    let interval = interval.max(Duration::from_millis(1));
                    let now = Instant::now();
                    // Behind by more than an interval (it was stopped, or late): start from now.
                    next = match next.checked_add(interval) {
                        Some(n) if n > now => n,
                        _ => now.checked_add(interval).unwrap_or(now),
                    };
                    std::thread::sleep(next.saturating_duration_since(now));
                    // This thread holds no lock here.
                    feed.tick(Stamp::Synthesized(next), Some(interval), generation).send();
                }
            })?;
        Ok(Self { running })
    }

    fn set(&self, to: Option<bool>) {
        let (lock, changed) = &*self.running;
        *lock.lock().unwrap_or_else(PoisonError::into_inner) = to;
        changed.notify_all();
    }
}

impl Source for Timer {
    fn start(&mut self) -> Result<(), Error> {
        self.set(Some(true));
        Ok(())
    }

    fn stop(&mut self) {
        self.set(Some(false));
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.set(None);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn counted() -> (Wake, Arc<AtomicUsize>) {
        let n = Arc::new(AtomicUsize::new(0));
        let theirs = Arc::clone(&n);
        let wake: Wake = Arc::new(move || {
            theirs.fetch_add(1, Ordering::SeqCst);
        });
        (wake, n)
    }

    fn output() -> OutputId {
        OutputId(7)
    }

    /// A subscription to a 60 Hz output.
    fn sub(wake: Wake) -> (Ticks, Link) {
        subscription(output(), Duration::from_nanos(16_666_667), wake)
    }

    #[test]
    fn a_tick_is_read_with_its_basis_and_counted() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        assert_eq!(ticks.latest(), None);
        let at = Instant::now();
        link.feed()
            .tick_now(Stamp::Predicted(at), Some(Duration::from_millis(8)))
            .send();
        link.feed().tick_now(Stamp::Presented(at), None).send();
        let t = ticks.latest().unwrap();
        assert_eq!(
            (t.output, t.sequence, t.target, t.interval, t.generation),
            (output(), 2, Stamp::Presented(at), None, 0)
        );
        // The output's mode changed: later ticks are of the next one.
        link.remode(Duration::from_millis(8));
        assert_eq!(ticks.generation(), 1);
        link.feed().tick_now(Stamp::Predicted(at), None).send();
        assert_eq!(ticks.latest().map(|t| t.generation), Some(1));
        assert_eq!(Stamp::Synthesized(at).of_the_display(), None);
        assert_eq!(Stamp::Predicted(at).of_the_display(), Some(at));
    }

    #[test]
    fn a_tick_from_a_stopped_source_is_dropped() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        // Not started yet: a callback cannot have come from it.
        link.feed().tick_now(Stamp::Predicted(Instant::now()), None).send();
        assert_eq!(ticks.latest(), None);
    }

    #[test]
    fn a_time_read_before_a_mode_change_is_dropped() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        let feed = link.feed();
        // A source reads the mode, then its time; meanwhile the owner polls
        // and moves the output to another mode.
        let (_, read) = feed.mode();
        link.remode(Duration::from_millis(8));
        feed.tick(Stamp::Predicted(Instant::now()), None, read).send();
        assert_eq!(ticks.latest(), None, "a time of the old grid is no tick");
        let (_, read) = feed.mode();
        feed.tick(Stamp::Predicted(Instant::now()), None, read).send();
        assert_eq!(ticks.latest().map(|t| t.generation), Some(1));
    }

    #[test]
    fn wait_returns_the_next_tick_or_none_in_time() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        assert_eq!(ticks.wait(0, Duration::from_millis(5)), None);
        let feed = link.feed();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            feed.tick_now(Stamp::Predicted(Instant::now()), None).send();
        });
        let got = ticks.wait(0, Duration::from_secs(5)).unwrap();
        assert_eq!(got.sequence, 1);
        t.join().unwrap();
        assert_eq!(
            ticks.wait(1, Duration::from_millis(5)),
            None,
            "nothing after the first yet"
        );
    }

    #[test]
    fn an_unwanted_source_stops_only_after_the_idle_time_and_starts_when_wanted() {
        let (wake, woken) = counted();
        let (ticks, link) = sub(wake);
        let start = Instant::now();
        assert_eq!(link.settle(start), Settle::Start);
        ticks.run(false);
        assert_eq!(link.settle(Instant::now()), Settle::Keep, "not idle long enough");
        assert_eq!(woken.load(Ordering::SeqCst), 0);
        // Wanted again within the idle time: nothing to do, nobody woken.
        ticks.run(true);
        assert_eq!(woken.load(Ordering::SeqCst), 0);
        ticks.run(false);
        let later = Instant::now() + IDLE + Duration::from_millis(1);
        assert_eq!(link.settle(later), Settle::Stop);
        assert_eq!(link.settle(later), Settle::Keep);
        // Wanted while stopped: the owner is woken to start it.
        ticks.run(true);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        assert_eq!(link.settle(later), Settle::Start);
    }

    #[test]
    fn the_callback_asks_the_owner_once_to_stop_an_idle_source() {
        let (wake, woken) = counted();
        let (_ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        link.unwanted_since(Instant::now().checked_sub(IDLE + Duration::from_millis(1)).unwrap());
        let feed = link.feed();
        let asked = feed.tick_now(Stamp::Predicted(Instant::now()), None);
        assert_eq!(woken.load(Ordering::SeqCst), 0, "not woken from inside the tick");
        assert!(asked.owed());
        asked.send();
        assert!(!feed.tick_now(Stamp::Predicted(Instant::now()), None).owed());
        assert_eq!(woken.load(Ordering::SeqCst), 1, "asked once, not at every vblank");
        assert_eq!(link.settle(Instant::now()), Settle::Stop);
    }

    #[test]
    fn dropping_the_reader_closes_the_source() {
        let (wake, woken) = counted();
        let (ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        drop(ticks);
        assert_eq!(woken.load(Ordering::SeqCst), 1);
        assert_eq!(link.settle(Instant::now()), Settle::Close);
    }

    #[test]
    fn an_ended_output_wakes_its_waiter_with_nothing() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        link.end();
        assert!(ticks.ended());
        assert_eq!(ticks.wait(0, Duration::from_secs(5)), None);
        assert_eq!(link.settle(Instant::now()), Settle::Close);
    }

    #[test]
    fn a_timer_ticks_synthesized_times_at_its_interval_and_stops() {
        let (wake, _) = counted();
        let (ticks, link) = sub(wake);
        let interval = Duration::from_millis(4);
        link.remode(interval);
        let mut timer = Timer::spawn(link.feed()).unwrap();
        assert_eq!(link.settle(Instant::now()), Settle::Start);
        timer.start().unwrap();
        let a = ticks.wait(0, Duration::from_secs(5)).unwrap();
        let b = ticks.wait(a.sequence, Duration::from_secs(5)).unwrap();
        assert!(matches!(a.target, Stamp::Synthesized(_)));
        assert_eq!(b.interval, Some(interval));
        assert!(b.target.at() > a.target.at());
        timer.stop();
        let last = ticks.latest().unwrap().sequence;
        std::thread::sleep(Duration::from_millis(30));
        assert!(
            ticks.latest().unwrap().sequence <= last + 1,
            "at most the tick in flight"
        );
    }
}
