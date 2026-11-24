//! The render thread (ADR-0015): owns the GPU, the compositor's scene,
//! property trees and atlases, paced by the display's vsync. Each frame it
//! waits for a vblank, predicts when what it draws now will be presented,
//! hands the UI thread a [`MainFrame`] for that time when the UI asked for
//! one, waits for the commit until a deadline, and draws: the commit if it
//! came, else a compositor-only frame with scroll and motion moved on. It
//! never waits for the UI past the deadline, and sleeps on its channel when
//! nothing moves.
//!
//! It survives what would freeze the window (ADR-0015's invariant 7). A lost
//! device (a driver reset, a GPU removed) is made again on the same surface,
//! and the scene as committed is drawn whole from the UI's `Arc`s it keeps.
//! A panic drops everything it held: the device and the compositor are made
//! afresh and the UI is asked for a full commit, deltas sent before it
//! dropped. Chromium recovers from a lost GPU process the same way (a new
//! `LayerTreeFrameSink`, then a full commit). A panic that comes back three
//! times in ten seconds is the scene's, not the GPU's: the app closes,
//! saying why.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{Commit, Compositor, MainFrame, Presented};
use crate::clock::{self, Vsync};
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::scene::{Drawn, Held, Renderer, Scene};
use crate::text::Glyphs;

/// What the render thread is told.
pub(crate) enum ToRender {
    /// A UI frame, and what is drawn over it from now on.
    Commit(Box<Commit>, Option<Scene>),
    /// Replace what is drawn over the frames.
    Overlay(Option<Scene>),
    /// The UI has a frame to run: grant it a [`MainFrame`].
    NeedsFrame,
    /// The wheel at `point` (logical window pixels) moved `delta` at `at`:
    /// a trackpad's (`precise`) is resampled, a notched wheel's applied.
    /// `shown` is the frame on screen as it came: it is routed in that.
    Wheel {
        point: (f32, f32),
        delta: (f32, f32),
        at: Instant,
        precise: bool,
        began: bool,
        ended: bool,
        shown: Arc<Presented>,
    },
    /// The window is this many physical pixels now: its surface is
    /// configured to it at once, and shows the last frame at its own size
    /// until the UI commits one laid out at the new size.
    Resize((u32, u32)),
    /// The window is shown or hidden (occluded, minimized): hidden, nothing
    /// is drawn; shown again, the next frame is presented.
    Visible(bool),
    /// The OS asks for what the window shows again (it was exposed): the
    /// next frame presents it.
    Expose,
    /// The display refreshes this often.
    Interval(Duration),
    /// The window is on another output (ADR-0029), refreshing this often,
    /// with its vsync ticks where its OS has a source for them: nothing
    /// fitted to the last output holds.
    Output {
        interval: Duration,
        ticks: Option<gui_display::Ticks>,
    },
    /// The compositor's frame callback came (Wayland), at the time given:
    /// the next frame may be drawn. Callbacks come at the output's refresh,
    /// so they time its vsync ([`Vsync::called_back`]).
    FrameCallback(Instant),
    /// Read the last frame back, `overlay` over it.
    Capture(Option<Scene>, Sender<Option<Pixels>>),
    /// Fault injection (tests, the stress check): lose the GPU device as a
    /// driver reset would, or panic as a bug in the render thread would.
    LoseDevice,
    Panic,
    /// Nothing but a wake: the device was lost while the thread slept
    /// ([`crate::gpu::GpuRenderer::wake_on_lost`]), and it is to be made
    /// again and drawn.
    Wake,
    Quit,
}

/// What the render thread tells the UI thread.
pub(crate) enum FromRender {
    Begin(MainFrame),
    /// A commit was drawn: what the renderer did and holds (boxed: the
    /// other messages are small).
    Drawn(Box<Feedback>),
    /// The render thread was made afresh and holds nothing: the next commit
    /// must carry everything.
    Lost,
    /// The render thread cannot go on, and why.
    Failed(String),
    /// A commit applied, emptied: the UI's next frame writes into it.
    Spare(Box<Commit>),
    /// A wheel event over a wheel handler, its place in its gesture
    /// (`step`), `delta` as the UI's `Input::WheelAt` takes it, and the
    /// frame shown as it came: the UI routes it from there, and may
    /// consume it.
    Wheel {
        step: super::WheelStep,
        delta: (f32, f32),
        shown: Arc<Presented>,
        /// The node it was over in that frame.
        target: Option<super::HitTarget>,
    },
}

/// What the renderer did with a commit, for the UI's reports.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Feedback {
    pub drawn: Drawn,
    pub held: Held,
}

/// One frame the render thread drew, for measuring (ADR-0015's goals).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStat {
    /// The vblank it began at, and when it was to be presented.
    pub vsync: Instant,
    pub present_at: Instant,
    /// When drawing began (the commit came, or its deadline passed), and
    /// when it was submitted.
    pub drawing: Instant,
    pub submitted: Instant,
    /// The commit it applied this frame, if one.
    pub commit: Option<u64>,
    /// The last commit it shows.
    pub shows: u64,
    /// How long the UI took from its main frame to the commit drawn, if one.
    pub ui_took: Option<Duration>,
    /// When the UI was granted the main frame whose commit it drew, if one.
    pub granted: Option<Instant>,
    /// Its compositor-only part: drawn while the UI owed a commit.
    pub without_ui: bool,
    /// It made a lost device again before drawing (idle recovery, not a
    /// capture's).
    pub renewed: bool,
}

/// How the render thread waits for a vblank.
pub(crate) enum Pace {
    /// A window's surface: the acquire blocks until the frame presented
    /// last is shown, at a vblank. A frame begun with a texture held (the
    /// last one presented nothing) has nothing to wait for, and is paced
    /// by the vsync as fitted.
    Surface {
        /// Wayland: the window whose compositor's frame callback is waited
        /// for after each present, which stops drawing while it is hidden.
        callbacks: Option<Arc<dyn FrameCallbacks>>,
    },
    /// No display: a clock ticking at the given interval (tests, stress).
    Ticker(Duration),
}

/// A Wayland window's frame callbacks: one is asked for with each present,
/// and the compositor answers it at its output's next repaint, or never
/// while the window is hidden.
pub(crate) trait FrameCallbacks: Send + Sync {
    /// Ask for one with the present about to be made.
    fn before_present(&self);
    /// Have the callback delivered (as `RedrawRequested`) when it comes.
    fn after_present(&self);
}

impl FrameCallbacks for winit::window::Window {
    fn before_present(&self) {
        self.pre_present_notify();
    }

    fn after_present(&self) {
        self.request_redraw();
    }
}

