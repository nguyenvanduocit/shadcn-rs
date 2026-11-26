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
//!
//! **Input (ADR-0033).** Every input reaches the UI through this thread,
//! as every input reaches Blink's main thread through the compositor
//! thread (`widget_input_handler_manager.cc`): it applies the wheels and
//! hands the rest on in the order it took them, each with what the
//! compositor scrolled since the last hand-off ([`super::Outbox::pull`]).
//! So the UI handles every input after what the wheels sent before it
//! moved, and before what later ones move, with no cut, count or wait. It
//! never blocks with input waiting: it waits for a vblank, a commit or a
//! deadline on its channel, and asks the surface for a texture only once
//! the frame presented last shows, when one is free ([`Self::wait_vsync`]).
//! While it is made afresh after a panic, until its compositor can scroll
//! again, input goes straight to the UI ([`Forwarding`]); a device made
//! again leaves the compositor whole, and input goes through it all along.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{Commit, Compositor, MainFrame, Pulled, Seen, input_log};
use crate::clock::{self, Vsync};
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::scene::{Drawn, Held, Renderer, Scene};
use crate::text::Glyphs;
use crate::window::Input;

/// What the render thread is told.
pub(crate) enum ToRender {
    /// A UI frame, and what is drawn over it from now on.
    Commit(Box<Commit>, Option<Scene>),
    /// Replace what is drawn over the frames.
    Overlay(Option<Scene>),
    /// The UI has a frame to run: grant it a [`MainFrame`].
    NeedsFrame,
    /// The wheel at `point` (logical window pixels) moved `delta` at `at`:
    /// a trackpad's (`precise`) is resampled, a notched wheel's applied;
    /// `phase` its place in the platform's gesture. `at` is when the OS
    /// says it happened where it says so, else when it came. `shown` is
    /// the frame on screen as it came: it is routed in that. `event`: the
    /// platform's event it was made of, which an input log being recorded
    /// writes ([`input_log::Route`]).
    Wheel {
        point: (f32, f32),
        delta: (f32, f32),
        at: Instant,
        /// When the platform sent it: its hop is timed from here.
        sent: Instant,
        precise: bool,
        phase: super::WheelPhase,
        shown: Seen,
        event: Option<Box<input_log::Event>>,
    },
    /// Input for the UI (every input but the wheel goes through here, an
    /// assistive technology's action too, ADR-0033), stamped `at` with the
    /// frame `shown` as it came, and the platform's event it was made of:
    /// handed on at once, in the order it came, with what the compositor
    /// scrolled since the last hand-off. `sent`: when the platform sent it,
    /// which its hop is timed from (`at` is when the event happened, which
    /// an OS's stamp or a script's puts before the send).
    Input {
        input: Input,
        at: Instant,
        sent: Instant,
        shown: Seen,
        event: Option<Box<input_log::Event>>,
    },
    /// What the platform told the UI of its window (resized, focused,
    /// shown, its decorations, a screen reader, closing), in one order with
    /// the input ([`input_log::Told`], ADR-0033): what of it is the render
    /// thread's own (a size, being seen) taken in, then handed on as input
    /// is, logged as it is taken.
    Told {
        told: input_log::Told,
        at: Instant,
        shown: Seen,
        event: Option<Box<input_log::Event>>,
    },
    /// The window is this many physical pixels now: its surface is
    /// configured to it at once, and shows the last frame at its own size
    /// until the UI commits one laid out at the new size. Sent alone while
    /// input goes straight to the UI ([`Forwarding`]); else with
    /// [`Self::Told`].
    Resize((u32, u32)),
    /// The window is shown or hidden (occluded, minimized): hidden, nothing
    /// is drawn; shown again, the next frame is presented. Sent alone as
    /// [`Self::Resize`] is.
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
    /// The UI's wheel gesture, handed back as input came through the render
    /// thread again ([`FromRender::Restored`]): the compositor's latch from
    /// now on. The input that came meanwhile waited for it.
    Latch(Option<super::HandedGesture>),
    /// Another thread sent a value to the mailbox at this address: handed
    /// on to the UI as input is, logged here, so the UI takes it in order
    /// with the input and a replay where the log has it (ADR-0033).
    Mail(crate::mailbox::Address),
    /// Fault injection (tests, the stress check): lose the GPU device as a
    /// driver reset would, or panic as a bug in the render thread would.
    LoseDevice,
    /// Lose the device, and have no device answer for this long, as a GPU
    /// still resetting (fault injection).
    HoldLost(Duration),
    Panic,
    /// Fault injection: panic when it next reaches `Stage` (diverting,
    /// with a device being made on its helper thread, as it returns the
    /// UI's latch), as a bug there would.
    #[cfg(test)]
    PanicAt(PanicStage),
    /// Fling trackpad gestures after they end (`true`), or leave it to the
    /// platform's momentum ([`super::Compositor::set_flings`]): what a log
    /// recorded offscreen says its platform does.
    Flings(bool),
    /// Nothing but a wake: the device was lost while the thread slept
    /// ([`crate::gpu::GpuRenderer::wake_on_lost`]), and it is to be made
    /// again and drawn.
    Wake,
    Quit,
}

/// A panic injected (fault injection: a bug in the render thread).
fn injected_panic() -> ! {
    #[expect(
        clippy::panic,
        reason = "fault injection: a bug in the render thread, recovered from"
    )]
    {
        panic!("render thread: panic injected");
    }
}

/// Where an injected panic goes off ([`ToRender::PanicAt`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PanicStage {
    /// As it diverts input (after a panic): under the switch's lock.
    Divert,
    /// While a device is being made on its helper thread.
    Remake,
    /// As it returns the UI's latch, between that and the restore.
    Returned,
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
    /// A wheel event over a wheel handler, as the UI's input
    /// ([`super::Compositor::dispatch_wheel`]: its place in its gesture,
    /// its delta, the node it was over), and the frame shown as it came:
    /// the UI routes it from there, and may consume it.
    Wheel {
        input: Input,
        shown: Seen,
        /// What the compositor scrolled since the last hand-off: the UI
        /// takes it in first (ADR-0033).
        pulled: Pulled,
    },
    /// Input the platform sent ([`ToRender::Input`]), handed on in the
    /// order it came, with what the compositor scrolled since the last
    /// hand-off: the UI takes that in, then handles it (ADR-0033).
    Input {
        input: Input,
        at: Instant,
        /// When the platform sent it ([`ToRender::Input`]): the hop's start.
        sent: Instant,
        shown: Seen,
        pulled: Pulled,
    },
    /// What the platform told of its window ([`ToRender::Told`]), handed on
    /// as input is.
    Told {
        told: input_log::Told,
        pulled: Pulled,
    },
    /// A value for the mailbox at `at` ([`ToRender::Mail`]), handed on as
    /// input is.
    Mail {
        at: crate::mailbox::Address,
        pulled: Pulled,
    },
    /// Input stops going through the render thread ([`Forwarding`]): what
    /// the compositor scrolled and had not handed on (its gesture's
    /// resampling settled first), so the UI has it before any input sent
    /// after; and the wheel gesture under way where the compositor latched
    /// it to a container, which the UI's latch takes on, so the rest of
    /// that gesture scrolls the same container (ADR-0033).
    Diverted {
        pulled: Pulled,
        gesture: Option<super::HandedGesture>,
        /// The UI's own latch is on its way to the render thread (it was
        /// handed back as input came through, [`Self::Restored`]): it
        /// comes back ([`Self::Returned`]), and the UI holds its input
        /// until then.
        returning: bool,
    },
    /// The UI's latch, which reached the render thread after the route
    /// was diverted again, back to the UI ([`Self::Diverted`]'s
    /// `returning`): one side holds a gesture at a time, and none routes
    /// with it in flight.
    Returned(Option<super::HandedGesture>),
    /// Input comes through the render thread again, from after the input
    /// sent straight before this: the UI hands its wheel gesture back
    /// ([`ToRender::Latch`]) and latches none it hands, so one side holds
    /// a gesture's latch at a time (ADR-0033).
    Restored,
    /// The timeline has frames the UI has not looked at: it reads them
    /// there ([`super::OnScreen::look`]). Sent once until it looks, so a
    /// stalled UI holds one message and no frame.
    Presented,
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

/// Where the platform sends input (ADR-0033): to the render thread, which
/// hands it on, while it serves; straight to the UI while it cannot (made
/// afresh after a panic, or its device made again). The switch is made
/// under the lock every send is, with what the render thread holds handed
/// on first ([`RenderThread::divert`]), so no input overtakes another:
/// the UI handles them in the platform's order either way. The input log
/// is written under the same lock ([`input_log::Route`]), by whichever
/// path takes the input, so it is in that order too (Opus r10 P1).
#[derive(Debug)]
pub(crate) struct Forwarding(Mutex<input_log::Route>);

impl Forwarding {
    /// Input goes through the render thread.
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(input_log::Route::new())))
    }
    /// Run `send` with the route as it is now (whether input goes through
    /// the render thread, and the log), under the lock: no switch falls
    /// between the choice and the send.
    pub(crate) fn send<R>(&self, send: impl FnOnce(&mut input_log::Route) -> R) -> R {
        send(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Input goes straight to the UI from now on; `hand_on` runs first,
    /// under the lock, so what the render thread held reaches the UI before
    /// anything sent straight.
    fn divert(&self, at: Instant, hand_on: impl FnOnce(&mut input_log::Route)) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .divert(at, hand_on);
    }

    /// Input goes through the render thread again; `tell` runs under the
    /// lock, after everything sent straight. Whether a wheel went to the
    /// UI while diverted ([`input_log::Route::took_wheel`]).
    fn restore(&self, tell: impl FnOnce()) -> bool {
        let mut route = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !route.through {
            tell();
        }
        route.restore()
    }
}

