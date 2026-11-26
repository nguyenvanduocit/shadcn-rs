//! The native window on three threads (ADR-0015).
//!
//! - The **platform thread** (the main thread, as winit and AccessKit want)
//!   runs the event loop, the accessibility adapter, the input method and
//!   the cursor. It stamps each event with when it arrived and what was on
//!   screen then ([`Presented`]), and forwards it: the wheel to the render
//!   thread, everything else to the UI thread. It never runs component
//!   code, layout or GPU work.
//! - The **UI thread** owns the reactive runtime (thread-local, `!Send`),
//!   the components, layout, paint recording, hit testing and the
//!   accessibility tree. It runs a frame when the render thread grants one
//!   and commits what changed.
//! - The **render thread** ([`crate::compositor`]) owns the GPU, the
//!   committed scene and the atlases, scrolls and moves what it can by
//!   itself, and draws at the display's rate.
//!
//! Positions handed on are logical pixels; the surface is physical.
//!
//! Where the compositor leaves the window's decorations to the client
//! (Wayland without server-side decorations), the window draws its own
//! ([`decoration`]): the platform thread finds that out and carries out
//! what they ask, the UI thread lays them out and hit-tests the edges.

pub mod decoration;

pub use decoration::{Chrome, Edge, WindowAction};

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Theme, Window, WindowAttributes, WindowId};

use crate::clock;
use crate::compositor::input_log::{self, Recorder, Routed, Router, Told as Said};
use crate::compositor::thread::{
    Feedback, Forwarding, FrameCallbacks, FrameStat, FromRender, Pace, RenderThread, ToRender,
};
use crate::compositor::{Commit, Compose, Compositor, MainFrame, Pulled, Seen, Shown};
use crate::convert::{f64_to_f32, u32_to_f32};
use crate::events::{ImeEvent, KeyEvent, Modifiers, TouchEvent};
use crate::gpu::GpuRenderer;
use crate::image::Pixels;
use crate::interact::ImeTarget;
use crate::priority::{self, Applied, ThreadType};
use crate::scene::{Drawn, Held, Scene};
use crate::style::Cursor;
use crate::text::Glyphs;

/// Input the window passes to its [`WindowHandler`], in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    PointerMoved {
        x: f32,
        y: f32,
    },
    PointerLeft,
    /// The window lost the keyboard focus: every pointer down is cancelled,
    /// as Chrome sends `pointercancel` and `lostpointercapture` then.
    Blur,
    MouseDown {
        x: f32,
        y: f32,
    },
    MouseUp {
        x: f32,
        y: f32,
    },
    /// A button other than the primary went down: the secondary (right)
    /// one, the middle one, or another (back, forward). Only a press counts;
    /// their releases do nothing in `gui`.
    ButtonDown {
        x: f32,
        y: f32,
        button: crate::layer::PointerButton,
    },
    /// Scroll by this many pixels (positive moves the content up / left)
    /// on the UI thread: the devtools' wheel. The platform's wheel scrolls
    /// on the render thread ([`crate::compositor`]).
    Wheel {
        dx: f32,
        dy: f32,
    },
    /// The platform's wheel at `(x, y)`, which the render thread hands on
    /// (it is over a wheel handler): routed from where it was, under the
    /// frame it came under, whatever the mouse did since.
    WheelAt {
        /// Where and when, and its place in its gesture: the UI latches
        /// a gesture as the compositor does.
        step: crate::compositor::WheelStep,
        dx: f32,
        dy: f32,
        /// The node it was over in that frame, which it goes to while it
        /// is there.
        target: Option<crate::compositor::HitTarget>,
    },
    Key(KeyEvent),
    /// A key held down, sent again by the platform: a key press, but Space
    /// presses the focused control once per press (a browser activates a
    /// button on Space's release), so holding it toggles nothing again.
    KeyRepeat(KeyEvent),
    /// A finger or a pen touched, moved, lifted or was taken away.
    Touch(TouchEvent),
    /// An input method composed or committed text.
    Ime(ImeEvent),
    /// Assistive technology asked a node to act (press, focus, set its value).
    Act(accesskit::ActionRequest),
    Modifiers(Modifiers),
}

impl Input {
    /// The platform's wheel, sent the UI while the compositor cannot
    /// scroll ([`Forwarding`], ADR-0033): where it was, its kind and its
    /// place in its gesture kept, as the render thread would have handed
    /// it on; the UI routes it from what is at its point.
    pub(crate) fn diverted_wheel(
        point: (f32, f32),
        delta: (f32, f32),
        at: Instant,
        precise: bool,
        phase: crate::compositor::WheelPhase,
    ) -> Self {
        Self::WheelAt {
            step: crate::compositor::WheelStep {
                point,
                time: clock::nanos(at),
                precise,
                phase,
                gesture: None,
            },
            dx: delta.0,
            dy: delta.1,
            target: None,
        }
    }
}

/// When the UI should run its next frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Wake {
    pub redraw: bool,
    /// Run the next frame too: something the UI lays out moves.
    pub animating: bool,
    pub next_timer: Option<Instant>,
}

/// What runs on the UI thread.
pub trait WindowHandler {
    /// Run the frame `begin` grants, `size` physical pixels at `scale`
    /// (physical pixels per logical pixel), and commit it to `target`.
    fn frame(&mut self, target: &mut dyn Compose, begin: MainFrame, size: (u32, u32), scale: f32);
    /// Take in what the compositor scrolled that the render thread handed
    /// on with the input or frame it rides with (ADR-0033): before that
    /// input is handled. Return `true` when a frame must follow.
    fn take_scroll(&mut self, pulled: Pulled) -> bool;
    /// Go on with the wheel gesture the compositor latched to a container,
    /// handed on as input stops going through it (ADR-0033): the rest of
    /// that gesture, which the UI scrolls, stays on that container.
    fn take_gesture(&mut self, gesture: crate::compositor::HandedGesture);
    /// Hand the wheel gesture under way back as input comes through the
    /// compositor again (ADR-0033): one side latches a gesture at a time.
    /// The handler latches none it hands, unless it keeps it (`.1`).
    fn hand_gesture(&mut self) -> Option<crate::compositor::HandedGesture> {
        None
    }
    /// React to `input`, which arrived while `presented` was on screen.
    /// Return `true` when a frame must follow.
    fn input(&mut self, input: Input, presented: &Shown) -> bool;
    /// Deliver the one value another thread sent the mailbox at `at`,
    /// here in the input's order (ADR-0033). Return `true` when a frame
    /// must follow.
    fn mail(&mut self, at: crate::mailbox::Address) -> bool {
        crate::mailbox::deliver_one(at, std::time::Duration::ZERO)
    }
    /// The cursor to show where the pointer is.
    fn cursor(&self) -> Cursor;
    /// Where an input method may type, after the last frame; `None` turns
    /// the input method off.
    fn ime_target(&mut self) -> Option<ImeTarget>;
    /// What a screen reader must be told of the last frame, at `scale`:
    /// what changed since the last update, the whole tree after a reset.
    fn access_update(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate;
    /// The screen reader connected again or went away: the next update is
    /// the whole tree.
    fn access_reset(&mut self);
    /// Run due work and say when to run the next frame. `target` is where
    /// frames go (the devtools read frames back through it).
    fn idle(&mut self, target: &mut dyn Compose) -> Wake;
    /// The window took focus again: settings the user may have changed
    /// meanwhile (text smoothing) are read again. Whether to redraw.
    fn focused(&mut self) -> bool;
    /// The platform told the window is `size` logical pixels: written, in
    /// a turn of the window (ADR-0015), to what reads its size. A handler
    /// with no window signals (a test's) takes nothing.
    fn resized(&mut self, _size: (f32, f32)) {}
    /// The platform told the window can (or cannot) be seen, as
    /// [`Self::resized`].
    fn visible(&mut self, _visible: bool) {}
    /// The render thread was rebuilt and holds nothing (ADR-0015's
    /// invariant 7): the next frame must commit everything.
    fn renderer_lost(&mut self);
    /// The window's decorations changed: whether it draws its own, and the
    /// state they show ([`decoration::decorate`] lays them out). Whether to redraw.
    fn chrome(&mut self, chrome: Chrome) -> bool;
    /// What the window's own decorations asked of the platform since the
    /// last call (a move, a resize, maximize, close), oldest first.
    fn window_actions(&mut self) -> Vec<WindowAction>;
}

/// What the UI thread is told.
pub(crate) enum ToUi {
    /// An event, when it arrived (for a drag's resampling alone), what the
    /// compositor scrolled that rides with it (ADR-0033: the render thread
    /// hands every input on; straight from the platform, nothing), and
    /// what was on screen then.
    Input(Input, Instant, Pulled, Seen),
    /// What the platform told of its window, in one order with the input
    /// (ADR-0033), and what the compositor scrolled that rides with it.
    Told(input_log::Told, Pulled),
    Render(FromRender),
    /// The mailbox at this address has a value, in one order with the
    /// input (ADR-0033), and what the compositor scrolled that rides with it.
    Mail(crate::mailbox::Address, Pulled),
    /// The app stops (an offscreen window dropped, a test's end).
    Quit,
}

/// What the UI thread tells the platform thread.
#[derive(Debug)]
pub(crate) enum ToPlatform {
    Cursor(CursorIcon),
    /// The window's own decorations ask the platform to move, resize,
    /// maximize or close the window.
    Window(WindowAction),
    /// The render thread could not go on, and why: said, and the app's
    /// exit marked failed; the window stays, the UI taking its input
    /// straight (ADR-0033), until it is closed.
    RenderFailed(String),
    Ime(Option<ImeTarget>),
    Access(Box<accesskit::TreeUpdate>),
}

/// What reaches the event loop from elsewhere: the accessibility adapter,
/// or the UI thread.
#[derive(Debug)]
pub(crate) enum UserEvent {
    Access(accesskit_winit::Event),
    Ui(ToPlatform),
    /// The displays have something to take in (ADR-0029): an output came,
    /// went or changed, or a vsync source is to be started or stopped.
    Display,
    /// The window's scroll tap took an event ([`gui_display::tap_scrolls`]):
    /// a wake, should no other event take it in first.
    Scrolled,
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(event: accesskit_winit::Event) -> Self {
        Self::Access(event)
    }
}

/// The UI thread's way to the render thread: each commit is sent behind
/// its `Arc`s, never waited for.
pub(crate) struct RenderLink {
    tx: Sender<ToRender>,
    feedback: Feedback,
    presented: Arc<crate::compositor::OnScreen>,
    /// The last commit the render thread applied and sent back.
    spare: Option<Box<Commit>>,
    /// Each input as the UI takes it to handle, and how long after it was
    /// stamped (an offscreen run's measure, [`Offscreen::take_input_latencies`]).
    latency: Option<InputLatencies>,
}

/// Inputs and how long each took from its stamp to the UI taking it.
type InputLatencies = Arc<Mutex<Vec<(Input, Duration)>>>;

impl Compose for RenderLink {
    fn commit(&mut self, commit: Box<Commit>, overlay: Option<Scene>) {
        self.tx.send(ToRender::Commit(commit, overlay)).ok();
    }

    fn spare(&mut self) -> Option<Box<Commit>> {
        self.spare.take()
    }

    fn overlay(&mut self, overlay: Option<Scene>) {
        self.tx.send(ToRender::Overlay(overlay)).ok();
    }

    fn held(&self) -> Held {
        self.feedback.held
    }

    fn drawn(&self) -> Drawn {
        self.feedback.drawn
    }

    fn capture(&mut self, overlay: Option<Scene>) -> Option<Pixels> {
        // The devtools' screenshot: the one time the UI waits for the render thread.
        let (tx, rx) = mpsc::channel();
        self.tx.send(ToRender::Capture(overlay, tx)).ok()?;
        rx.recv_timeout(Duration::from_secs(5)).ok().flatten()
    }

    fn presented(&self) -> Shown {
        self.presented.now()
    }

    fn shown_and_next(&self) -> (Shown, Option<u64>) {
        self.presented.sample(clock::nanos(Instant::now()))
    }

    fn begin(&mut self, at: u64) -> MainFrame {
        MainFrame::at(at)
    }
}

/// What the render thread hands on, as the UI handles it: input with what
/// rides with it, alike whether it came through the render thread or, while
/// that cannot serve, straight from the platform ([`Forwarding`]).
fn as_handled(msg: ToUi) -> ToUi {
    match msg {
        ToUi::Render(FromRender::Input {
            input,
            at,
            sent: _,
            shown,
            pulled,
        }) => ToUi::Input(input, at, pulled, shown),
        ToUi::Render(FromRender::Told { told, pulled }) => ToUi::Told(told, pulled),
        ToUi::Render(FromRender::Mail { at, pulled }) => ToUi::Mail(at, pulled),
        other => other,
    }
}

/// Pointer moves while a button is held, resampled at each frame (ADR-0015,
/// Android's `InputConsumer` and Chromium's `linear_resampling`): the drag
/// sees one move a frame, where the pointer was 5 ms before the frame is
/// presented, interpolated between the samples around that time or
/// extrapolated at most `min(8 ms, half the last gap)`; samples closer than
/// 2 ms are not extrapolated from. Clicks and keys are never resampled.
#[derive(Clone, Debug, Default)]
pub struct PointerResampler {
    /// (nanoseconds on the frame clock, x, y), oldest first.
    samples: Vec<(u64, f32, f32)>,
    /// A sample came since the last frame took one.
    fresh: bool,
}

/// How far before the present time a drag is sampled, seconds.
const RESAMPLE_LATENCY: f32 = 0.005;
const RESAMPLE_MAX_PREDICTION: f32 = 0.008;

impl PointerResampler {
    /// The pointer was at `(x, y)` at `at` on the frame clock.
    pub fn push(&mut self, at: impl crate::clock::Moment, x: f32, y: f32) {
        self.samples.push((at.nanos(), x, y));
        self.fresh = true;
        if self.samples.len() > 16 {
            self.samples.remove(0);
        }
    }