/// How many frames in a row may present nothing on Wayland before one
/// presents anyway: a frame callback is asked for only with a present
/// (winit commits no surface without one), and a hidden window is known
/// only by its callback not coming. A window animating with nothing
/// changing (a hold) thus asks the compositor every few frames, and stops
/// drawing, asleep until its callback, once hidden.
const WAYLAND_PROBE: u32 = 8;

/// How long an occluded window waits before its surface is asked again,
/// with no event saying it can be shown.
const OCCLUDED_PROBE: Duration = Duration::from_secs(1);

/// The render thread's state.
pub(crate) struct RenderThread {
    compositor: Compositor,
    gpu: GpuRenderer,
    rx: Receiver<ToRender>,
    to_ui: Box<dyn Fn(FromRender) + Send>,
    /// What the platform thread stamps input with.
    presented: Arc<Mutex<Arc<Presented>>>,
    vsync: Vsync,
    pace: Pace,
    /// The UI asked for a main frame and has none.
    wants: bool,
    /// The main frame granted and not yet answered, and by when.
    owed: Option<(u64, Instant)>,
    sequence: u64,
    /// Something came that is not drawn yet (a commit, a scroll, an
    /// overlay, an expose): news, which a frame drawn for a vblank already
    /// tried may still be drawn for.
    dirty: bool,
    /// A present was owed and its texture could not be taken: retried at
    /// the next vblank, no news.
    retry: bool,
    visible: bool,
    /// The surface said the window cannot be shown (wgpu's `Occluded`):
    /// nothing is tried until it is shown or exposed, or this time comes
    /// (a slow probe, which is no frame).
    occluded_until: Option<Instant>,
    /// The size the window was last told to be, not yet configured: only
    /// the last of a drag's resizes configures the surface.
    resize_to: Option<(u32, u32)>,
    /// The vblank a main frame granted while the thread slept is for: the
    /// frame drawn next is drawn for it too, whatever its acquire took.
    granted_for: Option<Instant>,
    /// Wayland: the frame callback came since the last present, and when.
    called_back: bool,
    callback_at: Option<Instant>,
    log: Option<Arc<Mutex<Vec<FrameStat>>>>,
    /// The commit applied in the frame being drawn.
    applied: Option<u64>,
    /// The ticker's last vblank.
    ticked: Option<Instant>,
    /// The vsync ticks of the output the window is on, where its OS has a
    /// source for them (ADR-0029): wanted while frames are drawn, not while
    /// the thread sleeps, so a window at rest wakes nothing at a vblank.
    ticks: Option<gui_display::Ticks>,
    /// The vblank the last frame that presented, or took a texture, was
    /// drawn for: no later frame is drawn for it or an earlier one.
    reserved: Option<Instant>,
    /// The vblank the last frame was drawn for, when it presented nothing
    /// and took no texture: drawn for again only for something new before
    /// its deadline (a wheel, a commit), else the next frame is drawn for
    /// the vblank after it, not again and again for the same.
    blank: Option<Instant>,
    /// Wayland: frames in a row that presented nothing ([`WAYLAND_PROBE`]),
    /// and whether the last present answered the callback before it within
    /// its refresh, not a probe (the next callback then comes one refresh
    /// after that one: a sample of the interval).
    unpresented: u32,
    chained: bool,
    /// When the main frame owed was granted, and how long the UI took to
    /// answer the last one.
    granted: Option<Instant>,
    ui_took: Option<Duration>,
    /// When the main frame of the last commit was granted.
    answered: Option<Instant>,
    /// How long the UI took to answer its last main frames.
    ui_times: VecDeque<Duration>,
    /// The last frames' cost after their deadline: waking late, and drawing.
    costs: VecDeque<Duration>,
    /// Made afresh after a panic: commits sent before the UI knew are
    /// deltas against what was dropped, and only a full one is taken in.
    awaiting_full: bool,
    /// When it recovered from a panic, the last few times.
    recoveries: VecDeque<Instant>,
}

/// How many frames the draw cost is the worst of.
const COSTS: usize = 32;

/// The least time left to draw and submit a frame after its commit's
/// deadline; more when the last frames took more (a loaded machine wakes
/// the thread late), up to half the interval, which the UI keeps.
const DRAW_MARGIN: Duration = Duration::from_millis(3);
/// How long before a vblank a frame must be submitted to show at it.
const PRESENT_MARGIN: Duration = Duration::from_millis(1);
/// Slack over the worst recent cost.
const DRAW_SLACK: Duration = Duration::from_millis(1);
/// More panics than this within [`RECOVERY_WINDOW`] and it gives up.
const MAX_RECOVERIES: usize = 3;
const RECOVERY_WINDOW: Duration = Duration::from_secs(10);
/// How long to wait before asking again for a device that did not answer
/// (a GPU still resetting), and how many times to ask.
const REBUILD_RETRY: Duration = Duration::from_millis(100);
const REBUILD_TRIES: u32 = 50;

impl RenderThread {
    #[expect(clippy::too_many_arguments, reason = "everything the thread owns, handed over once")]
    pub(crate) fn new(
        compositor: Compositor,
        gpu: GpuRenderer,
        rx: Receiver<ToRender>,
        to_ui: Box<dyn Fn(FromRender) + Send>,
        presented: Arc<Mutex<Arc<Presented>>>,
        interval: Duration,
        pace: Pace,
        log: Option<Arc<Mutex<Vec<FrameStat>>>>,
    ) -> Self {
        Self {
            compositor,
            gpu,
            rx,
            to_ui,
            presented,
            vsync: Vsync::new(interval),
            pace,
            wants: false,
            owed: None,
            sequence: 0,
            dirty: false,
            retry: false,
            visible: true,
            occluded_until: None,
            resize_to: None,
            granted_for: None,
            called_back: true,
            callback_at: None,
            log,
            applied: None,
            ticked: None,
            ticks: None,
            reserved: None,
            blank: None,
            unpresented: 0,
            chained: false,
            granted: None,
            ui_took: None,
            answered: None,
            ui_times: VecDeque::new(),
            costs: VecDeque::new(),
            awaiting_full: false,
            recoveries: VecDeque::new(),
        }
    }

    /// Run until told to quit or the UI thread goes away, starting afresh
    /// after a panic (ADR-0015's invariant 7).
    pub(crate) fn run(mut self) {
        loop {
            match std::panic::catch_unwind(AssertUnwindSafe(|| self.serve())) {
                Ok(()) => return,
                Err(_) => {
                    if let Err(why) = self.recover() {
                        (self.to_ui)(FromRender::Failed(why));
                        return;
                    }
                }
            }
        }
    }