/// What of what the platform told is the render thread's own: the size its
/// surface is configured to, and whether the window can be seen.
pub(crate) fn own_part(told: &input_log::Told) -> Option<ToRender> {
    match told {
        input_log::Told::Resized { size, .. } => Some(ToRender::Resize(*size)),
        input_log::Told::Visible(v) => Some(ToRender::Visible(*v)),
        _ => None,
    }
}

/// How long before the vblank the frame presented last shows at an
/// acquire is made where the display says nothing of its own phase: it
/// blocks that long, a sample of the phase ([`Vsync::acquired`]), and input
/// waits no longer than that and the fit's error (ADR-0033).
const ACQUIRE_PROBE: Duration = Duration::from_micros(1_500);

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
    /// The output the window is on, in its mode: its vsync and every
    /// vblank a frame was drawn or granted for, made afresh as one.
    timing: Timing,
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
    /// A lost device no device answered for yet: asked again at this time,
    /// what comes meanwhile taken in (input scrolled, handed on).
    device_retry: Option<Instant>,
    /// Input came through it again and the UI's gesture is not back yet
    /// ([`Self::restore`]): the input that comes meanwhile waits in
    /// `waiting`, in its order.
    awaiting_latch: bool,
    waiting: VecDeque<ToRender>,
    /// Diverted again while the UI's latch was on its way here: it is
    /// returned to the UI as it comes ([`FromRender::Returned`]), and a
    /// restore due meanwhile waits for that (`restore_due`).
    latch_out: bool,
    restore_due: bool,
    /// Fault injection: no device answers until then ([`ToRender::HoldLost`]).
    no_device_until: Option<Instant>,
    /// Fault injection: panic when it next reaches this ([`ToRender::PanicAt`]).
    panic_at: Option<PanicStage>,
    /// A device being made on a thread of its own ([`Self::ensure_device`]),
    /// the wait before the next attempt after one failed, and whether the
    /// loss was told (once).
    remaking: Option<Receiver<Result<crate::gpu::Remade, crate::gpu::GpuError>>>,
    backoff: Duration,
    told_lost: bool,
    /// A device was made again and taken, and no frame drew on it yet
    /// ([`Frame::renewed`]): taken in [`Self::step`], before the frame.
    renewed: bool,
    /// The size the window was last told to be, not yet configured: only
    /// the last of a drag's resizes configures the surface.
    resize_to: Option<(u32, u32)>,
    /// Wayland: the frame callback came since the last present, and when.
    called_back: bool,
    callback_at: Option<Instant>,
    log: Option<Arc<Mutex<Vec<FrameStat>>>>,
    /// The commit applied in the frame being drawn.
    applied: Option<u64>,
    /// The ticker's last vblank.
    ticked: Option<Instant>,
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
    /// Where the platform sends input: switched while it cannot serve
    /// ([`Forwarding`]); none in tests that send it all.
    forwarding: Option<Arc<Forwarding>>,
    /// What came before input was diverted and is not input, taken in
    /// before anything on the channel once it serves again.
    deferred: VecDeque<ToRender>,
}

/// What the render thread times its frames by on the output the window is
/// on, in that output's current mode: the vsync fitted to it, its ticks,
/// and every vblank a frame was drawn or granted for. All of it is of one
/// output and one mode, so all of it is made afresh together when either
/// changes ([`Timing::reset`]): no deadline, phase or reserved vblank of
/// the last carries over to be aimed at on another grid.
///
/// A frame aims, waits and then writes what it was drawn for, and an
/// `Output` or `Interval` handled while it waits makes the timing afresh
/// under it. So what a frame aimed is of an [`Timing::epoch`]: a grant, a
/// reserved or blank vblank written of another epoch than the timing's is
/// dropped here, and the frame aims again ([`RenderThread::frame`]).
struct Timing {
    /// Counts the times the timing was made afresh.
    epoch: u64,
    vsync: Vsync,
    /// The vsync ticks of the output, where its OS has a source for them
    /// (ADR-0029): wanted while frames are drawn, not while the thread
    /// sleeps, so a window at rest wakes nothing at a vblank.
    ticks: Option<gui_display::Ticks>,
    /// The output's mode the ticks taken must be of: one taken before the
    /// change is of the last grid ([`gui_display::Tick::generation`]).
    generation: u64,
    /// The vblank the last frame that presented, or took a texture, was
    /// drawn for: no later frame is drawn for it or an earlier one.
    reserved: Option<Instant>,
    /// The vblank the last frame was drawn for, when it presented nothing
    /// and took no texture: drawn for again only for something new before
    /// its deadline (a wheel, a commit), else the next frame is drawn for
    /// the vblank after it, not again and again for the same.
    blank: Option<Instant>,
    /// The vblank a main frame granted while the thread slept is for: the
    /// frame drawn next is drawn for it too, whatever its acquire took.
    granted_for: Option<Instant>,
}

impl Timing {
    /// Timing afresh against `vsync`, with the output's `ticks`, of the
    /// mode they are in now.
    fn of(vsync: Vsync, ticks: Option<gui_display::Ticks>) -> Self {
        Self {
            epoch: 0,
            vsync,
            generation: ticks.as_ref().map_or(0, gui_display::Ticks::generation),
            ticks,
            reserved: None,
            blank: None,
            granted_for: None,
        }
    }

    /// The output's refresh is now `interval` (its mode changed, or the
    /// platform first reports it): everything is made afresh but its ticks,
    /// which are of the new mode from now on.
    fn remode(&mut self, interval: Duration) {
        // `Vsync::set_interval`'s rule, for all the timing.
        if self.vsync.reported() != Some(interval) {
            let ticks = self.ticks.take();
            self.reset(Vsync::reporting(interval), ticks);
        }
    }

    /// Made afresh against `vsync` and `ticks`, in a new epoch: whatever a
    /// frame aimed before is of the last.
    fn reset(&mut self, vsync: Vsync, ticks: Option<gui_display::Ticks>) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self {
            epoch,
            ..Self::of(vsync, ticks)
        };
    }

    /// Whether what was aimed in `epoch` is of the timing as it is now.
    fn holds(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }

    /// The frame next drawn is for `present`, granted to the UI in `epoch`.
    fn grant_for(&mut self, epoch: u64, present: Instant) {
        if self.holds(epoch) {
            self.granted_for = Some(present);
        }
    }

    /// A frame aimed in `epoch` presented, or took a texture, for `present`.
    fn reserve(&mut self, epoch: u64, present: Instant) {
        if self.holds(epoch) {
            self.reserved = Some(present);
            self.blank = None;
        }
    }

    /// A frame aimed in `epoch` was drawn for `present` to no effect.
    fn drew_blank(&mut self, epoch: u64, present: Instant) {
        if self.holds(epoch) {
            self.blank = Some(present);
        }
    }
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
pub(crate) const MAX_RECOVERIES: usize = 3;
const RECOVERY_WINDOW: Duration = Duration::from_secs(10);
/// How long to wait before asking again for a device that did not answer
/// (a GPU still resetting), at first.
const REBUILD_RETRY: Duration = Duration::from_millis(100);
/// The longest wait between two attempts at a lost device, backed off.
const REBUILD_MAX: Duration = Duration::from_secs(5);
/// How often a device being made is looked for, the thread meanwhile
/// asleep on its channel.
const REMAKE_POLL: Duration = Duration::from_millis(4);

impl RenderThread {
    pub(crate) fn new(
        compositor: Compositor,
        gpu: GpuRenderer,
        rx: Receiver<ToRender>,
        to_ui: Box<dyn Fn(FromRender) + Send>,
        interval: Duration,
        pace: Pace,
        log: Option<Arc<Mutex<Vec<FrameStat>>>>,
    ) -> Self {
        Self {
            compositor,
            gpu,
            rx,
            to_ui,
            timing: Timing::of(Vsync::new(interval), None),
            pace,
            wants: false,
            owed: None,
            sequence: 0,
            dirty: false,
            retry: false,
            visible: true,
            occluded_until: None,
            device_retry: None,
            awaiting_latch: false,
            latch_out: false,
            restore_due: false,
            waiting: VecDeque::new(),
            no_device_until: None,
            panic_at: None,
            remaking: None,
            backoff: REBUILD_RETRY,
            told_lost: false,
            renewed: false,
            resize_to: None,
            called_back: true,
            callback_at: None,
            log,
            applied: None,
            ticked: None,
            unpresented: 0,
            chained: false,
            granted: None,
            ui_took: None,
            answered: None,
            ui_times: VecDeque::new(),
            costs: VecDeque::new(),
            awaiting_full: false,
            recoveries: VecDeque::new(),
            forwarding: None,
            deferred: VecDeque::new(),
        }
    }

    /// Input reaches it through `forwarding`, which it switches while it
    /// cannot serve.
    pub(crate) fn forwarding(mut self, forwarding: Arc<Forwarding>) -> Self {
        self.forwarding = Some(forwarding);
        self
    }