    /// Whether a move waits for a frame.
    #[must_use]
    pub fn holds(&self) -> bool {
        self.fresh
    }

    /// Where the pointer is for a frame presented at `present_at`, when it
    /// moved since the last frame; the samples before the one used are dropped.
    pub fn sample(&mut self, present_at: impl crate::clock::Moment) -> Option<(f32, f32)> {
        use crate::clock::span;
        if !std::mem::take(&mut self.fresh) {
            return None;
        }
        let t = crate::clock::shifted(present_at.nanos(), -RESAMPLE_LATENCY);
        let &(t1, x1, y1) = self.samples.last()?;
        let at = if t <= t1 {
            match self
                .samples
                .windows(2)
                .find(|w| matches!(w, [a, b] if a.0 <= t && t <= b.0))
            {
                Some([(ta, xa, ya), (tb, xb, yb)]) => {
                    let k = if tb > ta { span(*ta, t) / span(*ta, *tb) } else { 1.0 };
                    (xa + (xb - xa) * k, ya + (yb - ya) * k)
                }
                // Earlier than every sample: the earliest.
                _ => self.samples.first().map_or((x1, y1), |&(_, x, y)| (x, y)),
            }
        } else {
            let sample = self.samples.len().checked_sub(2).and_then(|i| self.samples.get(i));
            // Its speed over the last two samples ([`clock::rate`]: none
            // when they are closer than input is sampled).
            match sample.and_then(|&(t0, x0, y0)| Some((t0, clock::rate((x1 - x0, y1 - y0), t0, t1)?))) {
                Some((t0, v)) => {
                    let ahead = span(t1, t).min(RESAMPLE_MAX_PREDICTION).min(span(t0, t1) / 2.0);
                    (x1 + v.0 * ahead, y1 + v.1 * ahead)
                }
                None => (x1, y1),
            }
        };
        // Keep the two last samples to interpolate and extrapolate from.
        let keep = self.samples.len().saturating_sub(2);
        self.samples.drain(..keep);
        Some(at)
    }

    /// The last raw position, and forget the samples (the button went up).
    pub fn flush(&mut self) -> Option<(f32, f32)> {
        let last = self.samples.last().filter(|_| self.fresh).map(|&(_, x, y)| (x, y));
        self.samples.clear();
        self.fresh = false;
        last
    }
}

/// The moves of a held button waiting for a frame ([`PointerResampler`]),
/// with what the screen showed as they came: a move lands where that
/// frame showed each node. Moves are merged only under one frame shown:
/// one that comes under another sends those waiting first, each under its
/// own; and any input that is not such a move sends them before it
/// ([`Self::flush`]), so nothing overtakes them (a leave is never followed
/// by a move from before it).
///
/// What the compositor scrolled that rides with a move held is taken in
/// as the move comes, after what was sent before it: the move sent later
/// stands for the last one held, which came after it (ADR-0033).
#[derive(Default)]
pub(crate) struct HeldMoves {
    resampler: PointerResampler,
    /// What was on screen as the moves waiting came.
    shown: Option<Seen>,
}

/// A move to send, and what was on screen as it came.
pub(crate) type HeldMove = ((f32, f32), Seen);

impl HeldMoves {
    /// The pointer was at `(x, y)` at `at` (nanoseconds on the frame
    /// clock, the moves' one source), with `shown` on screen; the move
    /// waiting under another frame shown, to send first.
    pub(crate) fn push(&mut self, at: u64, x: f32, y: f32, shown: Seen) -> Option<HeldMove> {
        let other = self.shown.is_some_and(|s| s != shown);
        let first = if other { self.flush() } else { None };
        self.resampler.push(at, x, y);
        self.shown = Some(shown);
        first
    }

    /// The move for a frame presented at `present_at`, when one waits.
    pub(crate) fn sample(&mut self, present_at: u64) -> Option<HeldMove> {
        let at = self.resampler.sample(present_at)?;
        Some((at, self.shown?))
    }

    /// The last move waiting, now, and nothing waits after.
    pub(crate) fn flush(&mut self) -> Option<HeldMove> {
        let at = self.resampler.flush()?;
        Some((at, self.shown?))
    }
}

/// Note where `input` puts the pointer on the timeline, as the platform
/// sends it: the render thread looks there whether a frame it shows
/// changes what lies under it ([`crate::compositor::OnScreen::point`]).
fn track_pointer(presented: &crate::compositor::OnScreen, input: &Input) {
    match input {
        Input::PointerMoved { x, y }
        | Input::MouseDown { x, y }
        | Input::MouseUp { x, y }
        | Input::ButtonDown { x, y, .. } => presented.point(Some((*x, *y))),
        Input::PointerLeft => presented.point(None),
        _ => {}
    }
}

/// The UI thread's loop: run frames when granted, input as it arrives,
/// timers and mail when due, and say what the platform must show.
pub(crate) fn ui_loop(
    handler: &mut dyn WindowHandler,
    rx: &Receiver<ToUi>,
    link: &mut RenderLink,
    platform: &dyn Fn(ToPlatform),
    title: &str,
) {
    let mut size = (0_u32, 0_u32);
    let mut scale = 1.0_f32;
    let mut wants = false;
    let mut asked = false;
    let mut access = false;
    let mut pressed = false;
    let mut held = HeldMoves::default();
    // The UI's latch is on its way back from the render thread
    // ([`FromRender::Returned`]), and the input held until it is.
    let mut returning = false;
    let mut held_back: std::collections::VecDeque<ToUi> = std::collections::VecDeque::new();
    let mut cursor = None;
    // Input carries the number of the frame it came under; it is looked
    // up on the timeline only as it is handled, so nothing waiting in the
    // queue holds a frame.
    let timeline = Arc::clone(&link.presented);
    let send_move = |handler: &mut dyn WindowHandler, ((x, y), seen): HeldMove| {
        handler.input(Input::PointerMoved { x, y }, &timeline.resolve(seen))
    };
    let mut chrome = Chrome::default();
    // The resize edge under the pointer, which shows its own cursor.
    let mut edge = None;
    // Where the input method was last told it may type.
    let mut ime: Option<Option<ImeTarget>> = None;
    // Hand on what the decorations asked, and show the cursor (the edge's,
    // or what the app shows there) and the input method's area: each a
    // value of the frame on screen (ADR-0015, one reader of what was
    // shown), so each follows it wherever it changed — a frame built, an
    // input, or a frame the compositor alone made coming on screen as the
    // handler idled — and is told only when it differs.
    let follow = |handler: &mut dyn WindowHandler,
                  edge: Option<Edge>,
                  cursor: &mut Option<CursorIcon>,
                  ime: &mut Option<Option<ImeTarget>>| {
        for action in handler.window_actions() {
            platform(ToPlatform::Window(action));
        }
        let c = edge.map_or_else(|| cursor_icon(handler.cursor()), Edge::cursor);
        if *cursor != Some(c) {
            *cursor = Some(c);
            platform(ToPlatform::Cursor(c));
        }
        let target = handler.ime_target();
        if ime.as_ref() != Some(&target) {
            *ime = Some(target);
            platform(ToPlatform::Ime(target));
        }
    };
    loop {
        let wake = handler.idle(link);
        // Hover converged on the frame on screen as the handler idled (a
        // timer at a frame's vblank, a frame entered): the cursor follows
        // it with no input, as Blink's fake mouse move updates hover and
        // the cursor after layout (`local_frame_view.cc:2146-2150`).
        follow(handler, edge, &mut cursor, &mut ime);
        wants |= wake.redraw || wake.animating;
        if wants && !asked && size.0 > 0 && size.1 > 0 {
            link.tx.send(ToRender::NeedsFrame).ok();
            asked = true;
        }
        // The one queue, in the order the render thread handed things on
        // (ADR-0033); what was held for the UI's latch first, once back.
        let msg = if !returning && let Some(m) = held_back.pop_front() {
            m
        } else {
            match wake.next_timer {
                Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                    Ok(m) => m,
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(m) => m,
                    Err(_) => return,
                },
            }
        };
        let msg = as_handled(msg);
        // While its latch is on its way back, input waits for it.
        if returning
            && matches!(
                msg,
                ToUi::Input(..) | ToUi::Told(..) | ToUi::Mail(..) | ToUi::Render(FromRender::Wheel { .. })
            )
        {
            held_back.push_back(msg);
            continue;
        }
        match msg {
            ToUi::Input(input, at, pulled, presented) => {
                if let Some(l) = &link.latency {
                    l.lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((input.clone(), at.elapsed()));
                }
                if let Input::PointerMoved { x, y } = input
                    && pressed
                {
                    // A drag: one move a frame, resampled, under the frame
                    // it came under.
                    if let Some(first) = held.push(clock::nanos(at), x, y, presented) {
                        send_move(handler, first);
                    }
                    handler.take_scroll(pulled);
                    // The move waits for a frame.
                    wants = true;
                    continue;
                }
                // Anything else goes after the moves held before it, and
                // after what the wheels before it scrolled.
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.take_scroll(pulled);
                let logical = (u32_to_f32(size.0) / scale, u32_to_f32(size.1) / scale);
                match input {
                    Input::PointerMoved { x, y } => edge = decoration::edge_at(&chrome, logical, x, y),
                    Input::PointerLeft => edge = None,
                    Input::MouseDown { x, y } => {
                        // An edge of a window that draws its own frame resizes
                        // it, before anything in the app sees the press.
                        if let Some(e) = decoration::edge_at(&chrome, logical, x, y) {
                            platform(ToPlatform::Window(WindowAction::Resize(e)));
                            continue;
                        }
                        pressed = true;
                    }
                    // The press ends, released or cancelled (the moves it
                    // resampled went first).
                    Input::MouseUp { .. } | Input::Blur => pressed = false,
                    _ => {}
                }
                wants |= handler.input(input, &timeline.resolve(presented));
                follow(handler, edge, &mut cursor, &mut ime);
            }
            ToUi::Render(FromRender::Begin(token)) => {
                asked = false;
                wants = false;
                if let Some(due) = held.sample(token.present_at) {
                    send_move(handler, due);
                }
                handler.frame(link, token, size, scale);
                follow(handler, edge, &mut cursor, &mut ime);
                if access {
                    platform(ToPlatform::Access(Box::new(handler.access_update(scale, title))));
                }
            }
            ToUi::Render(FromRender::Wheel {
                input,
                shown: under,
                pulled,
            }) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.take_scroll(pulled);
                wants |= handler.input(input, &timeline.resolve(under));
                follow(handler, edge, &mut cursor, &mut ime);
            }
            // What the render thread scrolled, and the gesture it latched,
            // before input goes straight to the UI ([`Forwarding`]): after
            // the moves held before it.
            ToUi::Render(FromRender::Diverted {
                pulled,
                gesture,
                returning: back,
            }) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.take_scroll(pulled);
                if let Some(g) = gesture {
                    handler.take_gesture(g);
                }
                returning |= back;
            }
            // Its own latch, back from the render thread, which the route
            // left before it got there: the input held meanwhile goes on
            // with it, in its order.
            ToUi::Render(FromRender::Returned(gesture)) => {
                if let Some(g) = gesture {
                    handler.take_gesture(g);
                }
                returning = false;
            }
            // Input comes through the render thread again, after all the
            // UI took straight: its gesture goes back with the route, and
            // the input that came since waits for it there (ADR-0033).
            ToUi::Render(FromRender::Restored) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                link.tx.send(ToRender::Latch(handler.hand_gesture())).ok();
            }
            // Made input by [`as_handled`].
            ToUi::Render(FromRender::Input { .. } | FromRender::Told { .. } | FromRender::Mail { .. }) => {}
            ToUi::Render(FromRender::Drawn(f)) => link.feedback = *f,
            // Frames entered the timeline: the handler's idle follows it
            // (hover, at each frame's vblank).
            ToUi::Render(FromRender::Presented) => link.presented.look(),
            ToUi::Render(FromRender::Spare(c)) => link.spare = Some(c),
            ToUi::Render(FromRender::Lost) => {
                handler.renderer_lost();
                wants = true;
            }
            // The render thread gave up: the UI stays, the one owner of
            // input from now on, taking it straight (ADR-0033, Codex r13
            // 2); the window keeps the last frame it showed. Its latch is
            // its own: none is on its way back any more.
            ToUi::Render(FromRender::Failed(why)) => {
                returning = false;
                platform(ToPlatform::RenderFailed(why));
            }
            // In order with the input around it: the moves held before it
            // go first, then what the compositor scrolled before it.
            ToUi::Told(told, pulled) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.take_scroll(pulled);
                match told {
                    Said::Access(true) => {
                        access = true;
                        handler.access_reset();
                        platform(ToPlatform::Access(Box::new(handler.access_update(scale, title))));
                    }
                    Said::Access(false) => {
                        access = false;
                        handler.access_reset();
                    }
                    Said::Focused => wants |= handler.focused(),
                    // Each a turn of the window (ADR-0015): the handler's.
                    Said::Visible(visible) => {
                        handler.visible(visible);
                        wants = true;
                    }
                    Said::Resized { size: s, scale: k } => {
                        size = s;
                        scale = k;
                        if k > 0.0 {
                            let logical = |p: u32| f32::from(u16::try_from(p).unwrap_or(u16::MAX)) / k;
                            handler.resized((logical(s.0), logical(s.1)));
                        }
                        wants = true;
                    }
                    Said::Chrome(c) => {
                        wants |= handler.chrome(c.clone());
                        chrome = c;
                    }
                    Said::Quit => return,
                }
            }
            // In order with the input around it, as told is.
            ToUi::Mail(at, pulled) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.take_scroll(pulled);
                wants |= handler.mail(at);
                follow(handler, edge, &mut cursor, &mut ime);
            }
            ToUi::Quit => return,
        }
    }
}