    /// Start afresh after a panic: a new device on the same surface, an
    /// empty compositor, and a full commit asked of the UI. Nothing is drawn
    /// until it comes, so the window keeps its last frame meanwhile.
    fn recover(&mut self) -> Result<(), String> {
        let now = Instant::now();
        self.recoveries
            .retain(|t| now.saturating_duration_since(*t) < RECOVERY_WINDOW);
        if self.recoveries.len() >= MAX_RECOVERIES {
            return Err(format!(
                "the render thread panicked {} times in {} s; giving up",
                MAX_RECOVERIES.saturating_add(1),
                RECOVERY_WINDOW.as_secs()
            ));
        }
        self.recoveries.push_back(now);
        self.compositor = Compositor::new(Glyphs::for_os());
        let mut tries = 0_u32;
        loop {
            let made = std::panic::catch_unwind(AssertUnwindSafe(|| self.gpu.rebuild()));
            match made {
                Ok(Ok(())) => break,
                Ok(Err(e)) if tries < REBUILD_TRIES => {
                    tries = tries.saturating_add(1);
                    eprintln!("render thread: no device yet ({e}); trying again");
                    std::thread::sleep(REBUILD_RETRY);
                }
                Ok(Err(e)) => return Err(format!("the render thread lost its device and got no other: {e}")),
                Err(_) => return Err("the render thread panicked making its device again".to_owned()),
            }
        }
        self.owed = None;
        self.granted = None;
        self.applied = None;
        self.dirty = false;
        self.retry = false;
        self.awaiting_full = true;
        // A main frame for the full commit.
        self.wants = true;
        (self.to_ui)(FromRender::Lost);
        Ok(())
    }

    /// A lost device made again, the committed scene to be drawn whole;
    /// `false` when no device answered yet (tried again next frame).
    fn ensure_device(&mut self) -> bool {
        if !self.gpu.lost() {
            return true;
        }
        match self.gpu.rebuild() {
            Ok(()) => {
                self.dirty = true;
                true
            }
            Err(e) => {
                eprintln!("render thread: the device was lost and none answers yet ({e})");
                std::thread::sleep(REBUILD_RETRY);
                false
            }
        }
    }

    /// The thread's loop, until told to quit or the UI thread goes away.
    fn serve(&mut self) {
        while self.step() {}
    }

    /// One turn of [`Self::serve`]: sleep until there is something to
    /// draw, take in what came, and draw a frame if one is due; `false` to
    /// quit.
    fn step(&mut self) -> bool {
        // Sleep on the channel unless something is to be drawn; an
        // occluded window until its slow probe.
        let occluded = self.occluded_until.filter(|t| *t > Instant::now());
        if occluded.is_none() {
            self.occluded_until = None;
        }
        let asleep = !self.busy() || !self.visible || !self.called_back || occluded.is_some();
        if asleep {
            if let Some(ticks) = &self.ticks {
                ticks.run(false);
            }
            let msg = match occluded {
                Some(t) => match self.rx.recv_timeout(t.saturating_duration_since(Instant::now())) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => None,
                    Err(RecvTimeoutError::Disconnected) => return false,
                },
                None => match self.rx.recv() {
                    Ok(msg) => Some(msg),
                    Err(_) => return false,
                },
            };
            if let Some(msg) = msg
                && !self.handle(msg)
            {
                return false;
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            if !self.handle(msg) {
                return false;
            }
        }
        // Input woke everything up: the UI starts its frame now, not
        // at the next vblank (cc's `BeginMainFrame` on demand).
        if asleep && self.wants && self.owed.is_none() && self.visible {
            self.take_tick();
            self.grant_now();
        }
        let hidden = self.occluded_until.is_some();
        !(self.busy() && self.visible && self.called_back && !hidden) || self.frame()
    }

    /// Want the display's ticks (a frame is about to be timed) and take the
    /// last one as the vsync's phase. A tick from before the source last
    /// stopped is taken once and then outlived by the acquires
    /// ([`Vsync::acquired`]) until the source, started again, ticks.
    fn take_tick(&mut self) {
        if let Some(ticks) = &self.ticks {
            ticks.run(true);
            if let Some(tick) = ticks.latest() {
                self.vsync.ticked(&tick);
            }
        }
    }

    /// Grant the UI a main frame now, the thread having slept: for the
    /// vblank the next frame is drawn for ([`Self::aim`]), which that frame
    /// keeps however long its acquire takes ([`Self::granted_for`]).
    fn grant_now(&mut self) {
        let now = Instant::now();
        let present = match &self.pace {
            Pace::Surface { .. } => {
                let p = self.aim(now);
                self.granted_for = Some(p);
                p
            }
            Pace::Ticker(i) => {
                let tick = self.next_tick(*i, now);
                tick.checked_add(*i).unwrap_or(tick)
            }
        };
        let deadline = present.checked_sub(self.margin()).filter(|d| *d > now).unwrap_or(now);
        self.grant(present, deadline);
    }

    /// Whether there is a frame to draw. A lost device is a reason of its
    /// own, read off the device rather than a flag a frame clears: lost
    /// while a frame drains the channel or waits for its commit (after the
    /// frame made sure of its device), the frame draws nothing and clears
    /// `dirty`, and the device is still to be made again and drawn whole
    /// before the thread sleeps.
    fn busy(&self) -> bool {
        let now = clock::seconds(Instant::now());
        self.dirty
            || self.retry
            || self.gpu.lost()
            || (self.wants && self.owed.is_none())
            || self.compositor.animating(now)
            || self.gpu.raster_pending()
    }

