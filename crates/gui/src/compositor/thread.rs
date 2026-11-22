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
    Wheel {
        point: (f32, f32),
        delta: (f32, f32),
        at: Instant,
        precise: bool,
        ended: bool,
    },
    /// The window's surface is this many physical pixels now.
    Resize((u32, u32)),
    /// The window is shown or hidden (occluded, minimized): hidden, nothing is drawn.
    Visible(bool),
    /// The display refreshes this often.
    Interval(Duration),
    /// The compositor's frame callback came (Wayland): the next frame may be drawn.
    FrameCallback,
    /// Read the last frame back, `overlay` over it.
    Capture(Option<Scene>, Sender<Option<Pixels>>),
    /// Fault injection (tests, the stress check): lose the GPU device as a
    /// driver reset would, or panic as a bug in the render thread would.
    LoseDevice,
    Panic,
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
}

/// How the render thread waits for a vblank.
pub(crate) enum Pace {
    /// A window's surface: the acquire blocks until a texture is free.
    Surface {
        /// Wayland: wait for the compositor's frame callback before acquiring,
        /// which stops drawing while the surface is hidden.
        callbacks: bool,
        window: Arc<winit::window::Window>,
    },
    /// No display: a clock ticking at the given interval (tests, stress).
    Ticker(Duration),
}

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
    /// A commit was applied and not yet drawn.
    dirty: bool,
    visible: bool,
    /// Wayland: the frame callback came since the last present.
    called_back: bool,
    log: Option<Arc<Mutex<Vec<FrameStat>>>>,
    /// The commit applied in the frame being drawn.
    applied: Option<u64>,
    /// The ticker's last vblank.
    ticked: Option<Instant>,
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
            visible: true,
            called_back: true,
            log,
            applied: None,
            ticked: None,
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
        loop {
            // Sleep on the channel unless something is to be drawn.
            let asleep = !self.busy() || !self.visible || !self.called_back;
            if asleep {
                match self.rx.recv() {
                    Ok(msg) => {
                        if !self.handle(msg) {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
            while let Ok(msg) = self.rx.try_recv() {
                if !self.handle(msg) {
                    return;
                }
            }
            // Input woke everything up: the UI starts its frame now, not
            // at the next vblank (cc's `BeginMainFrame` on demand).
            if asleep && self.wants && self.owed.is_none() && self.visible {
                let now = Instant::now();
                let present = match &self.pace {
                    // A drawable is free: drawn as soon as the commit comes.
                    Pace::Surface { .. } => self.vsync.present_time(now, PRESENT_MARGIN),
                    Pace::Ticker(i) => {
                        let tick = self.next_tick(*i, now);
                        tick.checked_add(*i).unwrap_or(tick)
                    }
                };
                let deadline = present.checked_sub(self.margin()).filter(|d| *d > now).unwrap_or(now);
                self.grant(present, deadline);
            }
            if self.busy() && self.visible && self.called_back && !self.frame() {
                return;
            }
        }
    }

    /// Whether there is a frame to draw.
    fn busy(&self) -> bool {
        let now = clock::seconds(Instant::now());
        self.dirty || (self.wants && self.owed.is_none()) || self.compositor.animating(now) || self.gpu.raster_pending()
    }

    /// Take in one message; `false` to quit.
    fn handle(&mut self, msg: ToRender) -> bool {
        match msg {
            ToRender::Commit(commit, overlay) => {
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
                    return true;
                }
                self.awaiting_full = false;
                self.applied = Some(commit.sequence);
                self.compositor.apply(*commit);
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
                ended,
            } => {
                let moved = if precise {
                    self.compositor.gesture(point, delta, clock::seconds(at), ended)
                } else {
                    self.compositor.wheel(point, delta, clock::seconds(at))
                };
                if moved {
                    self.dirty = true;
                    // The UI takes the scroll in with its next frame.
                    self.wants = true;
                }
            }
            ToRender::Resize(size) => {
                self.gpu.resize(size);
                self.dirty = true;
            }
            ToRender::Visible(v) => {
                self.visible = v;
                self.dirty |= v;
            }
            ToRender::Interval(i) => self.vsync.set_interval(i),
            ToRender::FrameCallback => self.called_back = true,
            ToRender::Capture(overlay, reply) => {
                let pixels = self
                    .ensure_device()
                    .then(|| self.compositor.capture(&mut self.gpu, overlay.as_ref()))
                    .flatten();
                reply.send(pixels).ok();
            }
            ToRender::LoseDevice => {
                self.gpu.lose();
                self.dirty = true;
            }
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

    /// Wait for the next vblank: when it was, and how long that took.
    fn wait_vsync(&mut self) -> (Instant, Duration) {
        match &self.pace {
            Pace::Surface { .. } => {
                let asked = Instant::now();
                match self.gpu.acquire() {
                    Some(waited) => (Instant::now(), waited),
                    None => {
                        // No texture (a minimized window): wait as the display would.
                        let next = self.vsync.next_after(asked);
                        std::thread::sleep(next.saturating_duration_since(Instant::now()));
                        (Instant::now(), asked.elapsed())
                    }
                }
            }
            Pace::Ticker(interval) => {
                let interval = *interval;
                let now = Instant::now();
                let next = self.next_tick(interval, now);
                std::thread::sleep(next.saturating_duration_since(Instant::now()));
                self.ticked = Some(next);
                (next, next.saturating_duration_since(now))
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
        if !self.ensure_device() {
            return true;
        }
        let (vblank, waited) = self.wait_vsync();
        let woke = Instant::now();
        self.vsync.acquired(vblank, waited);
        let present = match &self.pace {
            // The texture taken at a vblank shows at the next one.
            Pace::Surface { .. } => self.vsync.present_time(vblank, PRESENT_MARGIN),
            Pace::Ticker(i) => vblank.checked_add(*i).unwrap_or(vblank),
        };
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
        // Drawn without the commit the UI owes: a compositor-only frame.
        let without_ui = self.owed.is_some();
        let drawing = Instant::now();
        let at = clock::seconds(present);
        self.compositor.draw(&mut self.gpu, at);
        // A fling or a resampled gesture moved on: the UI takes it in, and
        // builds what it scrolls toward, with its next frame.
        self.wants |= self.compositor.scrolled();
        if let Pace::Surface {
            callbacks: true,
            window,
        } = &self.pace
        {
            window.pre_present_notify();
            window.request_redraw();
            self.called_back = false;
        }
        let drew = self.applied.take();
        let drawn = self.gpu.drawn();
        self.gpu.end_frame();
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
        let presented = self.compositor.presented();
        *self.presented.lock().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&presented);
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
            });
        }
        if drew.is_some() {
            (self.to_ui)(FromRender::Drawn(Box::new(Feedback {
                drawn,
                held: self.gpu.held(),
            })));
        }
        self.dirty = false;
        true
    }
}