    /// The next message: one deferred first, then what the channel has,
    /// waiting until `until` (none: for as long as it takes).
    fn next(&mut self, until: Option<Instant>) -> Result<ToRender, RecvTimeoutError> {
        if let Some(m) = self.deferred.pop_front() {
            return Ok(m);
        }
        match until {
            Some(t) => self.rx.recv_timeout(t.saturating_duration_since(Instant::now())),
            None => self.rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        }
    }

    /// The next message there is now, deferred first.
    fn next_now(&mut self) -> Option<ToRender> {
        self.deferred.pop_front().or_else(|| self.rx.try_recv().ok())
    }

    /// Take in what comes until `until`: `false` when told to quit.
    fn wait_until(&mut self, until: Instant) -> bool {
        loop {
            match self.next(Some(until)) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        return false;
                    }
                }
                Err(RecvTimeoutError::Timeout) => return true,
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// Input goes straight to the UI from now on ([`Forwarding`]): what the
    /// compositor scrolled and had not handed on goes first, then the input
    /// waiting on the channel, in its order; what else waits is kept for
    /// when it serves again. All under the switch's lock, so nothing sent
    /// straight overtakes it.
    pub(crate) fn divert(&mut self) {
        let Some(f) = self.forwarding.clone() else { return };
        // What it held is settled to now, the time the log's `diverted`
        // has, so a replay settles it to the same.
        let now = clock::current_nanos();
        f.divert(clock::instant_of(now), |route| {
            // Fault injection: a panic as it begins to divert, under the
            // switch's lock, nothing handed on yet.
            if self.panic_at == Some(PanicStage::Divert) {
                self.panic_at = None;
                injected_panic();
            }
            // What it accepted is handed on whole (Codex r11 1): the
            // displacement its gesture's resampling held goes into the
            // outbox, then all of it to the UI, with the gesture's latch.
            // After a panic the compositor may be half way through a
            // change: what it holds is taken only if that takes.
            let (pulled, gesture) = std::panic::catch_unwind(AssertUnwindSafe(|| {
                self.compositor.settle_gesture(now);
                (self.compositor.pull(), self.compositor.hand_gesture())
            }))
            .unwrap_or_default();
            // The UI's latch on its way here (input came through again,
            // and its gesture is not back yet): it goes back to the UI as
            // it comes, and the UI holds its input until then, so no
            // wheel it takes meanwhile is routed with no latch.
            let returning = self.awaiting_latch;
            self.latch_out |= returning;
            if !pulled.is_empty() || gesture.is_some() || returning {
                (self.to_ui)(FromRender::Diverted {
                    pulled,
                    gesture,
                    returning,
                });
            }
            // Each taken here is logged here, as it is handed on: what
            // waited and went back on the queue as the latch came
            // (the oldest), then what still waits for it, then the
            // channel: every input not yet handed on, in its order (the
            // fault matrix: input left on the queue went to the
            // compositor made afresh, a wheel into no scene).
            self.awaiting_latch = false;
            let (queued, kept): (VecDeque<ToRender>, VecDeque<ToRender>) =
                std::mem::take(&mut self.deferred).into_iter().partition(|m| {
                    matches!(
                        m,
                        ToRender::Input { .. } | ToRender::Wheel { .. } | ToRender::Told { .. } | ToRender::Mail(_)
                    )
                });
            self.deferred = kept;
            let waiting = std::mem::take(&mut self.waiting);
            for msg in queued
                .into_iter()
                .chain(waiting)
                .chain(std::iter::from_fn(|| self.rx.try_recv().ok()))
            {
                match msg {
                    ToRender::Input {
                        input,
                        at,
                        sent,
                        shown,
                        event,
                    } => {
                        if let Some(e) = &event {
                            route.record(at, e, shown);
                        }
                        (self.to_ui)(FromRender::Input {
                            input,
                            at,
                            sent,
                            shown,
                            pulled: Pulled::default(),
                        });
                    }
                    // The UI scrolls it, in order with the rest.
                    ToRender::Wheel {
                        point,
                        delta,
                        at,
                        sent,
                        precise,
                        phase,
                        shown,
                        event,
                    } => {
                        if let Some(e) = &event {
                            route.record(at, e, shown);
                        }
                        route.took_wheel(precise, phase);
                        (self.to_ui)(FromRender::Input {
                            input: Input::diverted_wheel(point, delta, at, precise, phase),
                            at,
                            sent,
                            shown,
                            pulled: Pulled::default(),
                        });
                    }
                    // The UI's part handed on in order; the thread's own
                    // kept for when it serves again.
                    ToRender::Told { told, at, shown, event } => {
                        if let Some(e) = &event {
                            route.record(at, e, shown);
                        }
                        if let Some(own) = own_part(&told) {
                            self.deferred.push_back(own);
                        }
                        (self.to_ui)(FromRender::Told {
                            told,
                            pulled: Pulled::default(),
                        });
                    }
                    ToRender::Mail(at) => {
                        route.observe(input_log::Observation::Mail(at.site, at.nth));
                        (self.to_ui)(FromRender::Mail {
                            at,
                            pulled: Pulled::default(),
                        });
                    }
                    // The UI's latch, on its way here as it diverts: back
                    // to the UI at once, before the input after it, with
                    // no device and no recovery waited for (Opus r13 I2).
                    ToRender::Latch(gesture) if self.latch_out => {
                        self.latch_out = false;
                        (self.to_ui)(FromRender::Returned(
                            gesture.filter(|g| !g.1).map(|g| super::HandedGesture(g.0, false, g.2)),
                        ));
                    }
                    other => self.deferred.push_back(other),
                }
            }
        });
    }

    /// Input goes through it again: the UI is told so under the switch's
    /// lock, after everything sent it straight, and hands its gesture back
    /// ([`ToRender::Latch`]); the input that comes until then waits for
    /// it, so no wheel is routed past the gesture's latch (Codex r12 1).
    fn restore(&mut self) {
        // The UI's latch is still on its way back to it: input comes
        // through again once it is there, one latch in flight at a time.
        if self.latch_out {
            self.restore_due = true;
            return;
        }
        let Some(f) = self.forwarding.clone() else { return };
        // A wheel the UI took meanwhile stops a fling handed on, as a new
        // gesture stops it in the compositor (ADR-0033, "Accepted motion").
        if f.restore(|| (self.to_ui)(FromRender::Restored)) {
            self.compositor.stop_fling();
        }
        self.awaiting_latch = true;
    }

    /// While no frame is drawn (no device, the window hidden), no tick
    /// moves a gesture on: what its resampling holds and its fling are
    /// applied whole before input or a frame is handed on, so the UI has
    /// all the compositor accepted (Opus r12 P2). Logged, for a replay to
    /// do the same there.
    fn settle_undrawn(&mut self) {
        let undrawn = self.gpu.lost() || !self.visible || self.occluded_until.is_some();
        if undrawn && self.compositor.gesture_moving() {
            // Logged with the time it settled to, which a replay settles to.
            let now = clock::current_nanos();
            self.compositor.settle_gesture(now);
            if let Some(f) = &self.forwarding {
                f.send(|route| route.observe_at(clock::instant_of(now), input_log::Observation::Settled));
            }
        }
    }

    /// Write `event`, taken at `at` and routed in `shown`, to the input
    /// log being recorded.
    fn record(&self, at: Instant, event: Option<Box<input_log::Event>>, shown: Seen) {
        if let (Some(f), Some(e)) = (&self.forwarding, event) {
            f.send(|route| route.record(at, &e, shown));
        }
    }

    /// Write what it did now to the input log being recorded.
    fn observe(&self, what: input_log::Observation) {
        if let Some(f) = &self.forwarding {
            f.send(|route| route.observe(what));
        }
    }

    /// Run until told to quit or the UI thread goes away, starting afresh
    /// after a panic (ADR-0015's invariant 7).
    ///
    /// The whole body is under the one catch (Codex r13, Opus r13): a
    /// panic while it diverts or recovers (made afresh, the compositor and
    /// its glyphs) is a panic like any, counted, and past the last
    /// recovery the thread stops with the UI the one owner of input,
    /// diverted for good ([`FromRender::Failed`]). Input goes straight to
    /// the UI until its compositor can scroll again: made afresh, it holds
    /// no scroll container until the UI's full commit, and a wheel it took
    /// meanwhile would scroll nothing (Codex r10 1, Opus r10 I1). That
    /// commit restores it.
    pub(crate) fn run(mut self) {
        let mut recovering = false;
        loop {
            let ran = std::panic::catch_unwind(AssertUnwindSafe(|| {
                if recovering {
                    self.divert();
                    self.recover();
                }
                self.serve();
            }));
            if ran.is_ok() {
                return;
            }
            recovering = true;
            let now = Instant::now();
            self.recoveries
                .retain(|t| now.saturating_duration_since(*t) < RECOVERY_WINDOW);
            if self.recoveries.len() >= MAX_RECOVERIES {
                self.fail(format!(
                    "it panicked {} times in {} s",
                    MAX_RECOVERIES.saturating_add(1),
                    RECOVERY_WINDOW.as_secs()
                ));
                return;
            }
            self.recoveries.push_back(now);
        }
    }