/// Run the UI thread's own loop over `messages`, then quit, for `handler`:
/// what it holds and how it stamps what it hands on, with no threads
/// (tests).
#[cfg(test)]
pub(crate) fn run_ui_loop(handler: &mut dyn WindowHandler, messages: Vec<ToUi>) {
    let (to_ui, rx) = mpsc::channel();
    let (tx, _render) = mpsc::channel();
    let mut link = RenderLink {
        tx,
        feedback: Feedback::default(),
        presented: crate::compositor::OnScreen::new(),
        spare: None,
        latency: None,
    };
    for m in messages.into_iter().chain([ToUi::Quit]) {
        to_ui.send(m).ok();
    }
    ui_loop(handler, &rx, &mut link, &|_| {}, "test");
}

/// The threads of an app, and what links them.
struct Threads {
    to_ui: Sender<ToUi>,
    to_render: Sender<ToRender>,
    /// What the device's lost callback wakes the render thread through.
    #[expect(dead_code, reason = "held: the lost callback holds it weakly")]
    waker: Arc<Sender<ToRender>>,
    presented: Arc<crate::compositor::OnScreen>,
    /// How the OS schedules each thread, as each set it when it started.
    priorities: Arc<Mutex<Vec<(&'static str, Applied)>>>,
    /// Where input goes: through the render thread, or straight to the UI
    /// while it cannot serve (ADR-0033).
    forwarding: Arc<Forwarding>,
    /// What the UI's input waited, where it is measured.
    latency: Option<InputLatencies>,
    /// What input waited to be handed on by the render thread, there.
    forwarded: Option<InputLatencies>,
}

impl Threads {
    /// Send `input`, which came at `at` with `shown` on screen and was made
    /// of `event`, through the render thread; straight to the UI while it
    /// cannot serve ([`Forwarding`]).
    fn input(&self, input: Input, at: Instant, shown: Seen, event: Option<input_log::Event>) {
        // The hop starts here, the switch's lock included (ADR-0033).
        let sent = Instant::now();
        self.forwarding.send(|route| {
            if route.through {
                let event = event.map(Box::new);
                self.to_render
                    .send(ToRender::Input {
                        input,
                        at,
                        sent,
                        shown,
                        event,
                    })
                    .ok();
            } else {
                // Taken here: logged here.
                if let Some(e) = &event {
                    route.record(at, e, shown);
                }
                self.to_ui.send(ToUi::Input(input, at, Pulled::default(), shown)).ok();
            }
        });
    }

    /// Send what the platform told of its window, which came at `at` with
    /// `shown` on screen, as input is sent: through the render thread, which
    /// takes its own part of it (a size, being seen) and hands the UI's on;
    /// while it cannot serve, its own part to it alone and the rest straight
    /// to the UI, in order with the input.
    fn told(&self, told: Said, at: Instant, shown: Seen, logged: bool) {
        self.forwarding.send(|route| {
            let event = logged.then(|| input_log::Event::Told(told.clone()));
            if route.through {
                self.to_render
                    .send(ToRender::Told {
                        told,
                        at,
                        shown,
                        event: event.map(Box::new),
                    })
                    .ok();
            } else {
                if let Some(e) = &event {
                    route.record(at, e, shown);
                }
                if let Some(own) = crate::compositor::thread::own_part(&told) {
                    self.to_render.send(own).ok();
                }
                self.to_ui.send(ToUi::Told(told, Pulled::default())).ok();
            }
        });
    }

    /// Send the render thread a wheel; while it cannot serve, the UI
    /// scrolls it, in order with the rest.
    fn wheel(&self, wheel: Wheel, event: Option<input_log::Event>) {
        let sent = Instant::now();
        self.forwarding.send(|route| {
            if route.through {
                self.to_render
                    .send(ToRender::Wheel {
                        point: wheel.point,
                        delta: wheel.delta,
                        at: wheel.at,
                        sent,
                        precise: wheel.precise,
                        phase: wheel.phase,
                        shown: wheel.shown,
                        event: event.map(Box::new),
                    })
                    .ok();
            } else {
                if let Some(e) = &event {
                    route.record(wheel.at, e, wheel.shown);
                }
                route.took_wheel(wheel.precise, wheel.phase);
                let input = Input::diverted_wheel(wheel.point, wheel.delta, wheel.at, wheel.precise, wheel.phase);
                self.to_ui
                    .send(ToUi::Input(input, wheel.at, Pulled::default(), wheel.shown))
                    .ok();
            }
        });
    }
}

/// A wheel the platform sends ([`ToRender::Wheel`]).
struct Wheel {
    point: (f32, f32),
    delta: (f32, f32),
    at: Instant,
    precise: bool,
    phase: crate::compositor::WheelPhase,
    shown: Seen,
}

/// The most hops held between two writes ([`write_hops`]): past it a hop
/// is dropped, not held (a writer that stalls costs no memory).
const HOPS_HELD: usize = 1 << 16;

/// Append the hops `hops` holds to `path` each second, one a line (the
/// input's kind and microseconds): taken out under the lock, formatted
/// and written outside it, so the render thread waits on no formatting
/// and no file. It stops with the window: once it alone holds `hops`, it
/// writes what is left and ends.
fn write_hops(path: std::path::PathBuf, hops: InputLatencies) {
    let write = move || {
        let Ok(mut file) = std::fs::File::create(&path) else {
            return;
        };
        let mut taken = Vec::new();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let last = Arc::strong_count(&hops) == 1;
            std::mem::swap(&mut *hops.lock().unwrap_or_else(PoisonError::into_inner), &mut taken);
            let text: String = taken
                .drain(..)
                .map(|(input, d)| {
                    let kind = format!("{input:?}");
                    let kind = kind.split([' ', '(', '{']).next().unwrap_or("").to_owned();
                    format!("{kind} {}\n", d.as_micros())
                })
                .collect();
            if std::io::Write::write_all(&mut file, text.as_bytes()).is_err() || last {
                return;
            }
        }
    };
    let spawned = std::thread::Builder::new()
        .name("gui input hops".to_owned())
        .spawn(write);
    if let Err(e) = spawned {
        eprintln!("GUI_INPUT_HOPS: {e}");
    }
}

/// Start the render and UI threads: the render thread draws with `gpu`
/// paced by `pace`; the UI thread runs `start`'s handler. `platform` takes
/// what the UI thread tells the platform.
fn spawn(
    gpu: GpuRenderer,
    pace: Pace,
    interval: Duration,
    title: String,
    start: Box<dyn FnOnce() -> Box<dyn WindowHandler> + Send>,
    platform: Box<dyn Fn(ToPlatform) + Send>,
    log: Option<Arc<Mutex<Vec<FrameStat>>>>,
) -> std::io::Result<Threads> {
    let (to_ui, ui_rx) = mpsc::channel::<ToUi>();
    let (to_render, render_rx) = mpsc::channel::<ToRender>();
    let compositor = Compositor::new(Glyphs::for_os());
    let presented = compositor.timeline();
    let forwarding = Forwarding::new();
    let priorities = Arc::new(Mutex::new(Vec::new()));
    // Measured where frames are logged (offscreen runs).
    let latency = log.is_some().then(InputLatencies::default);
    // And the hop to the UI: from the platform's send to handed on, the
    // render thread's hop alone (ADR-0033); on a window too when
    // `GUI_INPUT_HOPS` names a file (its measure on a real surface),
    // written there each second by a thread of its own. Not from the
    // input's time: an OS's stamp (a tapped scroll's) or a script's comes
    // before the send, and the wait between is not this hop's.
    let hops_file = std::env::var_os("GUI_INPUT_HOPS").filter(|_| log.is_none());
    let held = if hops_file.is_some() { HOPS_HELD } else { usize::MAX };
    let forwarded = (log.is_some() || hops_file.is_some()).then(InputLatencies::default);
    if let (Some(path), Some(hops)) = (hops_file, forwarded.clone()) {
        write_hops(std::path::PathBuf::from(path), hops);
    }
    let hops = forwarded.clone();
    let ui = to_ui.clone();
    // A device lost while the render thread sleeps on its channel wakes
    // it, to be made again at once. Held weakly: the device does not keep
    // the channel open once the app has let go of it.
    let waker = Arc::new(to_render.clone());
    let wake = Arc::downgrade(&waker);
    gpu.wake_on_lost(move || {
        if let Some(tx) = wake.upgrade() {
            tx.send(ToRender::Wake).ok();
        }
    });
    let render = RenderThread::new(
        compositor,
        gpu,
        render_rx,
        Box::new(move |m| {
            if let (Some(h), FromRender::Input { input, sent, .. }) = (&hops, &m) {
                let mut h = h.lock().unwrap_or_else(PoisonError::into_inner);
                if h.len() < held {
                    h.push((input.clone(), sent.elapsed()));
                }
            }
            ui.send(ToUi::Render(m)).ok();
        }),
        interval,
        pace,
        log,
    )
    .forwarding(Arc::clone(&forwarding));
    let noted = Arc::clone(&priorities);
    std::thread::Builder::new()
        .name("gui render".to_owned())
        .spawn(move || {
            note(&noted, "render", priority::set_current(ThreadType::Render));
            render.run();
        })?;
    let mut link = RenderLink {
        tx: to_render.clone(),
        feedback: Feedback::default(),
        presented: Arc::clone(&presented),
        spare: None,
        latency: latency.clone(),
    };
    // Mail is input from the outside world: through the one producer, as
    // the platform's input is, logged where it is taken, so the UI takes
    // it in order with the input and a replay where the log has it
    // (ADR-0033, Codex r12 7).
    let (mail, mail_render, mail_route) = (to_ui.clone(), to_render.clone(), Arc::clone(&forwarding));
    let render_quit = to_render.clone();
    let noted = Arc::clone(&priorities);
    std::thread::Builder::new()
        .name("gui ui".to_owned())
        .stack_size(crate::layout::STACK)
        .spawn(move || {
            note(&noted, "ui", priority::set_current(ThreadType::Ui));
            crate::mailbox::wake_with(Arc::new(move |at| {
                mail_route.send(|route| {
                    if route.through {
                        mail_render.send(ToRender::Mail(at)).ok();
                    } else {
                        // Taken here: logged here.
                        route.observe(input_log::Observation::Mail(at.site, at.nth));
                        mail.send(ToUi::Mail(at, Pulled::default())).ok();
                    }
                });
            }));
            let mut handler = start();
            ui_loop(handler.as_mut(), &ui_rx, &mut link, platform.as_ref(), &title);
            render_quit.send(ToRender::Quit).ok();
        })?;
    Ok(Threads {
        to_ui,
        to_render,
        waker,
        presented,
        priorities,
        forwarding,
        latency,
        forwarded,
    })
}