    /// Take in one message; `false` to quit.
    pub(super) fn handle(&mut self, msg: ToRender) -> bool {
        match msg {
            ToRender::Commit(mut commit, overlay) => {
                if self.owed.is_some_and(|(seq, _)| seq <= commit.answers) {
                    self.owed = None;
                    self.answered = self.granted.take();
                    self.ui_took = self.answered.map(|g| g.elapsed());
                    if let Some(t) = self.ui_took {
                        self.ui_times.push_back(t);
                        if self.ui_times.len() > COSTS {
                            self.ui_times.pop_front();
                        }
                    }
                }
                if self.awaiting_full && !commit.full {
                    // A delta against what a panic dropped.
                    commit.clear();
                    (self.to_ui)(FromRender::Spare(commit));
                    return true;
                }
                self.awaiting_full = false;
                self.applied = Some(commit.sequence);
                self.compositor.apply(&mut commit);
                (self.to_ui)(FromRender::Spare(commit));
                self.compositor.set_overlay(overlay);
                self.dirty = true;
            }
            ToRender::Overlay(o) => {
                self.compositor.set_overlay(o);
                self.dirty = true;
            }
            ToRender::NeedsFrame => self.wants = true,
            ToRender::Wheel {
                point,
                delta,
                at,
                precise,
                began,
                ended,
                shown,
            } => {
                let w = super::WheelInput {
                    point,
                    delta,
                    at: clock::seconds(at),
                    precise,
                    began,
                    ended,
                    seen: shown.frame,
                };
                let wheeled = self.compositor.wheel_input(w);
                if let super::Wheeled::Ui(target) = wheeled {
                    (self.to_ui)(FromRender::Wheel {
                        step: w.step(),
                        delta,
                        shown,
                        target,
                    });
                    return true;
                }
                if wheeled == super::Wheeled::Scrolled {
                    self.dirty = true;
                    // The UI takes the scroll in with its next frame.
                    self.wants = true;
                }
            }
            // Configured by the next frame, with only the last of those
            // that came meanwhile.
            ToRender::Resize(size) => {
                self.resize_to = Some(size);
                self.occluded_until = None;
                self.dirty = true;
            }
            ToRender::Visible(v) => {
                if v && !self.visible {
                    self.gpu.expose();
                    self.occluded_until = None;
                }
                self.visible = v;
                self.dirty |= v;
            }
            ToRender::Expose => {
                self.gpu.expose();
                self.occluded_until = None;
                self.dirty = true;
            }
            ToRender::Interval(i) => self.vsync.set_interval(i),
            ToRender::Output { interval, ticks } => {
                self.vsync.moved(interval);
                self.ticks = ticks;
            }
            ToRender::FrameCallback(at) => {
                self.called_back = true;
                self.callback_at = Some(at);
                self.vsync.called_back(at, std::mem::take(&mut self.chained));
            }
            ToRender::Capture(overlay, reply) => {
                let pixels = self
                    .ensure_device()
                    .then(|| self.compositor.capture(&mut self.gpu, overlay.as_ref()))
                    .flatten();
                reply.send(pixels).ok();
            }
            // Made again by the next frame, which `busy` asks for.
            ToRender::LoseDevice => self.gpu.lose(),
            // Woken: `busy` reads the loss off the device.
            ToRender::Wake => {}
            ToRender::Panic => {
                #[expect(
                    clippy::panic,
                    reason = "fault injection: a bug in the render thread, recovered from"
                )]
                {
                    panic!("render thread: panic injected");
                }
            }
            ToRender::Quit => return false,
        }
        true
    }

    /// Wait for the next vblank where there is one to wait for: when the
    /// frame begins, and how long the display held it there, when that is
    /// a sample of the vsync. Only a texture acquired, or a tick, is: a
    /// frame begun with a texture held, or whose acquire failed (a minimized
    /// window), waited for nothing, and its deadline paces it
    /// ([`Self::frame`]).
    fn wait_vsync(&mut self) -> (Instant, Option<Duration>) {
        match &self.pace {
            Pace::Surface { .. } => {
                if self.gpu.holds_frame() {
                    return (Instant::now(), None);
                }
                let waited = self.gpu.acquire();
                (Instant::now(), waited)
            }
            Pace::Ticker(interval) => {
                let interval = *interval;
                let now = Instant::now();
                let next = self.next_tick(interval, now);
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                self.ticked = Some(next);
                (next, Some(next.saturating_duration_since(now)))
            }
        }
    }

    /// The ticker's next tick after `now`, on its grid.
    fn next_tick(&self, interval: Duration, now: Instant) -> Instant {
        match self.ticked {
            Some(t) if t.checked_add(interval).is_some_and(|n| n > now) => t.checked_add(interval).unwrap_or(now),
            Some(t) => {
                // Missed ticks: the next one on the grid.
                let behind = now.duration_since(t).as_secs_f64() / interval.as_secs_f64();
                let n = behind.floor() + 1.0;
                t.checked_add(interval.mul_f64(n)).unwrap_or(now)
            }
            None => now,
        }
    }

    /// The longest the UI took to answer a main frame lately, the one it
    /// owes now included.
    fn ui_worst(&self) -> Duration {
        let owing = self.granted.map(|g| g.elapsed()).unwrap_or_default();
        self.ui_times.iter().copied().fold(owing, Duration::max)
    }

    /// The time kept to draw after a commit's deadline: the worst recent
    /// cost and a little, at least [`DRAW_MARGIN`], at most half the interval.
    fn margin(&self) -> Duration {
        let worst = self.costs.iter().max().copied().unwrap_or_default();
        let interval = self.vsync.interval();
        worst.saturating_add(DRAW_SLACK).clamp(
            DRAW_MARGIN,
            interval.checked_div(2).unwrap_or_default().max(DRAW_MARGIN),
        )
    }

    /// The vblank a frame begun at `at` on a window's surface is drawn
    /// for: the first it can still make, but never one a frame that
    /// presented or took a texture was drawn for. Only one present shows
    /// at a vblank, and a frame begun while one waits for it (its acquire
    /// then blocks until it shows) is shown at the vblank after. The UI's
    /// main frame is granted for what this gives too, so it samples its
    /// motion for the vblank its commit is drawn for.
    fn aim(&self, at: Instant) -> Instant {
        let present = self.vsync.present_time(at, PRESENT_MARGIN);
        match self.reserved {
            Some(r) if present <= r => self.vsync.next_after(r),
            _ => present,
        }
    }

    /// Take in what comes until `deadline`, or until something new to
    /// draw came (a commit, a scroll, a main frame asked for): whether it
    /// did (what came before, drained by [`Self::serve`], included), or
    /// `None` when told to quit. A present retried ([`Self::retry`]) is no
    /// news: it waits for the next vblank, not tried at once again and again.
    fn news_until(&mut self, deadline: Instant) -> Option<bool> {
        loop {
            if self.dirty || (self.wants && self.owed.is_none()) {
                return Some(true);
            }
            let now = Instant::now();
            if now >= deadline {
                return Some(false);
            }
            match self.rx.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        return None;
                    }
                }
                Err(RecvTimeoutError::Timeout) => return Some(false),
                Err(RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Hand the UI a main frame presented at `present`, its commit wanted by `deadline`.
    fn grant(&mut self, present: Instant, deadline: Instant) {
        self.sequence = self.sequence.saturating_add(1);
        let token = MainFrame {
            sequence: self.sequence,
            present_at: clock::seconds(present),
            deadline,
            interval: self.vsync.interval(),
            scroll: self.compositor.begin_main_frame(
                self.sequence,
                clock::seconds(Instant::now()),
                super::horizon(self.ui_worst(), self.vsync.interval()),
            ),
        };
        self.wants = false;
        self.owed = Some((self.sequence, deadline));
        self.granted = Some(Instant::now());
        (self.to_ui)(FromRender::Begin(token));
    }

    /// Draw one frame; `false` when the UI thread is gone.
    fn frame(&mut self) -> bool {
        let was_lost = self.gpu.lost();
        if !self.ensure_device() {
            return true;
        }
        let renewed = was_lost && !self.gpu.lost();
        if let Some(size) = self.resize_to.take() {
            self.gpu.resize(size);
        }
        // Wanted before the acquire blocks, so a source that stopped while
        // the thread slept is started meanwhile; read after it, for the
        // tick of the vblank the acquire waited for.
        if let Some(ticks) = &self.ticks {
            ticks.run(true);
        }
        let (vblank, waited) = self.wait_vsync();
        let woke = Instant::now();
        self.take_tick();
        if let Some(waited) = waited {
            self.vsync.acquired(vblank, waited);
        }
        let surface = matches!(self.pace, Pace::Surface { .. });
        // The window cannot be shown: nothing is drawn, and nothing more
        // is tried until it is shown, exposed or resized, or a second
        // passes. That try is no frame: nothing is counted for it.
        if surface && self.gpu.occluded() {
            self.occluded_until = woke.checked_add(OCCLUDED_PROBE);
            return true;
        }
        let acquired = surface && waited.is_some();
        // A main frame granted while the thread slept is for a vblank: the
        // frame is drawn for that one, so the UI's motion and the
        // compositor's sample the same time.
        let reserved = self.reserved;
        let granted = self.granted_for.take().filter(|g| reserved.is_none_or(|r| *g > r));
        let mut present = match (&self.pace, granted) {
            (Pace::Surface { .. }, Some(g)) => g,
            // The texture taken at a vblank shows at the next one.
            (Pace::Surface { .. }, None) => self.aim(vblank),
            (Pace::Ticker(i), _) => vblank.checked_add(*i).unwrap_or(vblank),
        };
        // A frame already drawn for this vblank, to no effect: drawn for it
        // again only for something new before its deadline (a wheel may
        // still make it), else for the next, so that frames that change
        // nothing are not drawn again and again for the same vblank.
        if surface
            && granted.is_none()
            && let Some(b) = self.blank.filter(|b| present <= *b)
        {
            let by = b.checked_sub(self.margin()).unwrap_or(b);
            match self.news_until(by) {
                None => return false,
                Some(true) => present = b,
                Some(false) => present = self.vsync.next_after(b),
            }
        }
        // Wayland: a frame that would present nothing presents anyway now
        // and then, to ask for a frame callback ([`WAYLAND_PROBE`]).
        let callbacks = match &self.pace {
            Pace::Surface { callbacks: Some(c) } => Some(Arc::clone(c)),
            _ => None,
        };
        let probe = callbacks.is_some() && self.unpresented >= WAYLAND_PROBE;
        if probe {
            self.gpu.expose();
        }
        let deadline = present
            .checked_sub(self.margin())
            .filter(|d| *d > vblank)
            .unwrap_or(vblank);
        // What arrived while waiting (a commit, the wheel).
        while let Ok(msg) = self.rx.try_recv() {
            if !self.handle(msg) {
                return false;
            }
        }
        if self.wants && self.owed.is_none() {
            self.grant(present, deadline);
        }
        // Wait for the commit owed until the deadline, then draw without it;
        // a main frame asked for meanwhile is granted at once for this
        // frame's present time (cc's `BeginMainFrame` sent inside the impl
        // frame). The wait ends at the deadline even with nothing owed, so a
        // main frame asked for by input arriving meanwhile is still shown at
        // this vblank: measured side by side with drawing at once (stress
        // `threads`), the frame rate was the same and input to present
        // shorter.
        while self.owed.is_some() || self.applied.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match self.rx.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        return false;
                    }
                    if self.wants && self.owed.is_none() && self.applied.is_none() {
                        self.grant(present, deadline);
                    }
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
        // The device was lost meanwhile (a message drained above, a driver
        // reset): nothing it holds draws. The frame is drawn whole on a
        // device made again, at once, as `busy` keeps the thread awake.
        if self.gpu.lost() {
            return true;
        }
        // Drawn without the commit the UI owes: a compositor-only frame.
        let without_ui = self.owed.is_some();
        let drawing = Instant::now();
        let at = clock::seconds(present);
        self.compositor.draw(&mut self.gpu, at);
        // A fling or a resampled gesture moved on: the UI takes it in, and
        // builds what it scrolls toward, with its next frame.
        self.wants |= self.compositor.scrolled();
        let drew = self.applied.take();
        let drawn = self.gpu.drawn();
        // Wayland: a frame callback answers a commit, so one is asked for
        // only when this frame presents.
        let presenting = self.gpu.presenting();
        if let Some(c) = callbacks.as_ref().filter(|_| presenting) {
            c.before_present();
        }
        let presents = self.gpu.presents();
        self.gpu.end_frame();
        let presented_now = self.gpu.presents() != presents;
        // Only a present made asks for a callback: one predicted and not
        // made (its texture could not be taken) leaves the thread free to
        // draw, not waiting for a callback that never comes.
        if let Some(c) = callbacks.filter(|_| presented_now) {
            c.after_present();
            // The next callback comes a refresh after the last only when
            // this present answers it within that refresh: not a probe,
            // nor one made after the window idled (a key repeat's).
            let now = Instant::now();
            self.chained = !probe
                && self.called_back
                && self
                    .callback_at
                    .is_some_and(|c| now.saturating_duration_since(c) < self.vsync.interval());
            self.called_back = false;
        }
        // The vblank this frame was drawn for is taken when it presented or
        // holds a texture taken for it; drawn to no effect, a later frame
        // may still be drawn for it.
        if presented_now || acquired {
            self.reserved = Some(present);
            self.blank = None;
            self.unpresented = 0;
        } else if surface {
            self.blank = Some(present);
            self.unpresented = self.unpresented.saturating_add(1);
        }
        let submitted = Instant::now();
        // What the frame cost after its deadline: waking late at the vblank
        // and drawing, or, drawn at the deadline, all from it to the submit
        // (waking late from the wait included).
        let late = woke.saturating_duration_since(vblank);
        let cost = late
            .saturating_add(submitted.saturating_duration_since(drawing))
            .max(submitted.saturating_duration_since(deadline));
        self.costs.push_back(cost);
        if self.costs.len() > COSTS {
            self.costs.pop_front();
        }
        // What input is stamped with is what the window shows: a frame
        // meant to present that could not leaves the last one there.
        let owed = surface && self.gpu.owes_present();
        let presented = self.compositor.presented();
        if !owed {
            *self.presented.lock().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&presented);
        }
        if let Some(log) = &self.log {
            log.lock().unwrap_or_else(PoisonError::into_inner).push(FrameStat {
                vsync: vblank,
                present_at: present,
                drawing,
                submitted,
                commit: drew,
                shows: presented.commit,
                ui_took: drew.and(self.ui_took),
                granted: drew.and(self.answered),
                without_ui,
                renewed,
            });
        }
        if drew.is_some() {
            (self.to_ui)(FromRender::Drawn(Box::new(Feedback {
                drawn,
                held: self.gpu.held(),
            })));
        }
        // A presentation whose texture could not be taken is still owed:
        // retried at the next vblank. All that came is drawn.
        self.dirty = false;
        self.retry = owed;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::Fault;

    /// A render thread on a headless device, ticking every millisecond,
    /// and the sender of its channel; `None` without a GPU.
    fn ticking() -> Option<(RenderThread, Sender<ToRender>)> {
        let gpu = GpuRenderer::headless().ok()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(|_| {}),
            Arc::new(Mutex::new(Arc::new(Presented::default()))),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        );
        Some((thread, tx))
    }

    /// A render thread paced by a window's surface (`gpu`'s, if any),
    /// refreshing every `interval`, logging its frames; and the sender of
    /// its channel.
    fn on_surface(
        gpu: GpuRenderer,
        interval: Duration,
    ) -> (RenderThread, Arc<Mutex<Vec<FrameStat>>>, Sender<ToRender>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let log = Arc::new(Mutex::new(Vec::new()));
        let thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(|_| {}),
            Arc::new(Mutex::new(Arc::new(Presented::default()))),
            interval,
            Pace::Surface { callbacks: None },
            Some(Arc::clone(&log)),
        );
        (thread, log, tx)
    }

    #[test]
    fn a_frame_whose_acquire_failed_is_no_vsync_sample() {
        // No window: every acquire fails, as a minimized one's may. The
        // frames wait for their deadlines, which is no vblank.
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, _tx) = on_surface(gpu, Duration::from_millis(4));
        for _ in 0..4 {
            assert!(thread.frame());
        }
        assert_eq!(thread.vsync.phase(), None, "no phase fitted to timer wakes");
    }

    #[test]
    fn an_outputs_ticks_are_the_phase_of_its_frames_and_unwanted_while_the_thread_sleeps() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, tx) = on_surface(gpu, Duration::from_millis(4));
        // Scripted displays play the OS (ADR-0029); `woken` counts the
        // times their owner is asked to poll.
        let woken = Arc::new(AtomicUsize::new(0));
        let theirs = Arc::clone(&woken);
        let (mut displays, os) = gui_display::Displays::scripted(move || {
            theirs.fetch_add(1, Ordering::SeqCst);
        });
        os.plug(gui_display::Script::output(1, "built-in"));
        displays.poll();
        let id = displays.outputs().first().map(|o| o.id).unwrap();
        let ticks = displays.ticks(id).unwrap();

        // Fitted to the last display, then told of this one.
        thread.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let interval = Duration::from_millis(8);
        assert!(thread.handle(ToRender::Output {
            interval,
            ticks: Some(ticks),
        }));
        assert_eq!(
            (thread.vsync.interval(), thread.vsync.phase()),
            (interval, None),
            "nothing fitted holds"
        );

        // A timer's tick is no phase; the display's forecast is, at the
        // next frame (whose acquire fails here: no window).
        os.tick(1, gui_display::Stamp::Synthesized(Instant::now()), None);
        assert!(thread.frame());
        assert_eq!(thread.vsync.phase(), None);
        let vblank = Instant::now() + Duration::from_millis(3);
        os.tick(1, gui_display::Stamp::Predicted(vblank), None);
        assert!(thread.frame());
        assert_eq!(thread.vsync.phase(), Some(vblank));

        // Nothing to draw: the thread sleeps, and no longer wants ticks.
        // After the idle time the source asks to be stopped, and is.
        thread.dirty = false;
        thread.retry = false;
        tx.send(ToRender::Wake).unwrap();
        assert!(thread.step());
        std::thread::sleep(gui_display::IDLE + Duration::from_millis(30));
        let before = woken.load(Ordering::SeqCst);
        os.tick(1, gui_display::Stamp::Predicted(Instant::now()), None);
        assert_eq!(
            woken.load(Ordering::SeqCst),
            before + 1,
            "an idle source asks its owner to stop it"
        );
        displays.poll();
        assert_eq!(os.running(1), 0, "a window at rest wakes nothing at a vblank");

        // A frame is drawn again: the ticks are wanted, and started.
        assert!(thread.frame());
        assert_eq!(woken.load(Ordering::SeqCst), before + 2);
        displays.poll();
        assert_eq!(os.running(1), 1);
    }

    #[test]
    fn frames_that_change_nothing_present_nothing_and_are_drawn_for_later_vblanks() {
        // The texture each frame takes at its start is kept when it changed
        // nothing; the next frame waits for no vblank then, but is still
        // drawn for a later one than the last, not again and again for the
        // same in a loop.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let interval = Duration::from_millis(4);
        let (mut thread, log, _tx) = on_surface(gpu, interval);
        // A phase fitted, as acquires that waited give it: a frame begun
        // at its deadline is then within the same vblank's margin.
        thread.vsync.acquired(Instant::now(), Duration::from_millis(2));
        for _ in 0..6 {
            assert!(thread.frame());
        }
        assert_eq!(thread.gpu.presents(), 1, "the first frame, then nothing changed");
        let frames = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
        assert_eq!(frames.len(), 6);
        for pair in frames.windows(2) {
            let [a, b] = pair else { continue };
            assert!(
                b.present_at > a.present_at,
                "each frame is drawn for a later vblank: {:?} then {:?}",
                a.present_at,
                b.present_at
            );
        }
    }

    /// Frames until one draws for a vblank to no effect: the first
    /// presents, the next takes a texture, the one after holds it.
    fn to_a_blank_frame(thread: &mut RenderThread) {
        for _ in 0..3 {
            assert!(thread.frame());
        }
        assert!(thread.blank.is_some(), "the last frame presented nothing");
    }

    #[test]
    fn a_frame_that_presented_nothing_leaves_its_vblank_to_what_comes_next() {
        // A frame drawn for a vblank to no effect does not take it: a wheel
        // or a commit arriving before its deadline is drawn for it still.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let interval = Duration::from_millis(50);
        let (mut thread, log, tx) = on_surface(gpu, interval);
        thread.vsync.acquired(Instant::now(), Duration::from_millis(2));
        to_a_blank_frame(&mut thread);
        // The frame of nothing drew early for the next vblank, which is
        // still a long way off.
        thread.vsync = Vsync::new(interval);
        thread.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let blank = Some(thread.aim(Instant::now()));
        thread.blank = blank;
        // Taken in by the thread's own turn, before its frame.
        tx.send(ToRender::Expose).ok();
        assert!(thread.step());
        let frames = log.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let last = frames.last().map(|f| f.present_at);
        assert!(last <= blank, "drawn for the vblank left free: {last:?}, {blank:?}");
        assert_eq!(thread.gpu.presents(), 2);
    }

    #[test]
    fn a_main_frame_is_granted_for_the_vblank_its_frame_is_drawn_for() {
        // Right after a frame that presented for vblank T, a frame begun
        // now is drawn for T + 1, and so is a main frame granted now
        // ([`RenderThread::aim`]), not T.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, _tx) = on_surface(gpu, Duration::from_millis(50));
        thread.vsync.acquired(Instant::now(), Duration::from_millis(2));
        assert!(thread.frame());
        let reserved = thread.reserved.expect("the frame presented");
        // Asked while T's present waits for it, in time for T otherwise.
        let early = reserved.checked_sub(Duration::from_millis(20)).unwrap_or(reserved);
        assert_eq!(
            thread.aim(early),
            thread.vsync.next_after(reserved),
            "granted for T + 1"
        );
    }

    #[test]
    fn a_present_whose_texture_timed_out_is_drawn_again_at_the_next_wake() {
        let Ok(mut gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        gpu.virtual_display().faults.extend([Fault::Timeout, Fault::Timeout]);
        let (mut thread, log, tx) = on_surface(gpu, Duration::from_millis(20));
        tx.send(ToRender::Expose).ok();
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 0, "no texture, at the start or when drawing");
        assert!(thread.retry && thread.busy(), "still owed: the thread stays awake");
        assert!(!thread.dirty, "a retry is no news");
        let failed = thread.blank.expect("the frame presented nothing");
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 1, "the next wake presents it");
        assert!(!thread.retry);
        let last = log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .map(|f| f.present_at);
        assert!(
            last.is_some_and(|l| l > failed),
            "retried at the next vblank, not at once for the same"
        );
    }

    #[test]
    fn a_present_that_failed_leaves_input_stamped_with_what_the_window_shows() {
        // A commit drawn and not presented (its texture could not be
        // taken) is not what the window shows: input stays stamped with the
        // frame that is, until the commit is presented.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, tx) = on_surface(gpu, Duration::from_millis(4));
        let shown = Arc::clone(&thread.presented);
        let commit = |sequence: u64, background: crate::color::Color| {
            let mut c = Commit::default();
            c.between(&Scene::default(), &Scene::new((64, 64), 1.0, Some(background)));
            c.sequence = sequence;
            c.full = true;
            ToRender::Commit(Box::new(c), None)
        };
        let stamped = || shown.lock().unwrap_or_else(PoisonError::into_inner).commit;
        tx.send(commit(1, crate::color::Color::WHITE)).ok();
        assert!(thread.step());
        assert_eq!(stamped(), 1);
        thread
            .gpu
            .virtual_display()
            .faults
            .extend([Fault::Timeout, Fault::Timeout]);
        tx.send(commit(2, crate::color::Color::BLACK)).ok();
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 1, "commit 2 drawn, not presented");
        assert_eq!(stamped(), 1, "input lands in commit 1, which the window shows");
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 2);
        assert_eq!(stamped(), 2, "presented: input lands in it");
    }

    #[test]
    fn an_occluded_window_draws_nothing_until_shown_or_a_slow_probe() {
        let Ok(mut gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        gpu.virtual_display().faults.push_back(Fault::Occluded);
        let (mut thread, log, tx) = on_surface(gpu, Duration::from_millis(4));
        tx.send(ToRender::Expose).ok();
        assert!(thread.step());
        assert!(thread.occluded_until.is_some(), "occluded: nothing more is tried");
        let frames = || log.lock().unwrap_or_else(PoisonError::into_inner).len();
        assert_eq!(frames(), 0, "the try is no frame");
        // Shown again: drawn at once.
        thread.gpu.virtual_display().faults.push_back(Fault::Occluded);
        tx.send(ToRender::Visible(false)).ok();
        tx.send(ToRender::Visible(true)).ok();
        assert!(thread.step());
        assert!(thread.occluded_until.is_some(), "occluded again");
        // No event: asked again after a second, and drawn.
        let asked = Instant::now();
        assert!(thread.step(), "asleep until the probe");
        assert!(thread.step(), "the probe's frame");
        assert!(asked.elapsed() >= OCCLUDED_PROBE.mul_f32(0.9), "{:?}", asked.elapsed());
        assert_eq!(thread.gpu.presents(), 1);
        assert_eq!(frames(), 1);
    }

    #[test]
    fn a_drags_resizes_configure_the_surface_once() {
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, tx) = on_surface(gpu, Duration::from_millis(4));
        assert!(thread.frame());
        let configured = thread.gpu.virtual_display().configures;
        for size in [(70, 64), (80, 70), (96, 80)] {
            thread.gpu.virtual_display().window = size;
            tx.send(ToRender::Resize(size)).ok();
        }
        assert!(thread.step());
        assert_eq!(
            thread.gpu.virtual_display().configures - configured,
            1,
            "the last size only"
        );
        assert_eq!(thread.gpu.virtual_display().presented.last(), Some(&(96, 80)));
    }

    #[test]
    fn a_main_frame_granted_while_asleep_is_for_the_vblank_its_frame_is_drawn_for() {
        // No vsync fitted yet, and the frame's acquire takes a while: the
        // frame is still drawn for the vblank the UI's frame was granted for.
        let Ok(mut gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        gpu.virtual_display().acquire_takes = Duration::from_millis(15);
        let granted = Arc::new(Mutex::new(Vec::new()));
        let (mut thread, log, tx) = on_surface(gpu, Duration::from_millis(10));
        let told = Arc::clone(&granted);
        thread.to_ui = Box::new(move |m| {
            if let FromRender::Begin(token) = m {
                told.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(token.present_at);
            }
        });
        tx.send(ToRender::NeedsFrame).ok();
        assert!(thread.step(), "granted at once");
        assert_eq!(thread.vsync.phase(), None);
        tx.send(ToRender::Expose).ok();
        assert!(thread.step());
        let at = granted.lock().unwrap_or_else(PoisonError::into_inner).first().copied();
        let drawn = log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .map(|f| clock::seconds(f.present_at));
        assert!(at.is_some() && at == drawn, "granted for {at:?}, drawn for {drawn:?}");
    }

    /// Frame callbacks asked for, as a Wayland window's.
    #[derive(Default)]
    struct Asked(std::sync::atomic::AtomicU32);

    impl FrameCallbacks for Asked {
        fn before_present(&self) {}
        fn after_present(&self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn a_wayland_window_that_presents_nothing_still_asks_for_a_callback_now_and_then() {
        // Frames that present nothing ask the compositor for nothing, and a
        // hidden window would never be known as one: every few, one
        // presents, and then the thread sleeps until its callback.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, _tx) = on_surface(gpu, Duration::from_millis(2));
        let asked = Arc::new(Asked::default());
        let callbacks: Arc<dyn FrameCallbacks> = {
            let a: Arc<Asked> = Arc::clone(&asked);
            a
        };
        thread.pace = Pace::Surface {
            callbacks: Some(callbacks),
        };
        let count = || asked.0.load(std::sync::atomic::Ordering::Relaxed);
        assert!(thread.frame());
        assert_eq!(count(), 1, "the first present asks for one");
        assert!(!thread.called_back, "and the thread waits for it");
        assert!(thread.handle(ToRender::FrameCallback(Instant::now())));
        // One takes a texture, then frames of nothing, then the probe.
        for _ in 0..WAYLAND_PROBE + 2 {
            assert!(thread.frame());
        }
        assert_eq!(
            thread.gpu.presents(),
            2,
            "one probe after {WAYLAND_PROBE} frames of nothing"
        );
        assert_eq!(count(), 2);
        assert!(!thread.called_back, "asleep until the compositor answers");
    }

    /// A Wayland-paced render thread refreshing every `interval` as told
    /// (no monitor rate reported), and the callbacks it asks for.
    fn on_wayland(interval: Duration) -> Option<(RenderThread, Arc<Asked>, Sender<ToRender>)> {
        let gpu = GpuRenderer::with_virtual_window((64, 64)).ok()?;
        let (mut thread, _, tx) = on_surface(gpu, interval);
        let asked = Arc::new(Asked::default());
        let callbacks: Arc<dyn FrameCallbacks> = {
            let a: Arc<Asked> = Arc::clone(&asked);
            a
        };
        thread.pace = Pace::Surface {
            callbacks: Some(callbacks),
        };
        Some((thread, asked, tx))
    }

    #[test]
    fn a_wayland_present_predicted_and_not_made_leaves_the_thread_free_to_draw() {
        // An overlay hidden is to present the frame without it; its texture
        // cannot be taken: no present, so no callback comes, and the
        // thread must not wait for one.
        let Some((mut thread, asked, tx)) = on_wayland(Duration::from_millis(4)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        tx.send(ToRender::Overlay(Some(Scene::new((64, 64), 1.0, None)))).ok();
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 1);
        assert!(thread.handle(ToRender::FrameCallback(Instant::now())));
        thread
            .gpu
            .virtual_display()
            .faults
            .extend([Fault::Timeout, Fault::Timeout]);
        tx.send(ToRender::Overlay(None)).ok();
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 1, "the texture could not be taken");
        assert!(thread.called_back, "no present made: no callback waited for");
        assert_eq!(asked.0.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(thread.step(), "drawn again");
        assert_eq!(thread.gpu.presents(), 2);
    }

    #[test]
    fn wayland_callbacks_after_key_repeats_do_not_set_the_refresh() {
        // 60 Hz. A key repeating at a third of that presents every third
        // refresh, long after the callback before: no refresh.
        let refresh = Duration::from_millis(16);
        let Some((mut thread, _, tx)) = on_wayland(refresh) else {
            eprintln!("no GPU: skipped");
            return;
        };
        for _ in 0..6 {
            tx.send(ToRender::Expose).ok();
            assert!(thread.step());
            assert!(!thread.called_back);
            // The compositor answers at its next refresh; the next key comes
            // two refreshes later.
            std::thread::sleep(refresh);
            assert!(thread.handle(ToRender::FrameCallback(Instant::now())));
            std::thread::sleep(refresh.saturating_mul(2));
        }
        assert_eq!(thread.vsync.interval(), refresh, "key repeats are no refresh");
    }

    #[test]
    fn wayland_callbacks_after_probes_do_not_set_the_refresh() {
        // Idle: only probes present, each answered at the next refresh.
        let fast = Duration::from_millis(3);
        let Some((mut thread, _, _tx)) = on_wayland(fast) else {
            return;
        };
        for _ in 0..(WAYLAND_PROBE + 2) * 5 {
            assert!(thread.frame());
            if !thread.called_back {
                std::thread::sleep(fast);
                assert!(thread.handle(ToRender::FrameCallback(Instant::now())));
            }
        }
        assert_eq!(thread.vsync.interval(), fast, "probes are no refresh");
    }

    #[test]
    fn wayland_callbacks_a_present_answers_at_once_set_the_refresh() {
        // Told 60 Hz on a faster output: a window presenting each refresh,
        // at once after each callback, has the callbacks a refresh apart.
        let Some((mut thread, _, tx)) = on_wayland(Duration::from_millis(16)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let refresh = Duration::from_millis(10);
        let start = Instant::now();
        for _ in 0..16 {
            tx.send(ToRender::Expose).ok();
            // Drawn at once: nothing is owed and nothing waits.
            thread.applied = Some(0);
            assert!(thread.step());
            // The compositor answers at its first refresh after the present
            // (on a loaded machine a frame may miss one: a gap of two).
            let n = start.elapsed().as_nanos() / refresh.as_nanos() + 1;
            let due = start
                .checked_add(refresh.saturating_mul(u32::try_from(n).unwrap_or(u32::MAX)))
                .unwrap_or(start);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            assert!(thread.handle(ToRender::FrameCallback(due)));
        }
        let fitted = thread.vsync.interval();
        assert!(
            fitted.abs_diff(refresh) < Duration::from_micros(500),
            "the refresh: {fitted:?}"
        );
    }

    #[test]
    fn a_device_lost_while_the_thread_sleeps_wakes_it_to_make_it_again() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        // As the app wires it: the lost callback sends a wake.
        let wake = tx.clone();
        gpu.wake_on_lost(move || {
            wake.send(ToRender::Wake).ok();
        });
        let device = gpu.device();
        let presented = Arc::new(Mutex::new(Arc::new(Presented::default())));
        let thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(|_| {}),
            Arc::clone(&presented),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        );
        let frames = || presented.lock().unwrap_or_else(PoisonError::into_inner).frame;
        let running = std::thread::spawn(move || thread.run());
        // Idle: nothing to draw, it sleeps on its channel.
        std::thread::sleep(Duration::from_millis(100));
        let before = frames();
        // The driver reports the loss through the device's own callback,
        // on this thread; no message is sent for it.
        device.destroy();
        device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let until = Instant::now()
            .checked_add(Duration::from_secs(10))
            .unwrap_or_else(Instant::now);
        while frames() == before && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let after = frames();
        tx.send(ToRender::Quit).ok();
        running.join().ok();
        assert!(
            after > before,
            "the loss woke the thread, which made its device again and drew"
        );
    }

    #[test]
    fn a_device_lost_while_a_frame_drains_its_channel_keeps_the_thread_awake_until_made_again() {
        let Some((mut thread, tx)) = ticking() else {
            eprintln!("no GPU: skipped");
            return;
        };
        assert!(!thread.busy(), "idle: it would sleep");
        // The loss arrives after the frame made sure of its device, while
        // it takes in what came during its vsync wait.
        tx.send(ToRender::LoseDevice).ok();
        assert!(thread.frame(), "the frame runs");
        assert!(thread.gpu.lost(), "the device the frame drew on is lost");
        assert!(thread.busy(), "a lost device keeps it from sleeping");
        // The next frame makes it again, and then it may sleep.
        assert!(thread.frame());
        assert!(!thread.gpu.lost(), "made again");
        assert!(!thread.busy(), "made again and drawn: it may sleep");
    }
}