    /// Give up past the last recovery: input goes straight to the UI for
    /// good, what the thread holds of it handed on first, and the UI is
    /// told. Under its own catch: a panic here still leaves the route
    /// diverted, which the switch's lock guarantees by itself.
    fn fail(&mut self, why: String) {
        let diverted = std::panic::catch_unwind(AssertUnwindSafe(|| self.divert()));
        if diverted.is_err()
            && let Some(f) = &self.forwarding
        {
            f.divert(Instant::now(), |_| {});
        }
        (self.to_ui)(FromRender::Failed(why));
    }

    /// Start afresh after a panic: an empty compositor going on with what
    /// the one before accepted, a new device, and a full commit asked of
    /// the UI. Nothing blocks (Opus r13 I2): the device is marked lost and
    /// made on a thread of its own as any lost one is
    /// ([`Self::ensure_device`]), the thread serving its channel meanwhile
    /// (the UI's latch, input); nothing is drawn until the device and the
    /// commit come, so the window keeps its last frame meanwhile.
    fn recover(&mut self) {
        // The window still shows what was presented: the timeline goes on,
        // and so does what the compositor accepted of input (ADR-0033,
        // [`Accepted`]).
        let old = std::mem::replace(&mut self.compositor, Compositor::new(Glyphs::for_os()));
        let (timeline, accepted) = old.into_accepted();
        self.compositor = Compositor::after(Glyphs::for_os(), timeline, accepted);
        // What the panic left of the renderer is not trusted: made again.
        self.gpu.lose();
        self.remaking = None;
        self.device_retry = None;
        self.owed = None;
        self.granted = None;
        self.applied = None;
        self.dirty = false;
        self.retry = false;
        self.awaiting_full = true;
        // The full commit restores input; a restore due before is that one.
        self.restore_due = false;
        // A main frame for the full commit.
        self.wants = true;
        (self.to_ui)(FromRender::Lost);
    }

    /// Whether it has a device: a lost one is made again, the committed
    /// scene to be drawn whole. Never blocks: the device is made on a
    /// thread of its own ([`crate::gpu::Remake`], Codex r12 2: making one
    /// takes hundreds of milliseconds, and this thread hands every input
    /// on), and taken here once made; `false` until then. An attempt that
    /// fails is tried again after a back-off doubling from
    /// [`REBUILD_RETRY`] to [`REBUILD_MAX`], told once, and none is made
    /// while the window cannot be seen (Opus r12 P1).
    ///
    /// Input keeps going through it all along (Opus r11 I3): its
    /// compositor keeps its tables, its gesture and its latch, and scrolls
    /// without a device; only drawing waits for one. Diverting would hand
    /// the UI wheels to route afresh, off the gesture's latch.
    fn ensure_device(&mut self) -> bool {
        if !self.gpu.lost() {
            return true;
        }
        let now = Instant::now();
        if let Some(rx) = &self.remaking {
            let made = match rx.try_recv() {
                Ok(made) => made,
                // Fault injection: a panic while the device is being made.
                Err(std::sync::mpsc::TryRecvError::Empty) if self.panic_at == Some(PanicStage::Remake) => {
                    self.panic_at = None;
                    injected_panic();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Err(crate::gpu::GpuError("making the device panicked".to_owned()))
                }
            };
            self.remaking = None;
            match made.and_then(|m| self.gpu.take_made(m)) {
                Ok(old) => {
                    // The lost renderer is let go of off this thread:
                    // freeing its device takes milliseconds. No thread,
                    // and it goes here.
                    std::thread::Builder::new()
                        .name("gui device".to_owned())
                        .spawn(move || drop(old))
                        .ok();
                    self.renewed = true;
                    self.dirty = true;
                    // The lost device's last present never calls back:
                    // the new one has none made, so none is waited for
                    // (the fault matrix: a device lost after a present
                    // drew nothing again).
                    self.called_back = true;
                    self.device_retry = None;
                    self.backoff = REBUILD_RETRY;
                    if std::mem::take(&mut self.told_lost) {
                        eprintln!("render thread: a device answers again");
                    }
                    return true;
                }
                Err(e) => {
                    if !std::mem::replace(&mut self.told_lost, true) {
                        eprintln!("render thread: the device was lost and none answers yet ({e}); trying again");
                    }
                    self.device_retry = now.checked_add(self.backoff);
                    self.backoff = self.backoff.saturating_mul(2).min(REBUILD_MAX);
                    return false;
                }
            }
        }
        // Another attempt when due, while the window can be seen.
        if self.device_retry.is_some_and(|t| t > now) || !self.visible {
            return false;
        }
        self.device_retry = None;
        let remake = self.gpu.remake();
        let refused = self.no_device_until.is_some_and(|t| t > now);
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("gui device".to_owned())
            .spawn(move || {
                let made = if refused {
                    Err(crate::gpu::GpuError("no device answers (held lost)".to_owned()))
                } else {
                    remake.make()
                };
                tx.send(made).ok();
            });
        // No thread: an attempt that failed.
        if spawned.is_ok() {
            self.remaking = Some(rx);
        } else {
            self.device_retry = now.checked_add(self.backoff);
        }
        false
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
        // occluded window until its slow probe, a lost device that no
        // device answered for until it is asked again.
        let now = Instant::now();
        let occluded = self.occluded_until.filter(|t| *t > now);
        if occluded.is_none() {
            self.occluded_until = None;
        }
        // A lost device: one being made is looked for, another tried
        // when due; neither blocks (Codex r12 2). Paused while hidden.
        let lost = !self.ensure_device();
        let retry = if !lost {
            None
        } else if self.remaking.is_some() {
            now.checked_add(REMAKE_POLL)
        } else {
            self.device_retry
        };
        let paused = occluded.into_iter().chain(retry).min();
        let asleep = !self.busy() || !self.visible || !self.called_back || paused.is_some();
        if asleep {
            if let Some(ticks) = &self.timing.ticks {
                ticks.run(false);
            }
            let msg = match self.next(paused) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return false,
            };
            if let Some(msg) = msg
                && !self.handle(msg)
            {
                return false;
            }
        }
        while let Some(msg) = self.next_now() {
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
        let hidden = self.occluded_until.is_some() || self.gpu.lost();
        !(self.busy() && self.visible && self.called_back && !hidden) || self.frame()
    }

    /// Want the display's ticks (a frame is about to be timed) and take the
    /// last one as the vsync's phase. A tick from before the source last
    /// stopped is taken once and then outlived by the acquires
    /// ([`Vsync::acquired`]) until the source, started again, ticks.
    fn take_tick(&mut self) {
        let t = &mut self.timing;
        if let Some(ticks) = &t.ticks {
            ticks.run(true);
            // A tick of the mode before the last change is of another grid.
            if let Some(tick) = ticks.latest().filter(|k| k.generation == t.generation) {
                t.vsync.ticked(&tick);
            }
        }
    }

    /// Grant the UI a main frame now, the thread having slept: for the
    /// vblank the next frame is drawn for ([`Self::aim`]), which that frame
    /// keeps however long its acquire takes ([`Self::granted_for`]).
    fn grant_now(&mut self) {
        let now = Instant::now();
        let epoch = self.timing.epoch;
        let present = match &self.pace {
            Pace::Surface { .. } => {
                let p = self.aim(now);
                self.timing.grant_for(epoch, p);
                p
            }
            Pace::Ticker(i) => {
                let tick = self.next_tick(*i, now);
                tick.checked_add(*i).unwrap_or(tick)
            }
        };
        let deadline = present.checked_sub(self.margin()).filter(|d| *d > now).unwrap_or(now);
        self.grant(epoch, present, deadline);
    }

    /// Whether there is a frame to draw. A lost device is a reason of its
    /// own, read off the device rather than a flag a frame clears: lost
    /// while a frame drains the channel or waits for its commit (after the
    /// frame made sure of its device), the frame draws nothing and clears
    /// `dirty`, and the device is still to be made again and drawn whole
    /// before the thread sleeps.
    fn busy(&self) -> bool {
        let now = Instant::now();
        self.dirty
            || self.retry
            || self.gpu.lost()
            || (self.wants && self.owed.is_none())
            || self.compositor.animating(now)
            || self.gpu.raster_pending()
    }