/// Keep what thread `name` was given, for reports.
fn note(to: &Mutex<Vec<(&'static str, Applied)>>, name: &'static str, applied: Applied) {
    to.lock().unwrap_or_else(PoisonError::into_inner).push((name, applied));
}

struct WindowState {
    /// The window's scroll events taken from the OS whole, where it tells
    /// more than winit (macOS): winit then sees none of them. First, so it
    /// is dropped (its monitor removed) before the window is.
    _scrolls: Option<gui_display::ScrollTap>,
    window: Arc<Window>,
    /// Tells the platform's accessibility API what the window shows.
    access: accesskit_winit::Adapter,
    threads: Threads,
}

/// The platform thread's side of the window.
struct Platform {
    title: String,
    start: Option<Box<dyn FnOnce() -> Box<dyn WindowHandler> + Send>>,
    state: Option<WindowState>,
    /// The pointer and the modifiers, which input is routed with.
    router: Router,
    /// An input log is recorded (`GUI_RECORD_INPUT`): the render thread
    /// writes it, each event in the order it takes them (ADR-0033), and
    /// each is sent to it with what it routes.
    recording: bool,
    /// Where the input method was last told it may type.
    ime: Option<ImeTarget>,
    proxy: EventLoopProxy<UserEvent>,
    /// What the adapter last said: whether a screen reader listens.
    listening: bool,
    /// The render thread gave up: the app exits with an error when its
    /// window closes.
    failed: bool,
    /// The window's decorations as the UI thread was last told.
    chrome: Chrome,
    /// `GUI_DECORATIONS` chose who draws them.
    forced: Option<bool>,
    focused: bool,
    /// The OS's displays (ADR-0029), where it reads them: made with the
    /// window, on this thread, which owns them.
    displays: Option<gui_display::Displays>,
    /// What the render thread was last told of the window's display.
    told: Told,
    /// The scroll events the window's tap took, not yet taken in
    /// ([`Platform::take_scrolls`]): each is taken in before the next
    /// event winit hands on, so input keeps the order the OS sent it in.
    tapped: Rc<RefCell<VecDeque<gui_display::Scroll>>>,
}

/// Open a window titled `title` and run the handler `start` makes, on the
/// UI thread, until the window closes. `start` runs there, so everything
/// the app's state holds (signals, components) is made on the thread that
/// owns it.
pub fn run(title: &str, start: impl FnOnce() -> Box<dyn WindowHandler> + Send + 'static) -> ! {
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("failed to create event loop: {e}");
            std::process::exit(1);
        }
    };
    let mut platform = Platform {
        title: title.to_owned(),
        start: Some(Box::new(start)),
        state: None,
        router: Router::default(),
        recording: false,
        ime: None,
        proxy: event_loop.create_proxy(),
        listening: false,
        failed: false,
        chrome: Chrome::default(),
        forced: forced_decorations(),
        focused: true,
        displays: None,
        told: Told::default(),
        tapped: Rc::default(),
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    // The platform thread stamps input and feeds the render thread
    // (Chromium's browser UI thread). On macOS the main thread is already
    // user-interactive.
    let applied = priority::set_current(ThreadType::Platform);
    if applied.degraded {
        eprintln!("platform thread: {applied}");
    }
    if let Err(e) = event_loop.run_app(&mut platform) {
        eprintln!("event loop error: {e}");
        std::process::exit(1);
    }
    std::process::exit(i32::from(platform.failed))
}

/// Who draws the decorations as `GUI_DECORATIONS` asks: `client` has the
/// window draw its own on any platform (to see them off Wayland; where the
/// platform cannot resize from an edge, the edge does nothing), `server`
/// never. Unset, the window draws them where the compositor leaves them to it.
fn forced_decorations() -> Option<bool> {
    match std::env::var("GUI_DECORATIONS").ok()?.as_str() {
        "client" => Some(true),
        "server" => Some(false),
        _ => None,
    }
}

/// Whether the compositor left the decorations to the client: on Wayland,
/// winit asks for server-side decorations (xdg-decoration) and, refused or
/// without the protocol, puts a frame of its own around the surface, which
/// makes the outer size larger than the inner.
fn compositor_left_decorations(window: &Window) -> bool {
    is_wayland(window) && window.is_decorated() && window.outer_size() != window.inner_size()
}

fn cursor_icon(c: Cursor) -> CursorIcon {
    match c {
        Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::Text => CursorIcon::Text,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::ResizeColumn => CursorIcon::ColResize,
        Cursor::ResizeRow => CursorIcon::RowResize,
        Cursor::ResizeEastWest => CursorIcon::EwResize,
        Cursor::ResizeNorthSouth => CursorIcon::NsResize,
        Cursor::ResizeEast => CursorIcon::EResize,
        Cursor::ResizeWest => CursorIcon::WResize,
        Cursor::ResizeNorth => CursorIcon::NResize,
        Cursor::ResizeSouth => CursorIcon::SResize,
        Cursor::ResizeNorthEast => CursorIcon::NeResize,
        Cursor::ResizeNorthWest => CursorIcon::NwResize,
        Cursor::ResizeSouthEast => CursorIcon::SeResize,
        Cursor::ResizeSouthWest => CursorIcon::SwResize,
        Cursor::Move => CursorIcon::Move,
    }
}

/// Whether `window` is a Wayland surface (frame callbacks pace it).
fn is_wayland(window: &Window) -> bool {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    window
        .window_handle()
        .is_ok_and(|h| matches!(h.as_raw(), RawWindowHandle::Wayland(_)))
}

/// Whether the window may be on another display after `event`: moved
/// (onto another screen of the same scale, which neither resizes nor
/// rescales it), resized or rescaled (onto one of another scale), or shown
/// again. Wayland sends no `Moved`; its window learns its output only from
/// a rescale there.
fn display_may_change(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Occluded(false)
    )
}

/// What the render thread was last told of the display the window is on:
/// the output (and the OS handle it had, which names its vsync source),
/// and its interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Told {
    output: Option<(gui_display::OutputId, u32)>,
    interval: Option<Duration>,
}

/// What to tell the render thread of the window's display, and only what
/// it was not told (telling it an interval again resets its fitted vsync,
/// and each `Moved` of a drag asks). The window is on the monitor with the
/// OS handle `native`, for which the toolkit reports `reported`.
///
/// Where the OS's displays are read (ADR-0029) the output is theirs: its
/// interval is the current mode's, and on another output, or the same one
/// under another handle, the render thread gets that output's vsync ticks
/// (where its OS has a source) and starts its fit afresh. Elsewhere
/// (Wayland) the toolkit's rate is all there is.
///
/// The displays are polled first, so what the OS said and was not taken in
/// yet (a monitor winit already reports, before the displays' own wake is
/// handled) is known before anything is told: the render thread is told
/// of the output once, not of winit's rate and then of the output.
fn follow(
    mut displays: Option<&mut gui_display::Displays>,
    native: Option<u32>,
    reported: Option<Duration>,
    told: &mut Told,
) -> Option<ToRender> {
    if let Some(d) = displays.as_deref_mut() {
        // The events need no reading: the outputs are read below as they are now.
        d.poll();
    }
    let output = displays
        .as_ref()
        .zip(native)
        .and_then(|(d, n)| d.by_native(n))
        .map(|o| (o.id, o.native, o.refresh.interval));
    let now = Told {
        output: output.map(|(id, native, _)| (id, native)),
        interval: output.map(|(.., interval)| interval).or(reported).or(told.interval),
    };
    let interval = now.interval?;
    let was = std::mem::replace(told, now);
    if now.output != was.output {
        // No source for this output's vsync (X11), or none could be made:
        // the acquires are fitted, as on any display before ADR-0029.
        let ticks = displays.zip(now.output).and_then(|(d, (id, _))| d.ticks(id).ok());
        Some(ToRender::Output { interval, ticks })
    } else if now.interval != was.interval {
        Some(ToRender::Interval(interval))
    } else {
        None
    }
}

/// The OS's handle of the monitor `window` is on, as `gui_display` names
/// outputs: macOS's display id, X11's CRTC.
/// A user event the displays answer: their wake (`UserEvent::Display`) is
/// `follow` on what the window reports (`window`, read only then, `None`
/// with no window), its news for the render thread. What came, went or
/// changed is read off the displays as they are now (`follow` polls them,
/// which also starts or stops a vsync source), and only what the render
/// thread was not told is sent. Any other event is handed back.
fn on_display(
    event: UserEvent,
    displays: Option<&mut gui_display::Displays>,
    window: impl FnOnce() -> Option<(Option<u32>, Option<Duration>)>,
    told: &mut Told,
) -> Result<Option<ToRender>, UserEvent> {
    match event {
        UserEvent::Display => Ok(window().and_then(|(native, reported)| follow(displays, native, reported, told))),
        other => Err(other),
    }
}

fn native_of(window: &Window) -> Option<u32> {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::MonitorHandleExtMacOS;
        window.current_monitor().map(|m| m.native_id())
    }
    #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
    {
        use winit::platform::x11::MonitorHandleExtX11;
        window.current_monitor().map(|m| m.native_id())
    }
    // No displays are read here yet (ADR-0029): no handle names one.
    #[cfg(not(any(target_os = "macos", all(unix, not(target_os = "android")))))]
    {
        window.current_monitor().and(None)
    }
}

/// The display interval of the monitor `window` is on, when the platform
/// reports it.
fn interval_of(window: &Window) -> Option<Duration> {
    let mhz = window.current_monitor().and_then(|m| m.refresh_rate_millihertz())?;
    Some(clock::Vsync::from_millihertz(mhz).interval())
}

/// The input log `GUI_RECORD_INPUT` names, started for `window` (ADR-0015:
/// a log of the platform's own events, replayed by the compositor's tests).
fn record_input(window: &Window) -> Option<Recorder> {
    let path = std::env::var_os("GUI_RECORD_INPUT")?;
    let platform = if cfg!(target_os = "macos") {
        input_log::Platform::MacOs
    } else if cfg!(windows) {
        input_log::Platform::Windows
    } else if is_wayland(window) {
        input_log::Platform::Wayland
    } else if cfg!(unix) {
        input_log::Platform::X11
    } else {
        input_log::Platform::Other
    };
    let scale = f64_to_f32(window.scale_factor());
    let size = window.inner_size().to_logical::<f64>(window.scale_factor());
    let header = input_log::Header {
        platform,
        size: (
            u32::try_from(crate::convert::f64_to_u64(size.width.round())).unwrap_or(u32::MAX),
            u32::try_from(crate::convert::f64_to_u64(size.height.round())).unwrap_or(u32::MAX),
        ),
        scale,
        source: format!("recorded with GUI_RECORD_INPUT, gui {}", env!("CARGO_PKG_VERSION")),
    };
    match Recorder::start(std::path::Path::new(&path), &header) {
        Ok(r) => {
            eprintln!("recording input to {}", std::path::Path::new(&path).display());
            Some(r)
        }
        Err(e) => {
            eprintln!(
                "GUI_RECORD_INPUT: cannot write {}: {e}",
                std::path::Path::new(&path).display()
            );
            None
        }
    }
}

/// What a scroll event the window's tap took becomes, as winit's view
/// would make of it (`scrollWheel:`, winit 0.30.13 `view.rs`): the
/// modifiers the event holds, when they differ from those last told
/// (`update_modifiers`); the pointer moved to where it is, when it is
/// elsewhere and inside the view or a button is down (`mouse_motion`,
/// which skips a place outside the view with no button down); then the
/// wheel, with both of the OS's phases. In that order (Opus r7 N3).
fn tapped_events(s: &gui_display::Scroll, cursor: (f32, f32), held: Modifiers) -> Tapped {
    let m = s.modifiers;
    let now = Modifiers {
        shift: m.shift,
        control: m.control,
        alt: m.alt,
        command: m.command,
    };
    let modifiers = (now != held).then_some(input_log::Event::Modifiers(now));
    let moved = s
        .pointer()
        .map(|p| (f64_to_f32(p.0), f64_to_f32(p.1)))
        .filter(|&p| p != cursor)
        .map(|p| input_log::Event::Moved(p.0, p.1));
    let (x, y) = (f64_to_f32(s.delta.0), f64_to_f32(s.delta.1));
    let delta = if s.precise {
        input_log::WheelDelta::Pixels(x, y)
    } else {
        input_log::WheelDelta::Lines(x, y)
    };
    let wheel = input_log::Event::Wheel {
        delta,
        phase: wheel_phase(s),
    };
    Tapped {
        modifiers,
        moved,
        wheel,
    }
}

/// The events a tapped scroll event becomes ([`tapped_events`]), in order.
#[derive(Debug)]
struct Tapped {
    modifiers: Option<input_log::Event>,
    moved: Option<input_log::Event>,
    wheel: input_log::Event,
}

/// A scroll event's place in its gesture as the OS told it: its momentum's
/// phase while momentum runs (the two are never both set), else its
/// fingers'.
fn wheel_phase(s: &gui_display::Scroll) -> crate::compositor::WheelPhase {
    use crate::compositor::WheelPhase as W;
    use gui_display::ScrollPhase as P;
    match (s.momentum, s.phase) {
        // AppKit sends no `MayBegin` momentum; were it to, it starts the
        // momentum all the same.
        (P::MayBegin | P::Began, _) => W::MomentumBegan,
        (P::Changed | P::Stationary, _) => W::Momentum,
        (P::Ended | P::Cancelled, _) => W::MomentumEnded,
        // Fingers touched, not yet moving (`MayBegin`, a touch-to-stop):
        // the gesture's first contact. It ends the gesture before and
        // stops its fling, though nothing takes it; `Began` follows if the
        // fingers move (the same gesture, begun at this touch), `Cancelled`
        // if they lift without moving.
        (P::None, P::MayBegin) => W::Touched,
        (P::None, P::Began) => W::Began,
        (P::None, P::Changed | P::Stationary) => W::Changed,
        (P::None, P::Ended) => W::Ended,
        (P::None, P::Cancelled) => W::Cancelled,
        (P::None, P::None) => W::None,
    }
}

impl Platform {
    /// Take in the scroll events the window's tap took, in the order the
    /// OS sent them, each stamped with its own time.
    fn take_scrolls(&mut self) {
        loop {
            let Some(s) = self.tapped.borrow_mut().pop_front() else {
                return;
            };
            let t = tapped_events(&s, self.router.cursor(), self.router.modifiers());
            for e in [t.modifiers, t.moved].into_iter().flatten() {
                self.forward(e, s.at);
            }
            self.forward(t.wheel, s.at);
        }
    }