    /// Take in one message; `false` to quit.
    pub(super) fn handle(&mut self, msg: ToRender) -> bool {
        // Input waits for the UI's gesture as the route comes back to it,
        // in its order (Codex r12 1).
        if self.awaiting_latch
            && matches!(
                msg,
                ToRender::Input { .. } | ToRender::Wheel { .. } | ToRender::Told { .. } | ToRender::Mail(_)
            )
        {
            self.waiting.push_back(msg);
            return true;
        }
        match msg {
            // Diverted again before it came: the UI's still, returned to
            // it, which holds its input until then; and only then may
            // input come through again.
            ToRender::Latch(gesture) if !self.awaiting_latch => {
                self.latch_out = false;
                (self.to_ui)(FromRender::Returned(
                    gesture.filter(|g| !g.1).map(|g| super::HandedGesture(g.0, false, g.2)),
                ));
                // Fault injection: a panic between the latch returned and
                // input restored.
                if self.panic_at == Some(PanicStage::Returned) {
                    self.panic_at = None;
                    injected_panic();
                }
                if std::mem::take(&mut self.restore_due) {
                    self.restore();
                }
            }
            ToRender::Latch(gesture) => {
                if let Some(g) = gesture {
                    self.compositor.take_gesture(g);
                }
                self.awaiting_latch = false;
                // What waited goes first, in its order.
                while let Some(m) = self.waiting.pop_back() {
                    self.deferred.push_front(m);
                }
            }
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
                    // A delta against what a panic dropped: aborted. What
                    // the UI took for it stays kept, and the full commit
                    // carries it (ADR-0032, I4).
                    commit.clear();
                    self.observe(input_log::Observation::Aborted);
                    (self.to_ui)(FromRender::Spare(commit));
                    return true;
                }
                let recovered = std::mem::replace(&mut self.awaiting_full, false);
                self.applied = Some(commit.sequence);
                if let Some(f) = &self.forwarding {
                    f.send(|route| {
                        route.committed = true;
                        route.observe(input_log::Observation::Committed);
                    });
                }
                self.compositor.apply(&mut commit);
                (self.to_ui)(FromRender::Spare(commit));
                self.compositor.set_overlay(overlay);
                self.dirty = true;
                if recovered {
                    // Its tables are whole again: it can scroll what the
                    // UI shows, so input comes through it again.
                    self.restore();
                }
            }
            ToRender::Overlay(o) => {
                self.compositor.set_overlay(o);
                self.dirty = true;
            }
            ToRender::NeedsFrame => self.wants = true,
            // Handed on at once, after what every wheel taken before it
            // moved, before what any later one moves (ADR-0033).
            ToRender::Input {
                input,
                at,
                sent,
                shown,
                event,
            } => {
                self.settle_undrawn();
                self.record(at, event, shown);
                let pulled = self.compositor.pull();
                (self.to_ui)(FromRender::Input {
                    input,
                    at,
                    sent,
                    shown,
                    pulled,
                });
            }
            ToRender::Wheel {
                point,
                delta,
                at,
                sent: _,
                precise,
                phase,
                shown,
                event,
            } => {
                self.record(at, event, shown);
                let w = super::WheelInput {
                    point,
                    delta,
                    time: clock::nanos(at),
                    precise,
                    phase,
                    seen: shown,
                };
                match self.compositor.dispatch_wheel(w) {
                    super::Dispatched::Ui(input) => {
                        // Handed on as input is.
                        let pulled = self.compositor.pull();
                        (self.to_ui)(FromRender::Wheel { input, shown, pulled });
                        return true;
                    }
                    super::Dispatched::Scrolled => {
                        self.dirty = true;
                        // The UI takes the scroll in with its next frame.
                        self.wants = true;
                    }
                    super::Dispatched::Nothing => {}
                }
            }
            // Its own part taken in, then handed on as input is, after
            // what the wheels taken before it moved.
            ToRender::Told { told, at, shown, event } => {
                self.settle_undrawn();
                self.record(at, event, shown);
                if let Some(own) = own_part(&told)
                    && !self.handle(own)
                {
                    return false;
                }
                let pulled = self.compositor.pull();
                (self.to_ui)(FromRender::Told { told, pulled });
            }
            // Handed on as input is, after what the wheels taken before
            // it moved, and logged where it is taken.
            ToRender::Mail(at) => {
                self.settle_undrawn();
                self.observe(input_log::Observation::Mail(at.site, at.nth));
                let pulled = self.compositor.pull();
                (self.to_ui)(FromRender::Mail { at, pulled });
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
            ToRender::Interval(i) => self.timing.remode(i),
            ToRender::Output { interval, ticks } => {
                self.timing.reset(Vsync::reporting(interval), ticks);
            }
            ToRender::FrameCallback(at) => {
                self.called_back = true;
                self.callback_at = Some(at);
                self.timing.vsync.called_back(at, std::mem::take(&mut self.chained));
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
            ToRender::HoldLost(d) => {
                self.no_device_until = Instant::now().checked_add(d);
                self.gpu.lose();
            }
            // Woken: `busy` reads the loss off the device.
            ToRender::Wake => {}
            ToRender::Panic => injected_panic(),
            #[cfg(test)]
            ToRender::PanicAt(stage) => self.panic_at = Some(stage),
            ToRender::Flings(flings) => self.compositor.set_flings(flings),
            ToRender::Quit => return false,
        }
        true
    }