    /// Send what `event` becomes on, through the render thread (ADR-0033),
    /// stamped `at` (when the OS says it happened where it says so, else
    /// when it came) and with the frame on screen then, which it is routed
    /// in; with `event` itself while an input log is recorded there.
    fn forward(&mut self, event: input_log::Event, at: Instant) {
        let logged = self.recording.then(|| event.clone());
        let routed = self.router.route(event);
        let Some(s) = &self.state else { return };
        let shown = s.threads.presented.seen_at(clock::nanos(at));
        match routed {
            Routed::Ui(input) => {
                track_pointer(&s.threads.presented, &input);
                s.threads.input(input, at, shown, logged);
            }
            Routed::Wheel {
                point,
                delta,
                precise,
                phase,
            } => s.threads.wheel(
                Wheel {
                    point,
                    delta,
                    at,
                    precise,
                    phase,
                    shown,
                },
                logged,
            ),
            Routed::Told(told) => s.threads.told(told, at, shown, logged.is_some()),
        }
    }

    /// Tell the UI what the platform found of its window now, in order
    /// with the input ([`Threads::told`]), logged as input is.
    fn tell(&self, told: Said) {
        let Some(s) = &self.state else { return };
        let at = Instant::now();
        let shown = s.threads.presented.seen_at(clock::nanos(at));
        s.threads.told(told, at, shown, self.recording);
    }

    fn scale(&self) -> f32 {
        self.state.as_ref().map_or(1.0, |s| f64_to_f32(s.window.scale_factor()))
    }

    /// Tell the UI thread what the decorations show, when that changed.
    fn follow_chrome(&mut self) {
        let Some(s) = &self.state else { return };
        let w = &s.window;
        // With its own frame hidden, winit reports the window decorated
        // again only once the compositor draws the decorations after all.
        let client = self.chrome.client && (self.forced.is_some() || !w.is_decorated());
        let chrome = Chrome {
            client,
            title: self.title.clone(),
            maximized: w.is_maximized(),
            fullscreen: w.fullscreen().is_some(),
            active: self.focused,
            dark: w.theme() == Some(Theme::Dark),
            resizable: w.is_resizable(),
        };
        if chrome != self.chrome {
            self.tell(Said::Chrome(chrome.clone()));
            self.chrome = chrome;
        }
    }

    /// Do what the window's own decorations asked.
    fn act(&self, event_loop: &ActiveEventLoop, action: WindowAction) {
        let Some(s) = &self.state else { return };
        let w = &s.window;
        let done = match action {
            WindowAction::Move => w.drag_window(),
            WindowAction::Resize(edge) => w.drag_resize_window(edge.direction()),
            WindowAction::Minimize => {
                w.set_minimized(true);
                Ok(())
            }
            WindowAction::ToggleMaximize => {
                w.set_maximized(!w.is_maximized());
                Ok(())
            }
            WindowAction::Close => {
                self.close(event_loop);
                Ok(())
            }
            WindowAction::Menu { x, y } => {
                w.show_window_menu(LogicalPosition::new(x, y));
                Ok(())
            }
        };
        if let Err(e) = done {
            eprintln!("window {action:?}: {e}");
        }
    }

    fn close(&self, event_loop: &ActiveEventLoop) {
        self.tell(Said::Quit);
        event_loop.exit();
    }

    /// The window's size or scale changed: the render thread configures
    /// its surface to the size as it takes it (a swapchain not the window's
    /// size is out of date on Vulkan), showing the last frame at its own
    /// size until the UI commits one laid out at the new size and scale,
    /// which the UI takes in order with the input around it.
    fn resized(&self) {
        let Some(s) = &self.state else { return };
        let size = s.window.inner_size();
        let size = (size.width, size.height);
        self.tell(Said::Resized {
            size,
            scale: self.scale(),
        });
    }

    /// Tell the render thread which display the window is on now and how
    /// often it refreshes, when that changed.
    fn follow_display(&mut self) {
        let Some(s) = &self.state else { return };
        let news = follow(
            self.displays.as_mut(),
            native_of(&s.window),
            interval_of(&s.window),
            &mut self.told,
        );
        if let Some(news) = news {
            s.threads.to_render.send(news).ok();
        }
    }

    /// Tell the platform's input method where it may type: on while an
    /// input that takes composed text has focus, its candidates at the
    /// caret, and started afresh when focus moves to another input so a
    /// composition never carries over.
    fn follow_ime(&mut self, target: Option<ImeTarget>) {
        let Some(state) = self.state.as_ref() else { return };
        if target == self.ime {
            return;
        }
        let moved = target.map(|t| t.field) != self.ime.map(|t| t.field);
        if moved && self.ime.is_some() {
            state.window.set_ime_allowed(false);
        }
        if let Some(t) = target {
            if moved {
                state.window.set_ime_allowed(true);
            }
            state.window.set_ime_cursor_area(
                LogicalPosition::new(t.area.x, t.area.y),
                LogicalSize::new(t.area.width, t.area.height),
            );
        }
        self.ime = target;
    }
}

impl ApplicationHandler<UserEvent> for Platform {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Some(start) = self.start.take() else { return };
        // Hidden until the accessibility adapter is attached, which it must
        // be before the window first shows.
        let attrs = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_inner_size(LogicalSize::new(1280.0, 820.0))
            .with_visible(false);
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("create_window: {e}");
                event_loop.exit();
                return;
            }
        };
        // winit has the compositor's first configure by now, so whether it
        // took the decorations is known before the first frame: if not, the
        // window draws its own and hides winit's.
        let client = self.forced.unwrap_or_else(|| compositor_left_decorations(&window));
        if client {
            window.set_decorations(false);
        }
        self.focused = window.has_focus();
        // The surface is made here, on the main thread, as Metal wants;
        // the render thread draws into it from then on.
        let gpu = match GpuRenderer::for_window(Arc::clone(&window)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("no GPU to paint the window on: {e}");
                event_loop.exit();
                return;
            }
        };
        let access = accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());
        let proxy = self.proxy.clone();
        let callbacks: Option<Arc<dyn FrameCallbacks>> = if is_wayland(&window) {
            let w: Arc<Window> = Arc::clone(&window);
            Some(w)
        } else {
            None
        };
        let pace = Pace::Surface { callbacks };
        // 60 Hz until the platform says (Wayland's frame callbacks then fit it).
        let reported = interval_of(&window);
        let threads = match spawn(
            gpu,
            pace,
            reported.unwrap_or(Duration::from_micros(16_667)),
            self.title.clone(),
            start,
            Box::new(move |m| {
                proxy.send_event(UserEvent::Ui(m)).ok();
            }),
            None,
        ) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("cannot start the UI and render threads: {e}");
                event_loop.exit();
                return;
            }
        };
        window.set_visible(true);
        if let Some(r) = record_input(&window) {
            // Written where each event is taken, under the route's lock:
            // by the render thread, which also says where it granted
            // frames, drew them and took commits, or by this thread while
            // input goes straight to the UI (ADR-0033).
            threads.forwarding.send(|route| route.record_with(r));
            self.recording = true;
        }
        // The OS's displays, where they are read: the wake has this thread
        // poll them. Where they are not (Wayland), winit's rate and the
        // frame callbacks stand, as before.
        let wake = self.proxy.clone();
        self.displays = gui_display::Displays::connect(&window, move || {
            wake.send_event(UserEvent::Display).ok();
        })
        .ok();
        // The window's scroll events whole, where the OS tells more than
        // winit hands on: queued as they come, taken in before the next
        // event winit hands on, and woken for should none come.
        let tapped = Rc::clone(&self.tapped);
        let wake = self.proxy.clone();
        let scrolls = gui_display::tap_scrolls(u64::from(window.id()), move |s| {
            tapped.borrow_mut().push_back(s);
            wake.send_event(UserEvent::Scrolled).ok();
        });
        self.state = Some(WindowState {
            window,
            access,
            threads,
            _scrolls: scrolls,
        });
        // The monitor's own rate, as reported: what bounds the fit.
        self.follow_display();
        // Sent before the size, so the first frame is laid out with them.
        self.chrome.client = client;
        self.follow_chrome();
        self.resized();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        self.take_scrolls();
        if let Some(state) = self.state.as_mut() {
            state.access.process_event(&state.window, &event);
        }
        let scale = self.scale();
        if display_may_change(&event) {
            self.follow_display();
        }
        match event {
            WindowEvent::CloseRequested => self.close(event_loop),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                if focused {
                    self.tell(Said::Focused);
                }
                // Losing focus loses the pointers' capture: what they held
                // is cancelled, in order with the input before it.
                if !focused {
                    self.forward(input_log::Event::Blur, Instant::now());
                }
                self.follow_chrome();
            }
            WindowEvent::ThemeChanged(_) => self.follow_chrome(),
            WindowEvent::Occluded(hidden) => self.tell(Said::Visible(!hidden)),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                // A configure: maximized or fullscreen may have changed with the size.
                self.follow_chrome();
                self.resized();
            }
            // On Wayland, the frame callback asked for after a present: the
            // render thread may draw again. Elsewhere the OS asks for what
            // the window shows (it was exposed): the next frame presents it.
            WindowEvent::RedrawRequested => {
                if let Some(s) = &self.state {
                    let m = if is_wayland(&s.window) {
                        ToRender::FrameCallback(Instant::now())
                    } else {
                        ToRender::Expose
                    };
                    s.threads.to_render.send(m).ok();
                }
            }
            // A scroll event winit made: where the window's scroll tap
            // takes them (macOS), winit makes none.
            event => {
                if let Some(input) = input_log::from_winit(event, scale) {
                    self.forward(input, Instant::now());
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.take_scrolls();
        let window = || {
            self.state
                .as_ref()
                .map(|s| (native_of(&s.window), interval_of(&s.window)))
        };
        let event = match on_display(event, self.displays.as_mut(), window, &mut self.told) {
            Ok(news) => {
                if let (Some(news), Some(s)) = (news, &self.state) {
                    s.threads.to_render.send(news).ok();
                }
                return;
            }
            Err(other) => other,
        };
        match event {
            UserEvent::Access(event) => {
                // An action is input: routed, logged and sent as any is.
                if let accesskit_winit::WindowEvent::ActionRequested(request) = event.window_event {
                    self.forward(input_log::Event::Act(request), Instant::now());
                    return;
                }
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        self.listening = true;
                        self.tell(Said::Access(true));
                    }
                    accesskit_winit::WindowEvent::ActionRequested(_) => {}
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                        self.listening = false;
                        self.tell(Said::Access(false));
                    }
                }
            }
            // Taken in above: the displays by `on_display`, the scrolls as
            // every event's first step.
            UserEvent::Display | UserEvent::Scrolled => {}
            UserEvent::Ui(ToPlatform::Cursor(c)) => {
                if let Some(s) = &self.state {
                    s.window.set_cursor(c);
                }
            }
            UserEvent::Ui(ToPlatform::Window(action)) => self.act(event_loop, action),
            UserEvent::Ui(ToPlatform::Ime(target)) => self.follow_ime(target),
            UserEvent::Ui(ToPlatform::RenderFailed(why)) => {
                eprintln!("the render thread stopped ({why}); the window shows its last frame and still takes input");
                self.failed = true;
            }
            UserEvent::Ui(ToPlatform::Access(update)) => {
                if let Some(s) = self.state.as_mut()
                    && self.listening
                {
                    s.access.update_if_active(|| *update);
                }
            }
        }
    }
}

/// An app on its UI and render threads with no window (ADR-0015): the
/// render thread draws into its canvas at a fixed rate, and whoever holds
/// this is the platform thread, stamping input. What the stress scenes and
/// the threading tests measure.
pub struct Offscreen {
    threads: Threads,
    log: Arc<Mutex<Vec<FrameStat>>>,
    cursor: (f32, f32),
    /// What [`Self::send`] routes the platform's events by, as the
    /// window's event loop does.
    router: Router,
    /// [`Self::send`] logs what it sends ([`Self::record_input`]).
    recording: bool,
}

impl Offscreen {
    /// Start `start`'s handler on a UI thread, drawn `size` physical pixels
    /// at `scale` by a render thread ticking every `interval`.
    ///
    /// # Errors
    ///
    /// When there is no GPU or a thread cannot start.
    pub fn start(
        start: impl FnOnce() -> Box<dyn WindowHandler> + Send + 'static,
        size: (u32, u32),
        scale: f32,
        interval: Duration,
    ) -> Result<Self, String> {
        let gpu = GpuRenderer::headless().map_err(|e| e.to_string())?;
        let log = Arc::new(Mutex::new(Vec::new()));
        let threads = spawn(
            gpu,
            Pace::Ticker(interval),
            interval,
            "offscreen".to_owned(),
            Box::new(start),
            Box::new(|_| {}),
            Some(Arc::clone(&log)),
        )
        .map_err(|e| e.to_string())?;
        threads.told(Said::Resized { size, scale }, Instant::now(), Seen::BeforeFirst, false);
        Ok(Self {
            threads,
            log,
            cursor: (0.0, 0.0),
            router: Router::default(),
            recording: false,
        })
    }