    /// Wait for the next vblank where there is one to wait for: when the
    /// frame begins, and how long the display held it there, when that is
    /// a sample of the vsync. Only a texture acquired, or a tick, is: a
    /// frame begun with a texture held, or whose acquire failed (a minimized
    /// window), waited for nothing, and its deadline paces it
    /// ([`Self::frame`]). `None` when told to quit meanwhile.
    ///
    /// It never blocks with input waiting (ADR-0033): until the vblank the
    /// frame presented last shows at, when the surface frees a texture, it
    /// waits on its channel and hands input on; only then does it ask for
    /// the texture, which a display whose phase it knows (its vsync source,
    /// or a Wayland frame callback) gives at once, and one it fits from the
    /// acquires gives within [`ACQUIRE_PROBE`] and the fit's error. A
    /// ticker's tick is waited for on the channel alike.
    fn wait_vsync(&mut self) -> Option<(Instant, Option<Duration>)> {
        match &self.pace {
            Pace::Surface { .. } => {
                if self.gpu.holds_frame() {
                    return Some((Instant::now(), None));
                }
                // Taking in what comes until the acquire is due, input
                // handed on at once; asked again after each message, which
                // may change when (the window on another output).
                while let Some(due) = self.acquire_due(Instant::now()) {
                    match self.next(Some(due)) {
                        Ok(msg) => {
                            if !self.handle(msg) {
                                return None;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return None,
                    }
                }
                let waited = self.gpu.acquire();
                Some((Instant::now(), waited))
            }
            Pace::Ticker(interval) => {
                let interval = *interval;
                let now = Instant::now();
                let next = self.next_tick(interval, now);
                if !self.wait_until(next) {
                    return None;
                }
                self.ticked = Some(next);
                Some((next, Some(next.saturating_duration_since(now))))
            }
        }
    }

    /// When a texture is to be asked for, at `now`: at the vblank the frame
    /// presented last shows at (the surface frees one then), or
    /// [`ACQUIRE_PROBE`] before it where the display tells nothing of its
    /// phase; none when that is past or nothing was presented for a vblank
    /// ahead.
    fn acquire_due(&self, now: Instant) -> Option<Instant> {
        let shows = self.timing.reserved?;
        let due = if self.timing.vsync.told_lately(now) {
            shows
        } else {
            shows.checked_sub(ACQUIRE_PROBE)?
        };
        (due > now).then_some(due)
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
        let interval = self.timing.vsync.interval();
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
        let present = self.timing.vsync.present_time(at, PRESENT_MARGIN);
        match self.timing.reserved {
            Some(r) if present <= r => self.timing.vsync.next_after(r),
            _ => present,
        }
    }

    /// When the commit of a frame begun at `vblank` and presented at
    /// `present` is wanted by: the time kept to draw before `present`.
    fn deadline(&self, present: Instant, vblank: Instant) -> Instant {
        present
            .checked_sub(self.margin())
            .filter(|d| *d > vblank)
            .unwrap_or(vblank)
    }

    /// A frame begun at `vblank`, aimed again on timing made afresh while
    /// it waited: the epoch it is aimed in now, its present and deadline.
    fn reaim(&self, vblank: Instant) -> (u64, Instant, Instant) {
        let present = match &self.pace {
            Pace::Surface { .. } => self.aim(vblank),
            Pace::Ticker(i) => vblank.checked_add(*i).unwrap_or(vblank),
        };
        (self.timing.epoch, present, self.deadline(present, vblank))
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
            match self.next(Some(deadline)) {
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

    /// Hand the UI a main frame presented at `present`, its commit wanted
    /// by `deadline`, both aimed in `epoch`: none when the timing was made
    /// afresh since ([`Timing::holds`]).
    fn grant(&mut self, epoch: u64, present: Instant, deadline: Instant) {
        if !self.timing.holds(epoch) {
            return;
        }
        self.settle_undrawn();
        self.sequence = self.sequence.saturating_add(1);
        let token = MainFrame {
            sequence: self.sequence,
            present_at: clock::nanos(present),
            deadline,
            interval: self.timing.vsync.interval(),
            // What it scrolled, handed on with the frame (ADR-0033).
            taken: self.compositor.pull_for_frame(),
            scroll: self.compositor.begin_main_frame(
                Instant::now(),
                super::horizon(self.ui_worst(), self.timing.vsync.interval()),
            ),
        };
        // When it is presented is consumed by the UI (its drag resampled
        // for it): logged with it (Codex r10 3), and sent under the same
        // lock, so input the platform sends straight meanwhile (diverted)
        // is in the log on the same side of the grant as in the UI's queue
        // (Codex r11 plausible).
        self.wants = false;
        self.owed = Some((self.sequence, deadline));
        self.granted = Some(Instant::now());
        match &self.forwarding {
            Some(f) => f.send(|route| {
                route.grant(present);
                (self.to_ui)(FromRender::Begin(token));
            }),
            None => (self.to_ui)(FromRender::Begin(token)),
        }
    }

    /// Draw one frame; `false` when the UI thread is gone.
    fn frame(&mut self) -> bool {
        if !self.ensure_device() {
            return true;
        }
        let renewed = std::mem::take(&mut self.renewed);
        if let Some(size) = self.resize_to.take() {
            self.gpu.resize(size);
        }
        // Wanted before the acquire blocks, so a source that stopped while
        // the thread slept is started meanwhile; read after it, for the
        // tick of the vblank the acquire waited for.
        if let Some(ticks) = &self.timing.ticks {
            ticks.run(true);
        }
        let Some((vblank, waited)) = self.wait_vsync() else {
            return false;
        };
        // Lost while it waited (a message taken in): drawn whole on a
        // device made again, as `busy` keeps the thread awake.
        if self.gpu.lost() {
            return true;
        }
        let woke = Instant::now();
        self.take_tick();
        if let Some(waited) = waited {
            self.timing.vsync.acquired(vblank, waited);
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
        // What the frame aims below is of the timing as it is now.
        let mut epoch = self.timing.epoch;
        // A main frame granted while the thread slept is for a vblank: the
        // frame is drawn for that one, so the UI's motion and the
        // compositor's sample the same time.
        let reserved = self.timing.reserved;
        let granted = self
            .timing
            .granted_for
            .take()
            .filter(|g| reserved.is_none_or(|r| *g > r));
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
            && let Some(b) = self.timing.blank.filter(|b| present <= *b)
        {
            let by = b.checked_sub(self.margin()).unwrap_or(b);
            match self.news_until(by) {
                None => return false,
                Some(true) => present = b,
                Some(false) => present = self.timing.vsync.next_after(b),
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
        let mut deadline = self.deadline(present, vblank);
        // What arrived while waiting (a commit, the wheel).
        while let Some(msg) = self.next_now() {
            if !self.handle(msg) {
                return false;
            }
        }
        // The output or its mode changed meanwhile: aimed again, afresh.
        if !self.timing.holds(epoch) {
            (epoch, present, deadline) = self.reaim(vblank);
        }
        if self.wants && self.owed.is_none() {
            self.grant(epoch, present, deadline);
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
            match self.next(Some(deadline)) {
                Ok(msg) => {
                    if !self.handle(msg) {
                        return false;
                    }
                    if !self.timing.holds(epoch) {
                        (epoch, present, deadline) = self.reaim(vblank);
                    }
                    if self.wants && self.owed.is_none() && self.applied.is_none() {
                        self.grant(epoch, present, deadline);
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
        // Exact on the frame clock: the physics takes spans of instants.
        self.compositor.draw(&mut self.gpu, present);
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
                    .is_some_and(|c| now.saturating_duration_since(c) < self.timing.vsync.interval());
            self.called_back = false;
        }
        // The vblank this frame was drawn for is taken when it presented or
        // holds a texture taken for it; drawn to no effect, a later frame
        // may still be drawn for it.
        if presented_now || acquired {
            self.timing.reserve(epoch, present);
            self.unpresented = 0;
        } else if surface {
            self.timing.drew_blank(epoch, present);
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
        // What input is stamped with is what the window shows, from the
        // vblank this frame was drawn for: a present made, or nothing to
        // present (its pixels are those on screen: a commit that changed
        // only what takes the pointer shows all the same). A present that
        // could not be made (no texture) is owed: the frame on screen stays
        // what input is stamped with. The UI hears of it once until it
        // looks (its hover follows the timeline, not this message).
        let owed = surface && self.gpu.owes_present();
        let presented = self.compositor.presented();
        // Every draw is logged where it fell among the events, with the
        // display time the compositor ticked scroll and motion to (Codex
        // r11 2), so a replay draws each there, at that time: the frame
        // input is routed in from its present shows what this one did.
        if let Some(f) = &self.forwarding {
            f.send(|route| route.drawn(presented.frame, present));
        }
        // The UI is woken only for a frame that changes what is under the
        // pointer: with none in the window, or one at rest over what an
        // animation leaves as it was, never.
        if !owed && self.compositor.did_present() && self.compositor.timeline().tell() {
            (self.to_ui)(FromRender::Presented);
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
    use crate::compositor::Shown;
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
            interval,
            Pace::Surface { callbacks: None },
            Some(Arc::clone(&log)),
        );
        (thread, log, tx)
    }

    /// ADR-0033's fallback: the platform sent a press and a wheel through
    /// the render thread, which panicked before it took them in. It hands
    /// them on as it diverts, so the release the platform sends next,
    /// straight to the UI, comes after them; a resize waits for it to
    /// serve again; once it does, input goes through it again.
    #[test]
    fn input_diverted_from_a_render_thread_that_panicked_keeps_its_order() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let ui: Arc<Mutex<Vec<String>>> = Arc::default();
        let to_ui = Arc::clone(&ui);
        let (tx, rx) = std::sync::mpsc::channel();
        let forwarding = Forwarding::new();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(move |m| {
                let name = match m {
                    FromRender::Input {
                        input: Input::WheelAt { step, dx, dy, .. },
                        ..
                    } => format!("WheelAt {:?} {dx} {dy} {} {:?}", step.point, step.precise, step.phase),
                    FromRender::Input { input, .. } => format!("{input:?}"),
                    other => format!("other {}", matches!(other, FromRender::Diverted { .. })),
                };
                to_ui.lock().unwrap_or_else(PoisonError::into_inner).push(name);
            }),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        let at = Instant::now();
        let send = |input: Input| {
            forwarding.send(|route| {
                let through = route.through;
                if through {
                    tx.send(ToRender::Input {
                        input,
                        at,
                        sent: at,
                        shown: Seen::BeforeFirst,
                        event: None,
                    })
                    .ok();
                } else {
                    ui.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push(format!("{input:?} straight"));
                }
                through
            })
        };
        assert!(send(Input::MouseDown { x: 1.0, y: 2.0 }));
        tx.send(ToRender::Wheel {
            point: (1.0, 2.0),
            delta: (0.0, 3.0),
            at,
            sent: at,
            precise: true,
            phase: super::super::WheelPhase::Changed,
            shown: Seen::BeforeFirst,
            event: None,
        })
        .ok();
        tx.send(ToRender::Resize((8, 8))).ok();
        thread.divert();
        assert!(!send(Input::MouseUp { x: 1.0, y: 2.0 }));
        assert_eq!(
            *ui.lock().unwrap_or_else(PoisonError::into_inner),
            [
                "MouseDown { x: 1.0, y: 2.0 }",
                "WheelAt (1.0, 2.0) 0 3 true Changed",
                "MouseUp { x: 1.0, y: 2.0 } straight"
            ],
            "in the platform's order"
        );
        assert!(
            matches!(thread.next_now(), Some(ToRender::Resize((8, 8)))),
            "kept for later"
        );
        thread.restore();
        assert!(send(Input::MouseDown { x: 1.0, y: 2.0 }), "through it again");
        // The fault matrix's finding (release, a panic before a gesture's
        // end and another before the next wheel): the input that waited
        // for the UI's latch goes back on the queue when the latch comes;
        // diverted again before it is handled, it goes to the UI in its
        // order, not to the compositor made afresh, which has no scene to
        // scroll a wheel in (one was lost so).
        tx.send(ToRender::Wheel {
            point: (1.0, 2.0),
            delta: (0.0, 4.0),
            at,
            sent: at,
            precise: false,
            phase: super::super::WheelPhase::None,
            shown: Seen::BeforeFirst,
            event: None,
        })
        .ok();
        for _ in 0..2 {
            let m = thread.next_now().expect("waiting on the channel");
            assert!(thread.handle(m));
        }
        tx.send(ToRender::Latch(None)).ok();
        let latch = thread.next_now().expect("the UI's latch");
        assert!(thread.handle(latch));
        ui.lock().unwrap_or_else(PoisonError::into_inner).clear();
        assert!(send(Input::MouseUp { x: 1.0, y: 2.0 }));
        thread.divert();
        assert_eq!(
            *ui.lock().unwrap_or_else(PoisonError::into_inner),
            [
                "MouseDown { x: 1.0, y: 2.0 }",
                "WheelAt (1.0, 2.0) 0 4 false None",
                "MouseUp { x: 1.0, y: 2.0 }"
            ],
            "what waited for the latch, then the channel, in order"
        );
        assert!(thread.next_now().is_none(), "no input left for the compositor");
    }

    /// One latch in flight at a time (the fault matrix's class): diverted
    /// again while the UI's latch is on its way back to it, the render
    /// thread tells the UI the latch is coming back, returns it as it
    /// comes, and comes through again (a restore due meanwhile) only then.
    #[test]
    fn a_latch_in_flight_when_diverted_again_goes_back_before_input_comes_through() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let ui: Arc<Mutex<Vec<String>>> = Arc::default();
        let to_ui = Arc::clone(&ui);
        let (tx, rx) = std::sync::mpsc::channel();
        let forwarding = Forwarding::new();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(move |m| {
                let name = match m {
                    FromRender::Diverted { returning, .. } => format!("diverted returning {returning}"),
                    FromRender::Returned(_) => "returned".to_owned(),
                    FromRender::Restored => "restored".to_owned(),
                    _ => "other".to_owned(),
                };
                to_ui.lock().unwrap_or_else(PoisonError::into_inner).push(name);
            }),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        let through = || forwarding.send(|route| route.through);
        thread.divert();
        thread.restore();
        assert!(through());
        // A panic before the UI's latch came back.
        thread.divert();
        assert!(!through());
        // Recovered meanwhile: its restore waits for the latch.
        thread.restore();
        assert!(!through(), "no restore with the UI's latch in flight");
        tx.send(ToRender::Latch(None)).ok();
        let latch = thread.next_now().expect("the UI's latch");
        assert!(thread.handle(latch));
        assert!(through(), "restored once it is back");
        assert_eq!(
            *ui.lock().unwrap_or_else(PoisonError::into_inner),
            ["restored", "diverted returning true", "returned", "restored"]
        );
    }

    /// Opus r13 I2, made deterministic by a gate that holds the device: a
    /// panic while the UI's latch is on its way back, and no device answers
    /// for a while. Recovering blocks nothing: the latch goes back to the
    /// UI as the panic diverts, not after a device is made (it waited ~230
    /// ms for the synchronous rebuild, the UI holding its input meanwhile).
    #[test]
    fn a_latch_in_flight_goes_back_at_once_while_no_device_answers() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let ui: Arc<Mutex<Vec<(String, Instant)>>> = Arc::default();
        let to_ui = Arc::clone(&ui);
        let (tx, rx) = std::sync::mpsc::channel();
        let forwarding = Forwarding::new();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(move |m| {
                let name = match m {
                    FromRender::Returned(_) => "returned",
                    FromRender::Failed(_) => "failed",
                    _ => "other",
                };
                to_ui
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((name.to_owned(), Instant::now()));
            }),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        thread.divert();
        thread.restore();
        // No device answers for two seconds, and the UI's latch is on its
        // way when the render thread panics.
        assert!(thread.handle(ToRender::HoldLost(Duration::from_secs(2))));
        tx.send(ToRender::Latch(None)).ok();
        let panicked = Instant::now();
        thread.divert();
        thread.recover();
        let returned = || {
            ui.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .find(|(n, _)| n == "returned")
                .map(|(_, t)| *t)
        };
        let until = panicked + Duration::from_secs(3);
        while returned().is_none() && Instant::now() < until {
            thread.step();
        }
        let took = returned().map(|t| t.saturating_duration_since(panicked));
        assert!(
            took.is_some_and(|t| t < Duration::from_millis(16)),
            "the latch went back after {took:?}"
        );
    }

    /// Codex r10 1: a render thread made afresh after a panic holds no
    /// scroll container until the UI's full commit, so a wheel it took
    /// would scroll nothing. Input stays with the UI until that commit is
    /// applied (a delta dropped meanwhile changes nothing), then comes
    /// through it again.
    #[test]
    fn input_goes_to_the_ui_until_a_recovered_compositor_can_scroll() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (_tx, rx) = std::sync::mpsc::channel();
        let forwarding = Forwarding::new();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(|_| {}),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        let through = || forwarding.send(|route| route.through);
        thread.divert();
        thread.recover();
        assert!(!through(), "nothing to scroll yet");
        let commit = |sequence: u64, full: bool| {
            let mut c = Commit::default();
            c.between(&Scene::default(), &Scene::new((64, 64), 1.0, None));
            c.sequence = sequence;
            c.full = full;
            ToRender::Commit(Box::new(c), None)
        };
        assert!(thread.handle(commit(1, false)));
        assert!(!through(), "a delta is dropped: still nothing to scroll");
        assert!(thread.handle(commit(2, true)));
        assert!(through(), "its tables whole, input comes through it again");
    }

    /// Opus r11 I3: a device lost and made again leaves the compositor its
    /// tables, its latch and its gesture: it can scroll throughout, so
    /// input keeps going through it. A wheel waiting as the device is made
    /// again is the compositor's to take, never handed to the UI to route
    /// afresh (which would pass a latched list's gesture at its end on to
    /// the list around it).
    #[test]
    fn a_device_made_again_keeps_input_going_through_the_compositor() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let ui: Arc<Mutex<Vec<String>>> = Arc::default();
        let to_ui = Arc::clone(&ui);
        let (tx, rx) = std::sync::mpsc::channel();
        let forwarding = Forwarding::new();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(move |m| {
                let name = match m {
                    FromRender::Input { input, .. } => format!("{input:?}"),
                    _ => "other".to_owned(),
                };
                to_ui.lock().unwrap_or_else(PoisonError::into_inner).push(name);
            }),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        // A present made, its callback not yet come, as the device is lost.
        thread.called_back = false;
        assert!(thread.handle(ToRender::LoseDevice));
        tx.send(ToRender::Wheel {
            point: (1.0, 2.0),
            delta: (0.0, 3.0),
            at: Instant::now(),
            sent: Instant::now(),
            precise: false,
            phase: super::super::WheelPhase::None,
            shown: Seen::BeforeFirst,
            event: None,
        })
        .ok();
        // Made on a thread of its own, taken once made.
        let until = Instant::now() + Duration::from_secs(10);
        while !thread.ensure_device() {
            assert!(Instant::now() < until, "made again");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(forwarding.send(|route| route.through), "input goes through it");
        assert!(
            ui.lock().unwrap_or_else(PoisonError::into_inner).is_empty(),
            "nothing handed to the UI to scroll"
        );
        assert!(
            matches!(thread.next_now(), Some(ToRender::Wheel { .. })),
            "the wheel is the compositor's"
        );
        // The fault matrix: the lost device's present never calls back;
        // the new device draws without waiting for it (failed: no frame
        // drawn again after a plain loss).
        assert!(thread.called_back, "no callback waited for on the new device");
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
        assert_eq!(thread.timing.vsync.phase(), None, "no phase fitted to timer wakes");
    }

    /// Scripted displays with one output of `interval` and handle `native`,
    /// and a subscription to its ticks.
    fn display(native: u32, interval: Duration) -> (gui_display::Displays, gui_display::Script, gui_display::Ticks) {
        let (mut displays, os) = gui_display::Displays::scripted(|| {});
        let mut output = gui_display::Script::output(native, "display");
        output.refresh.interval = interval;
        os.plug(output);
        displays.poll();
        let id = displays.outputs().first().map(|o| o.id).unwrap();
        let ticks = displays.ticks(id).unwrap();
        (displays, os, ticks)
    }

    #[test]
    fn moved_from_60_to_120_hz_the_next_frame_aims_at_a_120_hz_vblank() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let (mut thread, log, _tx) = on_surface(gpu, hz60);
        // On the 60 Hz display: a phase, and vblanks ahead that frames were
        // drawn and granted for.
        let now = Instant::now();
        thread.timing.vsync.acquired(now, Duration::from_millis(2));
        let old = now + hz60 * 3;
        thread.timing.reserved = Some(old);
        thread.timing.blank = Some(old);
        thread.timing.granted_for = Some(old + hz60);

        // Moved onto the 120 Hz display, which ticks.
        let (_displays, os, ticks) = display(2, hz120);
        assert!(thread.handle(ToRender::Output {
            interval: hz120,
            ticks: Some(ticks),
        }));
        let vblank = Instant::now() + Duration::from_millis(4);
        os.tick(2, gui_display::Stamp::Predicted(vblank), Some(hz120));
        assert!(thread.frame());
        let present = log.lock().unwrap().last().map(|f| f.present_at).unwrap();
        assert!(
            present < old,
            "no vblank of the old display is aimed at: {present:?}, {old:?}"
        );
        let steps = present.duration_since(vblank).as_secs_f64() / hz120.as_secs_f64();
        assert!(
            (steps - steps.round()).abs() < 1e-3,
            "a vblank of the 120 Hz grid: {steps} intervals after its tick"
        );
    }

    #[test]
    fn moved_to_120_hz_while_a_frame_waits_nothing_aimed_at_60_hz_is_granted_or_kept() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let (tx, rx) = std::sync::mpsc::channel();
        let (to_ui, from_render) = std::sync::mpsc::channel();
        let mut thread = RenderThread::new(
            Compositor::new(Glyphs::for_os()),
            gpu,
            rx,
            Box::new(move |m| {
                if let FromRender::Begin(frame) = m {
                    to_ui.send(frame.deadline).ok();
                }
            }),
            hz60,
            Pace::Surface { callbacks: None },
            None,
        );
        // On the 60 Hz display, a vblank ahead a frame was drawn for: the
        // next frame aims past it.
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let old = Instant::now() + hz60 * 3;
        thread.timing.reserved = Some(old);
        // Come while the frame aims, handled after it did: a main frame
        // asked for, and the window moved onto a 120 Hz display.
        tx.send(ToRender::NeedsFrame).unwrap();
        tx.send(ToRender::Output {
            interval: hz120,
            ticks: None,
        })
        .unwrap();
        assert!(thread.frame());
        let deadline = from_render.try_recv().expect("a main frame granted");
        assert!(
            deadline < old.checked_sub(hz120).unwrap(),
            "no main frame granted for a 60 Hz vblank aimed before the change"
        );
        for kept in [thread.timing.reserved, thread.timing.blank].into_iter().flatten() {
            assert!(kept < old, "no 60 Hz vblank written into the 120 Hz timing");
        }
    }

    #[test]
    fn moved_to_120_hz_while_a_frame_waits_for_its_commit_it_is_drawn_for_a_120_hz_vblank() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let (mut thread, log, tx) = on_surface(gpu, hz60);
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let old = Instant::now() + hz60 * 3;
        thread.timing.reserved = Some(old);
        // The main frame is granted for a 60 Hz vblank past `old`; the
        // window moves while the frame waits for its commit.
        tx.send(ToRender::NeedsFrame).unwrap();
        let mover = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            tx.send(ToRender::Output {
                interval: hz120,
                ticks: None,
            })
            .ok();
            tx
        });
        assert!(thread.frame());
        let _tx = mover.join().unwrap();
        let present = log.lock().unwrap().last().map(|f| f.present_at).unwrap();
        assert!(present < old, "drawn for a vblank of the 120 Hz timing, aimed again");
        for kept in [thread.timing.reserved, thread.timing.blank].into_iter().flatten() {
            assert!(kept < old, "no 60 Hz vblank written into the 120 Hz timing");
        }
    }

    #[test]
    fn after_a_mode_change_a_tick_of_the_old_mode_is_not_the_phase() {
        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let (mut thread, _, _tx) = on_surface(gpu, hz60);
        let (mut displays, os, ticks) = display(1, hz60);
        assert!(thread.handle(ToRender::Output {
            interval: hz60,
            ticks: Some(ticks),
        }));
        // The last tick of the 60 Hz mode, then the mode changes: the
        // platform thread polls and tells the render thread.
        os.tick(
            1,
            gui_display::Stamp::Predicted(Instant::now() + Duration::from_millis(5)),
            Some(hz60),
        );
        os.change(1, |d| d.refresh.interval = hz120);
        assert!(!displays.poll().is_empty());
        assert!(thread.handle(ToRender::Interval(hz120)));
        assert!(thread.frame());
        assert_eq!(thread.timing.vsync.phase(), None, "the 60 Hz tick is of another grid");
        assert_eq!(thread.timing.vsync.interval(), hz120);
        // The first tick of the new mode is the phase.
        let vblank = Instant::now() + Duration::from_millis(3);
        os.tick(1, gui_display::Stamp::Predicted(vblank), Some(hz120));
        assert!(thread.frame());
        assert_eq!(thread.timing.vsync.phase(), Some(vblank));
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
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let interval = Duration::from_millis(8);
        assert!(thread.handle(ToRender::Output {
            interval,
            ticks: Some(ticks),
        }));
        assert_eq!(
            (thread.timing.vsync.interval(), thread.timing.vsync.phase()),
            (interval, None),
            "nothing fitted holds"
        );

        // A timer's tick is no phase; the display's forecast is, at the
        // next frame (whose acquire fails here: no window).
        os.tick(1, gui_display::Stamp::Synthesized(Instant::now()), None);
        assert!(thread.frame());
        assert_eq!(thread.timing.vsync.phase(), None);
        let vblank = Instant::now() + Duration::from_millis(3);
        os.tick(1, gui_display::Stamp::Predicted(vblank), None);
        assert!(thread.frame());
        assert_eq!(thread.timing.vsync.phase(), Some(vblank));

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
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
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
        assert!(thread.timing.blank.is_some(), "the last frame presented nothing");
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
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        to_a_blank_frame(&mut thread);
        // The frame of nothing drew early for the next vblank, which is
        // still a long way off.
        thread.timing.vsync = Vsync::new(interval);
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        let blank = Some(thread.aim(Instant::now()));
        thread.timing.blank = blank;
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
        thread.timing.vsync.acquired(Instant::now(), Duration::from_millis(2));
        assert!(thread.frame());
        let reserved = thread.timing.reserved.expect("the frame presented");
        // Asked while T's present waits for it, in time for T otherwise.
        let early = reserved.checked_sub(Duration::from_millis(20)).unwrap_or(reserved);
        assert_eq!(
            thread.aim(early),
            thread.timing.vsync.next_after(reserved),
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
        let failed = thread.timing.blank.expect("the frame presented nothing");
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
    fn five_failed_presents_leave_the_frame_shown_stamped_and_kept() {
        // Frame A reaches the window; five commits after it are drawn and
        // none presents (no texture could be taken). Input stays stamped
        // with A, and A, all of it, stays kept for input stamped with it.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, tx) = on_surface(gpu, Duration::from_millis(4));
        let shown = thread.compositor.timeline();
        let commit = |sequence: u64, background: crate::color::Color| {
            let mut c = Commit::default();
            c.between(&Scene::default(), &Scene::new((64, 64), 1.0, Some(background)));
            c.sequence = sequence;
            c.full = true;
            ToRender::Commit(Box::new(c), None)
        };
        tx.send(commit(1, crate::color::Color::WHITE)).ok();
        assert!(thread.step());
        let a = shown.latest().expect("A presented");
        assert_eq!(a.commit, 1, "A is shown");
        for n in 2..=6_u64 {
            thread
                .gpu
                .virtual_display()
                .faults
                .extend([Fault::Timeout, Fault::Timeout]);
            tx.send(commit(n, crate::color::Color::BLACK)).ok();
            assert!(thread.step());
        }
        assert_eq!(thread.gpu.presents(), 1, "none of the five presented");
        assert!(thread.compositor.presented().frame > a.frame, "frames were drawn since");
        let Shown::Frame(stamped) = shown.at(u64::MAX) else {
            panic!("A shows")
        };
        assert!(
            Arc::ptr_eq(&stamped, &a),
            "input stays stamped with A, its own snapshot"
        );
    }

    #[test]
    fn a_commit_that_changes_only_what_takes_the_pointer_shows_from_the_next_vblank() {
        // Frame A reaches the window; then a commit adds only a box that
        // takes the pointer (no pixel changes), so nothing blits or
        // presents. The screen shows it all the same from the vblank it
        // was drawn for: it enters the timeline, and a click after that
        // lands on the box, with no other damage to carry it. A stays
        // kept for input stamped with it.
        let Ok(gpu) = GpuRenderer::with_virtual_window((64, 64)) else {
            eprintln!("no GPU: skipped");
            return;
        };
        let (mut thread, _, tx) = on_surface(gpu, Duration::from_millis(4));
        let shown = thread.compositor.timeline();
        let scene = Scene::new((64, 64), 1.0, Some(crate::color::Color::WHITE));
        let mut c = Commit::default();
        c.between(&Scene::default(), &scene);
        c.sequence = 1;
        c.full = true;
        tx.send(ToRender::Commit(Box::new(c), None)).ok();
        assert!(thread.step());
        let a = shown.latest().expect("A presented");
        let mut hit = scene.clone();
        let target = super::super::HitTarget {
            key: super::super::NodeKey(1),
            owner: None,
        };
        hit.compositing.hits = Arc::from([super::super::Hit {
            rect: crate::geometry::Bounds {
                x: 0.0,
                y: 0.0,
                width: 32.0,
                height: 32.0,
            },
            shape: crate::shape::Shape::default(),
            slop: crate::style::Edges::default(),
            transform: 0,
            clip: 0,
            effect: 0,
            scroller: None,
            wheel: false,
            target,
            cursor: None,
            below_modal: false,
        }]);
        let mut c = Commit::default();
        c.between(&scene, &hit);
        c.sequence = 2;
        tx.send(ToRender::Commit(Box::new(c), None)).ok();
        assert!(thread.step());
        assert_eq!(thread.gpu.presents(), 1, "no pixel changed: nothing presented");
        let Shown::Frame(now) = shown.at(u64::MAX) else {
            panic!("a frame shows")
        };
        assert_eq!(now.commit, 2, "the commit shows from the vblank it was drawn for");
        assert_eq!(now.hits.len(), 1, "a click there finds the box");
        let routed = thread.compositor.tables(super::super::Seen::Frame(a.frame));
        assert!(
            routed.is_some_and(|r| Arc::ptr_eq(&r, &a)),
            "input stamped with A is still routed in A"
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
        let shown = thread.compositor.timeline();
        let commit = |sequence: u64, background: crate::color::Color| {
            let mut c = Commit::default();
            c.between(&Scene::default(), &Scene::new((64, 64), 1.0, Some(background)));
            c.sequence = sequence;
            c.full = true;
            ToRender::Commit(Box::new(c), None)
        };
        // What input is stamped with once each present's vblank has come.
        let stamped = || match shown.at(u64::MAX) {
            Shown::Frame(p) => p.commit,
            _ => 0,
        };
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
        assert_eq!(thread.timing.vsync.phase(), None);
        tx.send(ToRender::Expose).ok();
        assert!(thread.step());
        let at = granted.lock().unwrap_or_else(PoisonError::into_inner).first().copied();
        let drawn = log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .map(|f| clock::nanos(f.present_at));
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
        assert_eq!(thread.timing.vsync.interval(), refresh, "key repeats are no refresh");
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
        assert_eq!(thread.timing.vsync.interval(), fast, "probes are no refresh");
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
        let fitted = thread.timing.vsync.interval();
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
        let compositor = Compositor::new(Glyphs::for_os());
        let presented = compositor.timeline();
        let thread = RenderThread::new(
            compositor,
            gpu,
            rx,
            Box::new(|_| {}),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        );
        // Each frame presented.
        let frames = || presented.latest().map_or(0, |p| p.frame);
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
        // The next turns make it again (on a thread of its own: each turn
        // looks for it), and then it may sleep.
        let until = Instant::now() + Duration::from_secs(10);
        while thread.gpu.lost() {
            assert!(Instant::now() < until, "made again");
            assert!(thread.step());
        }
        assert!(thread.frame());
        assert!(!thread.busy(), "made again and drawn: it may sleep");
    }
}