    /// Log what [`Self::send`] sends, and what the render thread observes,
    /// to `path` from now on, as `GUI_RECORD_INPUT` logs a window's
    /// (ADR-0033): headed `size` logical pixels at `scale`.
    ///
    /// # Errors
    ///
    /// When `path` cannot be written.
    pub fn record_input(&mut self, path: &std::path::Path, size: (u32, u32), scale: f32) -> std::io::Result<()> {
        let header = input_log::Header {
            platform: input_log::Platform::Other,
            size,
            scale,
            source: "recorded offscreen".to_owned(),
        };
        let r = Recorder::start(path, &header)?;
        // Its compositor flings as the platform the log names does, so a
        // replay of it does the same (Codex r13: the matrix flings).
        self.threads
            .to_render
            .send(ToRender::Flings(!header.platform.flings_itself()))
            .ok();
        self.threads.forwarding.send(|route| route.record_with(r));
        self.recording = true;
        Ok(())
    }

    /// Make the render thread panic when it next reaches `stage` (fault
    /// injection, [`crate::compositor::thread::PanicStage`]).
    #[cfg(test)]
    pub(crate) fn panic_render_at(&self, stage: crate::compositor::thread::PanicStage) {
        self.threads.to_render.send(ToRender::PanicAt(stage)).ok();
    }

    /// Send `event` as the platform thread sends one it took from the OS,
    /// stamped now: routed as the window's event loop routes it, and logged
    /// once recording ([`Self::record_input`]).
    pub fn send(&mut self, event: input_log::Event) {
        self.send_at(event, Instant::now());
    }

    /// [`Self::send`] stamped `at` (not after now), as a platform stamps
    /// an event with the time the OS took it: a script's events spaced so
    /// whatever their sending's delays (a trackpad's speed, its fling).
    pub(crate) fn send_at(&mut self, event: input_log::Event, at: Instant) {
        let logged = self.recording.then(|| event.clone());
        let routed = self.router.route(event);
        self.cursor = self.router.cursor();
        let shown = self.threads.presented.seen_at(clock::nanos(at));
        match routed {
            Routed::Ui(input) => {
                track_pointer(&self.threads.presented, &input);
                self.threads.input(input, at, shown, logged);
            }
            Routed::Wheel {
                point,
                delta,
                precise,
                phase,
            } => self.threads.wheel(
                Wheel {
                    point,
                    delta,
                    at,
                    precise,
                    phase,
                    shown,
                },
                logged,
            ),
            Routed::Told(told) => self.threads.told(told, at, shown, logged.is_some()),
        }
    }

    /// Send `input` as the platform thread does, stamped now.
    pub fn input(&mut self, input: Input) {
        if let Input::PointerMoved { x, y } | Input::MouseDown { x, y } | Input::MouseUp { x, y } = input {
            self.cursor = (x, y);
        }
        let now = Instant::now();
        let shown = self.threads.presented.seen_at(clock::nanos(now));
        track_pointer(&self.threads.presented, &input);
        self.threads.input(input, now, shown, None);
    }

    /// Turn the wheel at the pointer by `delta` (a trackpad's when `precise`).
    pub fn wheel(&mut self, delta: (f32, f32), precise: bool) {
        let (point, shown) = (self.cursor, self.shown().seen());
        let wheel = Wheel {
            point,
            delta,
            at: Instant::now(),
            precise,
            phase: crate::compositor::WheelPhase::None,
            shown,
        };
        self.threads.wheel(wheel, None);
    }

    /// What is on screen now.
    #[must_use]
    pub fn shown(&self) -> Shown {
        self.threads.presented.now()
    }

    /// The frame on screen now; before any, an empty one.
    #[must_use]
    pub fn presented(&self) -> Arc<crate::compositor::Presented> {
        match self.shown() {
            Shown::Frame(p) => p,
            Shown::BeforeFirst | Shown::Gone(_) => Arc::default(),
        }
    }

    /// The frames the render thread drew since the last call.
    #[must_use]
    pub fn take_frames(&self) -> Vec<FrameStat> {
        std::mem::take(&mut *self.log.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The input the UI took to handle since the last call, in the order
    /// it took them, each with how long after it was sent ([`Self::input`]).
    #[must_use]
    pub fn take_input_latencies(&self) -> Vec<(Input, Duration)> {
        self.threads.latency.as_ref().map_or_else(Vec::new, |l| {
            std::mem::take(&mut *l.lock().unwrap_or_else(PoisonError::into_inner))
        })
    }

    /// The input the render thread handed on since the last call, each
    /// with how long after it was sent: its hop alone (ADR-0033).
    #[must_use]
    pub fn take_forward_latencies(&self) -> Vec<(Input, Duration)> {
        self.threads.forwarded.as_ref().map_or_else(Vec::new, |l| {
            std::mem::take(&mut *l.lock().unwrap_or_else(PoisonError::into_inner))
        })
    }

    /// How the OS schedules the UI and render threads (ADR-0015): each
    /// thread's name and what it was given.
    #[must_use]
    pub fn priorities(&self) -> Vec<(&'static str, Applied)> {
        self.threads
            .priorities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Lose the render thread's GPU device, as a driver reset would
    /// (ADR-0015's invariant 7: it is made again and the window drawn whole).
    pub fn lose_device(&self) {
        self.threads.to_render.send(ToRender::LoseDevice).ok();
    }

    /// Lose the GPU device with no other answering for `long`, as a GPU
    /// still resetting (fault injection).
    pub fn lose_device_for(&self, long: Duration) {
        self.threads.to_render.send(ToRender::HoldLost(long)).ok();
    }

    /// Make the render thread panic, as a bug in it would (invariant 7: it
    /// starts afresh from a full commit of the UI's).
    pub fn panic_render(&self) {
        self.threads.to_render.send(ToRender::Panic).ok();
    }

    /// Whether input goes through the render thread now (ADR-0033), not
    /// straight to the UI.
    #[must_use]
    pub fn through(&self) -> bool {
        self.threads.forwarding.send(|route| route.through)
    }

    /// The last frame, read back.
    #[must_use]
    pub fn capture(&self) -> Option<Pixels> {
        self.capture_within(Duration::from_secs(5))
    }

    /// The last frame, read back if the render thread answers within
    /// `timeout`.
    #[must_use]
    pub fn capture_within(&self, timeout: Duration) -> Option<Pixels> {
        let (tx, rx) = mpsc::channel();
        self.threads.to_render.send(ToRender::Capture(None, tx)).ok()?;
        rx.recv_timeout(timeout).ok().flatten()
    }
}

impl Drop for Offscreen {
    fn drop(&mut self) {
        self.threads.to_ui.send(ToUi::Quit).ok();
        self.threads.to_render.send(ToRender::Quit).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::PointerResampler;
    use super::input_log::Router;
    use crate::events::Modifiers;

    /// What the compositor scrolled `take` numbers, one container's worth.
    fn pulled(take: u64) -> crate::compositor::Pulled {
        crate::compositor::Pulled {
            take,
            deltas: vec![crate::compositor::ScrollDelta::default()],
        }
    }

    /// Opus r10 P1, sent as the platform sends (r11 nit: through
    /// [`super::Threads::input`], not a stand-in): a log being recorded as
    /// the render thread panics. The press it had not taken is handed on
    /// as it diverts, and the release the platform sends straight after:
    /// both are in the log, in that order, each after the frame it was
    /// routed in, and the release reaches the UI straight.
    #[test]
    fn input_taken_on_the_fallback_path_is_in_the_log() {
        use std::sync::{Arc, Mutex, mpsc};
        use std::time::{Duration, Instant};

        use super::{Compositor, Forwarding, GpuRenderer, Input, Pace, RenderThread, Seen, ToUi, input_log};

        let Ok(gpu) = GpuRenderer::headless() else {
            eprintln!("no GPU: skipped");
            return;
        };
        let path = std::env::temp_dir().join(format!("gui-fallback-log-{}.txt", std::process::id()));
        let header = input_log::Header {
            platform: input_log::Platform::Other,
            size: (64, 64),
            scale: 1.0,
            source: "a test".to_owned(),
        };
        let Ok(recorder) = input_log::Recorder::start(&path, &header) else {
            eprintln!("no temp file: skipped");
            return;
        };
        let (to_render, rx) = mpsc::channel();
        let (to_ui, ui_rx) = mpsc::channel();
        let compositor = Compositor::new(super::Glyphs::for_os());
        let presented = compositor.timeline();
        let forwarding = Forwarding::new();
        forwarding.send(|route| route.record_with(recorder));
        let mut thread = RenderThread::new(
            compositor,
            gpu,
            rx,
            Box::new(|_| {}),
            Duration::from_millis(1),
            Pace::Ticker(Duration::from_millis(1)),
            None,
        )
        .forwarding(Arc::clone(&forwarding));
        let threads = super::Threads {
            to_ui,
            to_render: to_render.clone(),
            waker: Arc::new(to_render),
            presented,
            priorities: Arc::new(Mutex::new(Vec::new())),
            forwarding,
            latency: None,
            forwarded: None,
        };
        let at = Instant::now();
        let (down, up) = (Input::MouseDown { x: 1.0, y: 2.0 }, Input::MouseUp { x: 1.0, y: 2.0 });
        threads.input(down, at, Seen::Frame(4), Some(input_log::Event::Primary(true)));
        thread.divert();
        threads.input(up.clone(), at, Seen::Frame(5), Some(input_log::Event::Primary(false)));
        assert!(
            ui_rx.try_iter().any(|m| matches!(m, ToUi::Input(i, ..) if i == up)),
            "the release went straight to the UI"
        );
        let until = Instant::now() + Duration::from_secs(5);
        let log = loop {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(log) = input_log::InputLog::parse(&text)
                && log.events.len() == 2
            {
                break log;
            }
            assert!(Instant::now() < until, "the log has both: {text}");
            std::thread::sleep(Duration::from_millis(5));
        };
        std::fs::remove_file(&path).ok();
        let events: Vec<_> = log.events.iter().map(|e| e.event.clone()).collect();
        assert_eq!(
            events,
            [input_log::Event::Primary(true), input_log::Event::Primary(false)]
        );
        let seen: Vec<_> = log.observed.iter().map(|o| (o.after, o.what)).collect();
        assert_eq!(
            seen,
            [
                // Begun before any commit was applied.
                (0, input_log::Observation::Uncommitted),
                (0, input_log::Observation::Diverted),
                (0, input_log::Observation::Seen(Some(4))),
                (1, input_log::Observation::Seen(Some(5)))
            ]
        );
    }

    /// ADR-0033: what rides with each input the render thread hands on is
    /// taken in right before that input is handled, after the drag moves
    /// held before it, whichever way it came (through the render thread,
    /// or straight while it could not serve); a held move's is taken in as
    /// the move comes, so the move sent later is handled after it.
    #[test]
    fn what_rides_with_input_is_taken_in_right_before_it() {
        use std::sync::mpsc;

        use super::{FromRender, Input, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: kept(&[1]),
            spare: None,
            latency: None,
        };
        let now = std::time::Instant::now();
        let through = |input, take| {
            ToUi::Render(FromRender::Input {
                input,
                at: now,
                sent: now,
                shown: shown(1),
                pulled: pulled(take),
            })
        };
        for msg in [
            through(Input::MouseDown { x: 10.0, y: 10.0 }, 1),
            through(Input::PointerMoved { x: 20.0, y: 10.0 }, 2),
            through(Input::PointerMoved { x: 30.0, y: 10.0 }, 3),
            ToUi::Render(FromRender::Diverted {
                pulled: pulled(4),
                gesture: None,
                returning: false,
            }),
            ToUi::Input(Input::MouseUp { x: 30.0, y: 10.0 }, now, pulled(5), shown(1)),
            ToUi::Quit,
        ] {
            to_ui.send(msg).ok();
        }
        let mut seen = Seen(Vec::new());
        super::ui_loop(&mut seen, &rx, &mut link, &|_| {}, "test");
        assert_eq!(
            seen.0,
            [
                "take 1",
                "MouseDown { x: 10.0, y: 10.0 } @1",
                "take 2",
                "take 3",
                "PointerMoved { x: 30.0, y: 10.0 } @1",
                "take 4",
                "take 5",
                "MouseUp { x: 30.0, y: 10.0 } @1",
            ]
        );
    }

    /// One latch in flight at a time (the fault matrix's class): diverted
    /// while its own latch is on its way to the render thread, the UI
    /// holds the input that comes until the latch is back, then handles it
    /// in its order; what is not input (a frame's feedback) goes on.
    #[test]
    fn input_waits_for_the_uis_latch_on_its_way_back() {
        use std::sync::mpsc;

        use super::{FromRender, Input, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: kept(&[1]),
            spare: None,
            latency: None,
        };
        let now = std::time::Instant::now();
        for msg in [
            ToUi::Render(FromRender::Diverted {
                pulled: pulled(1),
                gesture: None,
                returning: true,
            }),
            ToUi::Input(Input::MouseDown { x: 1.0, y: 1.0 }, now, pulled(2), shown(1)),
            ToUi::Render(FromRender::Lost),
            ToUi::Input(Input::MouseUp { x: 1.0, y: 1.0 }, now, pulled(3), shown(1)),
            ToUi::Render(FromRender::Returned(None)),
            ToUi::Input(Input::PointerLeft, now, pulled(4), shown(1)),
            ToUi::Quit,
        ] {
            to_ui.send(msg).ok();
        }
        let mut seen = Seen(Vec::new());
        super::ui_loop(&mut seen, &rx, &mut link, &|_| {}, "test");
        assert_eq!(
            seen.0,
            [
                "take 1",
                "lost",
                "take 2",
                "MouseDown { x: 1.0, y: 1.0 } @1",
                "take 3",
                "MouseUp { x: 1.0, y: 1.0 } @1",
                "take 4",
                "PointerLeft @1",
            ]
        );
    }

    /// A zero-delta end of one gesture, then a zero-delta begin of the
    /// next, as the window's tap took them: each one wheel event, routed
    /// with its own phase (read off its own event, never matched to
    /// another), the pointer moved to where the second is first. Routed
    /// in order, the end leaves the gesture closed and the begin opens
    /// one: neither is the other's.
    #[test]
    fn a_tapped_zero_delta_end_then_begin_routes_each_with_its_own_phase() {
        use super::input_log::{Event, Routed};
        use crate::compositor::WheelPhase;
        use gui_display::ScrollPhase as P;
        let now = std::time::Instant::now();
        let tapped = |phase, point| tapped(phase, point, now);
        let mut router = Router::default();
        let mut route = |s: &gui_display::Scroll| {
            let t = super::tapped_events(s, router.cursor(), router.modifiers());
            assert!(t.modifiers.is_none(), "none held, none told");
            let moved = t.moved.map(|m| router.route(m));
            (moved, router.route(t.wheel))
        };
        let (moved, end) = route(&tapped(P::Ended, (0.0, 0.0)));
        assert!(moved.is_none(), "where the pointer is already");
        assert!(
            matches!(
                end,
                Routed::Wheel {
                    phase: WheelPhase::Ended,
                    ..
                }
            ),
            "{end:?}"
        );
        let (moved, begin) = route(&tapped(P::Began, (40.0, 20.0)));
        assert_eq!(
            moved,
            Some(Routed::Ui(super::Input::PointerMoved { x: 40.0, y: 20.0 })),
            "the pointer moved to it first"
        );
        assert!(
            matches!(
                begin,
                Routed::Wheel {
                    phase: WheelPhase::Began,
                    point: (40.0, 20.0),
                    ..
                }
            ),
            "{begin:?}"
        );
        let t = super::tapped_events(&tapped(P::MayBegin, (40.0, 20.0)), (40.0, 20.0), Modifiers::default());
        assert!(t.moved.is_none());
        assert!(matches!(
            t.wheel,
            Event::Wheel {
                phase: WheelPhase::Touched,
                ..
            }
        ));
    }

    /// A tapped scroll event at `point`, inside the view, no button down,
    /// no modifier held.
    fn tapped(phase: gui_display::ScrollPhase, point: (f64, f64), at: std::time::Instant) -> gui_display::Scroll {
        gui_display::Scroll {
            phase,
            momentum: gui_display::ScrollPhase::None,
            at,
            delta: (0.0, 0.0),
            precise: true,
            point,
            inside: true,
            buttons: false,
            modifiers: gui_display::ScrollModifiers::default(),
        }
    }

    /// As winit's `scrollWheel:` makes of it (Opus r7 N3): a tapped event
    /// outside the view with no button down moves no pointer (its
    /// `mouse_motion` skips it), one with a button down does; and the
    /// modifiers the event holds are told first when they differ from
    /// those last told (`update_modifiers`), a ⌘ held over the wheel
    /// reaching the UI before the wheel does.
    #[test]
    fn a_tapped_scroll_moves_and_modifies_as_winits_view_would() {
        use super::input_log::Event;
        use gui_display::ScrollPhase as P;
        let now = std::time::Instant::now();
        let outside = gui_display::Scroll {
            inside: false,
            ..tapped(P::Changed, (-5.0, 20.0), now)
        };
        let t = super::tapped_events(&outside, (10.0, 10.0), Modifiers::default());
        assert!(t.moved.is_none(), "outside, no button: no move");
        let dragging = gui_display::Scroll {
            buttons: true,
            ..outside
        };
        let t = super::tapped_events(&dragging, (10.0, 10.0), Modifiers::default());
        assert!(
            matches!(t.moved, Some(Event::Moved(-5.0, 20.0))),
            "a button down: {t:?}"
        );
        let command = gui_display::Scroll {
            modifiers: gui_display::ScrollModifiers {
                command: true,
                ..Default::default()
            },
            ..tapped(P::Changed, (10.0, 10.0), now)
        };
        let t = super::tapped_events(&command, (10.0, 10.0), Modifiers::default());
        assert!(
            matches!(t.modifiers, Some(Event::Modifiers(m)) if m.command && !m.shift),
            "⌘ told: {t:?}"
        );
        let held = Modifiers {
            command: true,
            ..Modifiers::default()
        };
        assert!(
            super::tapped_events(&command, (10.0, 10.0), held).modifiers.is_none(),
            "told once"
        );
        // Routed in order, the wheel sees ⌘ held.
        let mut router = Router::default();
        let t = super::tapped_events(&command, router.cursor(), router.modifiers());
        for e in [t.modifiers, t.moved].into_iter().flatten() {
            router.route(e);
        }
        assert!(router.modifiers().command, "held as the wheel comes");
    }

    #[test]
    fn a_moved_window_reads_its_display_again() {
        // Onto a screen of the same scale and another refresh rate: no
        // resize, no rescale, and the render thread's interval must follow.
        let moved = winit::event::WindowEvent::Moved(winit::dpi::PhysicalPosition::new(1920, 0));
        assert!(super::display_may_change(&moved));
        assert!(super::display_may_change(&winit::event::WindowEvent::Occluded(false)));
        assert!(!super::display_may_change(&winit::event::WindowEvent::Occluded(true)));
        assert!(!super::display_may_change(&winit::event::WindowEvent::Focused(true)));
    }

    #[test]
    fn the_render_thread_is_told_the_display_interval_only_when_it_changed() {
        // Each `Moved` of a drag reads the display again; telling the
        // render thread the same interval would reset its fitted vsync.
        // No displays read (Wayland): the toolkit's rate is all there is.
        use super::{ToRender, Told, follow};
        let hz60 = std::time::Duration::from_micros(16_667);
        let hz120 = std::time::Duration::from_micros(8_333);
        let mut told = Told::default();
        assert!(matches!(follow(None, None, Some(hz60), &mut told), Some(ToRender::Interval(i)) if i == hz60));
        assert!(follow(None, None, Some(hz60), &mut told).is_none());
        assert!(matches!(follow(None, None, Some(hz120), &mut told), Some(ToRender::Interval(i)) if i == hz120));
        assert!(follow(None, None, Some(hz120), &mut told).is_none());
        assert!(
            follow(None, None, None, &mut told).is_none(),
            "a rate not reported is not a change"
        );
        assert_eq!(told.interval, Some(hz120));
    }

    /// From what the OS says to what the render thread times its frames by
    /// (ADR-0029): scripted displays play the OS; `follow` is what the
    /// platform thread runs on each of its events; the render thread takes
    /// the message as it does a window's.
    #[test]
    fn an_os_display_event_reaches_the_render_threads_vsync() {
        use super::{ToRender, Told, follow};
        use std::time::Duration;
        let hz60 = Duration::from_nanos(16_666_667);
        let hz120 = Duration::from_nanos(8_333_333);
        let woken = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let theirs = std::sync::Arc::clone(&woken);
        let (mut displays, os) = gui_display::Displays::scripted(move || {
            theirs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        let fast = {
            let mut d = gui_display::Script::output(2, "external");
            d.refresh.interval = hz120;
            d
        };
        os.plug(gui_display::Script::output(1, "built-in"));
        os.plug(fast);
        displays.poll();
        let mut told = Told::default();
        let mut vsync = crate::clock::Vsync::new(Duration::from_millis(10));
        let mut ticks = None;
        let mut take = |news: Option<ToRender>, vsync: &mut crate::clock::Vsync| match news {
            Some(ToRender::Output { interval, ticks: t }) => {
                vsync.moved(interval);
                ticks = t;
                "output"
            }
            Some(ToRender::Interval(i)) => {
                vsync.set_interval(i);
                "interval"
            }
            Some(_) => "other",
            None => "nothing",
        };
        // The window opens on the built-in display: winit says 59.9 Hz, the
        // OS's own mode is what is taken.
        let winit = Some(Duration::from_micros(16_694));
        assert_eq!(
            take(follow(Some(&mut displays), Some(1), winit, &mut told), &mut vsync),
            "output"
        );
        assert_eq!(vsync.interval(), hz60);
        assert_eq!(os.running(1), 1, "its vsync source is started");
        assert_eq!(
            take(follow(Some(&mut displays), Some(1), winit, &mut told), &mut vsync),
            "nothing"
        );

        // Dragged onto the external one (a `Moved`): its interval, its ticks.
        assert_eq!(
            take(follow(Some(&mut displays), Some(2), winit, &mut told), &mut vsync),
            "output"
        );
        assert_eq!(vsync.interval(), hz120);
        displays.poll();
        assert_eq!(
            (os.running(1), os.running(2)),
            (0, 1),
            "the old output's source is closed"
        );

        // The OS changes that display's mode with no window event at all:
        // the wake has the platform thread poll, and the interval follows.
        let before = woken.load(std::sync::atomic::Ordering::SeqCst);
        os.change(2, |d| d.refresh.interval = hz60);
        assert!(woken.load(std::sync::atomic::Ordering::SeqCst) > before);
        assert!(!displays.poll().is_empty());
        assert_eq!(
            take(follow(Some(&mut displays), Some(2), winit, &mut told), &mut vsync),
            "interval"
        );
        assert_eq!(vsync.interval(), hz60);

        // The same panel under another OS handle (a GPU switch): its old
        // source is dead, so the render thread gets the new one.
        os.change(2, |d| d.native = 7);
        displays.poll();
        assert_eq!(
            take(follow(Some(&mut displays), Some(7), winit, &mut told), &mut vsync),
            "output"
        );
        displays.poll();
        assert_eq!((os.running(2), os.running(7)), (0, 1));

        // Pulled out: the OS moves the window to the display that stays.
        os.unplug(7);
        displays.poll();
        assert_eq!(
            take(follow(Some(&mut displays), Some(1), winit, &mut told), &mut vsync),
            "output"
        );
        assert_eq!(told.output.map(|(_, native)| native), Some(1));
    }

    /// The displays' wake is a `UserEvent::Display` for the event loop,
    /// which `user_event` hands to `on_display`: a change the OS told of,
    /// with no window event and no poll yet, reaches the render thread
    /// through that event alone.
    #[test]
    fn the_displays_wake_alone_tells_the_render_thread_of_a_change() {
        use super::{ToRender, Told, UserEvent, on_display};
        use std::time::Duration;
        let hz120 = Duration::from_nanos(8_333_333);
        let winit = Some(Duration::from_micros(16_694));
        // The event loop's proxy, as `connect`'s wake sends to it.
        let (proxy, events) = std::sync::mpsc::channel();
        let (mut displays, os) = gui_display::Displays::scripted(move || {
            proxy.send(UserEvent::Display).ok();
        });
        let mut told = Told::default();
        let dispatch = |displays: &mut gui_display::Displays, told: &mut Told| {
            let mut news = Vec::new();
            while let Ok(event) = events.try_recv() {
                news.push(
                    on_display(event, Some(&mut *displays), || Some((Some(1), winit)), told)
                        .ok()
                        .flatten(),
                );
            }
            news
        };
        os.plug(gui_display::Script::output(1, "built-in"));
        // Kept, as the render thread keeps the output's ticks it was sent.
        let first = dispatch(&mut displays, &mut told);
        assert!(matches!(first.as_slice(), [Some(ToRender::Output { .. })]));
        // The OS changes the mode; its wake is all the platform thread gets.
        os.change(1, |d| d.refresh.interval = hz120);
        let news = dispatch(&mut displays, &mut told);
        assert!(
            matches!(news.as_slice(), [Some(ToRender::Interval(i))] if *i == hz120),
            "the wake's event reads the change and tells it: {} news",
            news.len()
        );
        assert!(dispatch(&mut displays, &mut told).is_empty(), "nothing more woke it");
        // Any other event is not the displays'.
        assert!(
            on_display(
                UserEvent::Ui(super::ToPlatform::RenderFailed(String::new())),
                Some(&mut displays),
                || Some((Some(1), winit)),
                &mut told
            )
            .is_err()
        );
    }

    /// winit may report a monitor the displays have not polled yet (their
    /// wake not handled): the render thread is told of that output once,
    /// not of winit's rate first and of the output after, each a reset.
    #[test]
    fn a_monitor_winit_reports_before_the_displays_poll_resets_the_vsync_once() {
        use super::{ToRender, Told, follow};
        use std::time::Duration;
        let (mut displays, os) = gui_display::Displays::scripted(|| {});
        os.plug(gui_display::Script::output(1, "built-in"));
        let mut told = Told::default();
        let winit = Some(Duration::from_micros(16_694));
        assert!(follow(Some(&mut displays), Some(1), winit, &mut told).is_some());
        // Plugged and the window moved onto it; the displays not polled since.
        let mut fast = gui_display::Script::output(3, "external");
        fast.refresh.interval = Duration::from_nanos(8_333_333);
        os.plug(fast);
        let mut resets = 0;
        for _ in 0..3 {
            match follow(Some(&mut displays), Some(3), winit, &mut told) {
                Some(ToRender::Output { interval, ticks }) => {
                    assert_eq!(interval, Duration::from_nanos(8_333_333));
                    assert!(ticks.is_some());
                    resets += 1;
                }
                Some(_) => resets += 1,
                None => {}
            }
        }
        assert_eq!(resets, 1, "told once, of the output");
    }

    /// A handler that writes down each input and the frame shown as it came.
    struct Seen(Vec<String>);

    /// The frame `shown` stamps, `-` before the first.
    fn frame_of(shown: &super::Shown) -> String {
        match shown {
            super::Shown::Frame(p) => p.frame.to_string(),
            super::Shown::BeforeFirst => "-".to_owned(),
            super::Shown::Gone(n) => format!("gone {n}"),
        }
    }

    /// The stamp of the frame numbered `frame`.
    fn shown(frame: u64) -> crate::compositor::Seen {
        crate::compositor::Seen::Frame(frame)
    }

    /// A timeline that presented `frames`, each at its number's
    /// nanosecond (long past).
    fn kept(frames: &[u64]) -> std::sync::Arc<crate::compositor::OnScreen> {
        let on = crate::compositor::OnScreen::new();
        for &frame in frames {
            let p = crate::compositor::Presented {
                frame,
                present_at: frame,
                ..crate::compositor::Presented::default()
            };
            on.presented(std::sync::Arc::new(p), &mut std::collections::VecDeque::new());
        }
        on
    }

    impl super::WindowHandler for Seen {
        fn frame(&mut self, _: &mut dyn super::Compose, _: super::MainFrame, _: (u32, u32), _: f32) {}
        fn take_scroll(&mut self, pulled: crate::compositor::Pulled) -> bool {
            if !pulled.is_empty() {
                self.0.push(format!("take {}", pulled.take));
            }
            false
        }
        fn take_gesture(&mut self, _: crate::compositor::HandedGesture) {
            self.0.push("gesture".to_owned());
        }
        fn input(&mut self, input: super::Input, presented: &super::Shown) -> bool {
            self.0.push(format!("{input:?} @{}", frame_of(presented)));
            false
        }
        fn cursor(&self) -> super::Cursor {
            super::Cursor::Default
        }
        fn ime_target(&mut self) -> Option<super::ImeTarget> {
            None
        }
        #[expect(clippy::unreachable, reason = "no screen reader connects in the test")]
        fn access_update(&mut self, _: f32, _: &str) -> accesskit::TreeUpdate {
            unreachable!("no screen reader connects")
        }
        fn access_reset(&mut self) {}
        fn idle(&mut self, _: &mut dyn super::Compose) -> super::Wake {
            super::Wake {
                redraw: false,
                animating: false,
                next_timer: None,
            }
        }
        fn focused(&mut self) -> bool {
            false
        }
        fn renderer_lost(&mut self) {
            self.0.push("lost".to_owned());
        }
        fn chrome(&mut self, _: super::Chrome) -> bool {
            false
        }
        fn window_actions(&mut self) -> Vec<super::WindowAction> {
            Vec::new()
        }
    }

    #[test]
    fn held_moves_keep_the_frame_they_came_under_and_go_before_what_follows() {
        use std::sync::mpsc;

        use super::{FromRender, Input, MainFrame, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: kept(&[1, 2]),
            spare: None,
            latency: None,
        };
        let (a, b) = (shown(1), shown(2));
        let now = std::time::Instant::now();
        let input = |i, p: &crate::compositor::Seen| ToUi::Input(i, now, crate::compositor::Pulled::default(), *p);
        // A drag's moves under two frames shown, then the pointer leaves
        // before the next frame: each move goes under its own frame, both
        // before the leave, and none after it.
        for msg in [
            input(Input::MouseDown { x: 10.0, y: 10.0 }, &a),
            input(Input::PointerMoved { x: 20.0, y: 10.0 }, &a),
            input(Input::PointerMoved { x: 30.0, y: 10.0 }, &b),
            input(Input::PointerLeft, &b),
            ToUi::Render(FromRender::Begin(MainFrame::at(1.0))),
            ToUi::Quit,
        ] {
            to_ui.send(msg).ok();
        }
        let mut seen = Seen(Vec::new());
        super::ui_loop(&mut seen, &rx, &mut link, &|_| {}, "test");
        assert_eq!(
            seen.0,
            [
                "MouseDown { x: 10.0, y: 10.0 } @1",
                "PointerMoved { x: 20.0, y: 10.0 } @1",
                "PointerMoved { x: 30.0, y: 10.0 } @2",
                "PointerLeft @2"
            ]
        );
    }

    #[test]
    fn a_wheel_the_compositor_forwards_keeps_its_own_point_and_frame() {
        use std::sync::mpsc;

        use super::{FromRender, Input, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: kept(&[1, 2]),
            spare: None,
            latency: None,
        };
        let now = std::time::Instant::now();
        let none = crate::compositor::Pulled::default;
        // The pointer last moved under frame 1 at (10, 10); the compositor
        // then forwards a wheel it found at (50, 60) under frame 2: the
        // wheel lands there, under that frame, after the held move.
        for msg in [
            ToUi::Input(Input::MouseDown { x: 10.0, y: 10.0 }, now, none(), shown(1)),
            ToUi::Input(Input::PointerMoved { x: 10.0, y: 10.0 }, now, none(), shown(1)),
            ToUi::Render(FromRender::Wheel {
                input: Input::WheelAt {
                    step: crate::compositor::WheelStep {
                        point: (50.0, 60.0),
                        ..crate::compositor::WheelStep::default()
                    },
                    dx: 0.0,
                    dy: 40.0,
                    target: None,
                },
                shown: shown(2),
                pulled: none(),
            }),
            ToUi::Quit,
        ] {
            to_ui.send(msg).ok();
        }
        let mut seen = Seen(Vec::new());
        super::ui_loop(&mut seen, &rx, &mut link, &|_| {}, "test");
        assert_eq!(
            seen.0,
            [
                "MouseDown { x: 10.0, y: 10.0 } @1",
                "PointerMoved { x: 10.0, y: 10.0 } @1",
                "WheelAt { step: WheelStep { point: (50.0, 60.0), time: 0, precise: false, phase: None, gesture: None }, dx: 0.0, dy: 40.0, target: None } @2"
            ]
        );
    }

    /// A handler that writes down the frame on screen each time it is idle
    /// and it changed, waits for the next vblank a frame entered shows
    /// from, and quits once `last` shows. Its cursor is a grab while `last`
    /// shows (hover worked out at its vblank, no input).
    struct Idles {
        log: Vec<String>,
        last: u64,
        shows_last: bool,
        quit: std::sync::mpsc::Sender<super::ToUi>,
        /// The field its input method types into.
        ime: crate::focus::FocusId,
    }

    impl super::WindowHandler for Idles {
        fn frame(&mut self, _: &mut dyn super::Compose, _: super::MainFrame, _: (u32, u32), _: f32) {}
        fn take_scroll(&mut self, _: crate::compositor::Pulled) -> bool {
            false
        }
        fn take_gesture(&mut self, _: crate::compositor::HandedGesture) {}
        fn input(&mut self, _: super::Input, _: &super::Shown) -> bool {
            false
        }
        fn cursor(&self) -> super::Cursor {
            if self.shows_last {
                super::Cursor::Grab
            } else {
                super::Cursor::Default
            }
        }
        /// A field the input method types into, its area where the frame
        /// on screen has its caret: one of its own once `last` shows.
        fn ime_target(&mut self) -> Option<super::ImeTarget> {
            Some(super::ImeTarget {
                field: self.ime,
                area: crate::geometry::Bounds {
                    x: if self.shows_last { 20.0 } else { 0.0 },
                    y: 0.0,
                    width: 2.0,
                    height: 16.0,
                },
            })
        }
        #[expect(clippy::unreachable, reason = "no screen reader connects in the test")]
        fn access_update(&mut self, _: f32, _: &str) -> accesskit::TreeUpdate {
            unreachable!("no screen reader connects")
        }
        fn access_reset(&mut self) {}
        fn idle(&mut self, target: &mut dyn super::Compose) -> super::Wake {
            let (now, next) = target.shown_and_next();
            let at = format!("@{}", frame_of(&now));
            if self.log.last() != Some(&at) {
                self.log.push(at);
            }
            self.shows_last = now.seen() == crate::compositor::Seen::Frame(self.last);
            if self.shows_last {
                self.quit.send(super::ToUi::Quit).ok();
            }
            super::Wake {
                redraw: false,
                animating: false,
                next_timer: next.map(crate::clock::instant_of),
            }
        }
        fn focused(&mut self) -> bool {
            false
        }
        fn renderer_lost(&mut self) {}
        fn chrome(&mut self, _: super::Chrome) -> bool {
            false
        }
        fn window_actions(&mut self) -> Vec<super::WindowAction> {
            Vec::new()
        }
    }

    /// A frame entered for a vblank still to come, which puts a box under
    /// the resting pointer, wakes the UI then, with no message: what it
    /// does at each frame shown (hover) is timed by the timeline, not by a
    /// message that may come late or not at all.
    #[test]
    fn the_ui_wakes_at_the_vblank_a_frame_entered_shows_from() {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let presented = kept(&[1]);
        presented.point(Some((10.0, 10.0)));
        let start = Instant::now();
        let vblank = start + Duration::from_millis(30);
        let rect = crate::geometry::Bounds {
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 32.0,
        };
        let b = crate::compositor::Presented::boxed(2, crate::clock::nanos(vblank), Some((7, rect)));
        let moves = presented.presented(std::sync::Arc::new(b), &mut std::collections::VecDeque::new());
        assert!(moves, "the box comes under the pointer");
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented,
            spare: None,
            latency: None,
        };
        // Should it never wake, the test ends all the same, and fails.
        let late = to_ui.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            late.send(super::ToUi::Quit).ok();
        });
        let mut idles = Idles {
            log: Vec::new(),
            last: 2,
            shows_last: false,
            quit: to_ui,
            ime: crate::focus::FocusHandle::new().id(),
        };
        let cursors = std::cell::RefCell::new(Vec::new());
        let imes = std::cell::RefCell::new(Vec::new());
        let platform = |m: super::ToPlatform| match m {
            super::ToPlatform::Cursor(c) => cursors.borrow_mut().push(c),
            super::ToPlatform::Ime(t) => imes.borrow_mut().push(t.map(|t| t.area.x)),
            _ => {}
        };
        super::ui_loop(&mut idles, &rx, &mut link, &platform, "test");
        assert_eq!(idles.log, ["@1", "@2"]);
        // Opus r9 P1: the cursor follows hover worked out at the vblank,
        // with no input and no frame drawn.
        assert_eq!(
            cursors.borrow().last(),
            Some(&super::cursor_icon(super::Cursor::Grab)),
            "{:?}",
            cursors.borrow()
        );
        // Review r12: so does the input method's area, a value of the frame
        // on screen as the cursor is, told once each time it changes.
        assert_eq!(*imes.borrow(), [Some(0.0), Some(20.0)]);
        let woke = start.elapsed();
        assert!(
            woke >= Duration::from_millis(30) && woke < Duration::from_secs(1),
            "woke at the vblank: {woke:?}"
        );
    }

    fn close(a: Option<(f32, f32)>, b: (f32, f32)) -> bool {
        a.is_some_and(|a| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3)
    }

    #[test]
    fn a_drag_is_sampled_5_ms_before_present() {
        let mut r = PointerResampler::default();
        r.push(0.000, 0.0, 0.0);
        r.push(0.010, 10.0, 0.0);
        r.push(0.020, 20.0, 0.0);
        // Presented at 20 ms: where the pointer was at 15 ms.
        let at = r.sample(0.020);
        assert!(close(at, (15.0, 0.0)), "{at:?}");
        assert_eq!(r.sample(0.030), None, "no move since: nothing to send");
    }

    #[test]
    fn a_drag_is_predicted_at_most_8_ms_and_half_a_gap_ahead() {
        let mut r = PointerResampler::default();
        r.push(0.000, 0.0, 0.0);
        r.push(0.020, 20.0, 0.0);
        // Wanted at 35 ms: 15 ms past the last sample, capped at 8 ms.
        assert!(close(r.sample(0.040), (28.0, 0.0)));
        let mut r = PointerResampler::default();
        r.push(0.000, 0.0, 0.0);
        r.push(0.004, 4.0, 0.0);
        // Half the 4 ms gap.
        assert!(close(r.sample(0.020), (6.0, 0.0)));
        let mut r = PointerResampler::default();
        r.push(0.000, 0.0, 0.0);
        r.push(0.001, 5.0, 0.0);
        // Samples 1 ms apart are not predicted from.
        assert!(close(r.sample(0.020), (5.0, 0.0)));
    }

    #[test]
    fn a_release_flushes_the_last_raw_position() {
        let mut r = PointerResampler::default();
        r.push(0.000, 0.0, 0.0);
        r.push(0.010, 10.0, 3.0);
        assert!(r.holds());
        assert_eq!(r.flush(), Some((10.0, 3.0)));
        assert!(!r.holds());
        assert_eq!(r.sample(0.1), None);
    }
}
