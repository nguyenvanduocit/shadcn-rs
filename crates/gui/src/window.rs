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

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{CursorIcon, Theme, Window, WindowAttributes, WindowId};

use crate::clock;
use crate::compositor::input_log::{self, Recorder, Routed, Router};
use crate::compositor::thread::{Feedback, FrameCallbacks, FrameStat, FromRender, Pace, RenderThread, ToRender};
use crate::compositor::{Commit, Compose, Compositor, MainFrame, Presented};
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
    /// React to `input`, which arrived while `presented` was on screen;
    /// return `true` when a frame must follow.
    fn input(&mut self, input: Input, presented: &Arc<Presented>) -> bool;
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
    /// An event, when it arrived and what was on screen then.
    Input(Input, Instant, Arc<Presented>),
    Render(FromRender),
    Access(AccessEvent),
    Focused,
    /// The window can (or cannot) be seen.
    Visible(bool),
    /// The window is this many physical pixels at this scale.
    Resized((u32, u32), f32),
    /// The window's decorations changed.
    Chrome(Chrome),
    /// A mailbox has mail.
    Mail,
    Quit,
}

/// What the accessibility adapter asked.
pub(crate) enum AccessEvent {
    Initial,
    Action(accesskit::ActionRequest),
    Deactivated,
}

/// What the UI thread tells the platform thread.
#[derive(Debug)]
pub(crate) enum ToPlatform {
    Cursor(CursorIcon),
    /// The window's own decorations ask the platform to move, resize,
    /// maximize or close the window.
    Window(WindowAction),
    /// The render thread could not go on: close, saying why.
    Quit(String),
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
    presented: Arc<Mutex<Arc<Presented>>>,
    /// The last commit the render thread applied and sent back.
    spare: Option<Box<Commit>>,
}

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

    fn presented(&self) -> Arc<Presented> {
        Arc::clone(&self.presented.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn begin(&mut self, at: f32) -> MainFrame {
        MainFrame::at(at)
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
    /// (seconds on the frame clock, x, y), oldest first.
    samples: Vec<(f32, f32, f32)>,
    /// A sample came since the last frame took one.
    fresh: bool,
}

/// How far before the present time a drag is sampled, seconds.
const RESAMPLE_LATENCY: f32 = 0.005;
const RESAMPLE_MAX_PREDICTION: f32 = 0.008;
const RESAMPLE_MIN_DELTA: f32 = 0.002;

impl PointerResampler {
    /// The pointer was at `(x, y)` at `at` seconds.
    pub fn push(&mut self, at: f32, x: f32, y: f32) {
        self.samples.push((at, x, y));
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
    pub fn sample(&mut self, present_at: f32) -> Option<(f32, f32)> {
        if !std::mem::take(&mut self.fresh) {
            return None;
        }
        let t = present_at - RESAMPLE_LATENCY;
        let &(t1, x1, y1) = self.samples.last()?;
        let at = if t <= t1 {
            match self
                .samples
                .windows(2)
                .find(|w| matches!(w, [a, b] if a.0 <= t && t <= b.0))
            {
                Some([(ta, xa, ya), (tb, xb, yb)]) => {
                    let k = if tb > ta { (t - ta) / (tb - ta) } else { 1.0 };
                    (xa + (xb - xa) * k, ya + (yb - ya) * k)
                }
                // Earlier than every sample: the earliest.
                _ => self.samples.first().map_or((x1, y1), |&(_, x, y)| (x, y)),
            }
        } else {
            match self.samples.len().checked_sub(2).and_then(|i| self.samples.get(i)) {
                Some(&(t0, x0, y0)) if t1 - t0 >= RESAMPLE_MIN_DELTA => {
                    let gap = t1 - t0;
                    let ahead = (t - t1).min(RESAMPLE_MAX_PREDICTION).min(gap / 2.0);
                    (x1 + (x1 - x0) / gap * ahead, y1 + (y1 - y0) / gap * ahead)
                }
                _ => (x1, y1),
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
#[derive(Default)]
struct HeldMoves {
    resampler: PointerResampler,
    /// What was on screen as the moves waiting came.
    shown: Option<Arc<Presented>>,
}

/// A move to send, and what was on screen as it came.
type HeldMove = ((f32, f32), Arc<Presented>);

impl HeldMoves {
    /// The pointer was at `(x, y)` at `at` seconds, with `shown` on
    /// screen; the move waiting under another frame shown, to send first.
    fn push(&mut self, at: f32, x: f32, y: f32, shown: &Arc<Presented>) -> Option<HeldMove> {
        let other = self.shown.as_ref().is_some_and(|s| !Arc::ptr_eq(s, shown));
        let first = if other { self.flush() } else { None };
        self.resampler.push(at, x, y);
        self.shown = Some(Arc::clone(shown));
        first
    }

    /// The move for a frame presented at `present_at`, when one waits.
    fn sample(&mut self, present_at: f32) -> Option<HeldMove> {
        let at = self.resampler.sample(present_at)?;
        Some((at, Arc::clone(self.shown.as_ref()?)))
    }

    /// The last move waiting, now, and nothing waits after.
    fn flush(&mut self) -> Option<HeldMove> {
        let at = self.resampler.flush()?;
        Some((at, Arc::clone(self.shown.as_ref()?)))
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
    let mut shown: Arc<Presented> = link.presented();
    let mut cursor = None;
    let mut chrome = Chrome::default();
    // The resize edge under the pointer, which shows its own cursor.
    let mut edge = None;
    // Hand on what the decorations asked, and show the cursor: the edge's,
    // or what the app shows there.
    let follow = |handler: &mut dyn WindowHandler, edge: Option<Edge>, cursor: &mut Option<CursorIcon>| {
        for action in handler.window_actions() {
            platform(ToPlatform::Window(action));
        }
        let c = edge.map_or_else(|| cursor_icon(handler.cursor()), Edge::cursor);
        if *cursor != Some(c) {
            *cursor = Some(c);
            platform(ToPlatform::Cursor(c));
        }
    };
    loop {
        let wake = handler.idle(link);
        wants |= wake.redraw || wake.animating;
        if wants && !asked && size.0 > 0 && size.1 > 0 {
            link.tx.send(ToRender::NeedsFrame).ok();
            asked = true;
        }
        let msg = match wake.next_timer {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            },
            None => match rx.recv() {
                Ok(m) => Some(m),
                Err(_) => return,
            },
        };
        let Some(msg) = msg else { continue };
        match msg {
            ToUi::Input(input, at, presented) => {
                if let Input::PointerMoved { x, y } = input
                    && pressed
                {
                    // A drag: one move a frame, resampled, under the frame
                    // it came under.
                    if let Some(first) = held.push(clock::seconds(at), x, y, &presented) {
                        send_move(handler, first);
                    }
                    shown = presented;
                    wants = true;
                    continue;
                }
                // Anything else goes after the moves held before it.
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                shown = presented;
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
                wants |= handler.input(input, &shown);
                follow(handler, edge, &mut cursor);
            }
            ToUi::Render(FromRender::Begin(token)) => {
                asked = false;
                wants = false;
                if let Some(due) = held.sample(token.present_at) {
                    send_move(handler, due);
                }
                handler.frame(link, token, size, scale);
                platform(ToPlatform::Ime(handler.ime_target()));
                follow(handler, edge, &mut cursor);
                if access {
                    platform(ToPlatform::Access(Box::new(handler.access_update(scale, title))));
                }
            }
            ToUi::Render(FromRender::Wheel {
                step,
                delta,
                shown: under,
                target,
            }) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                let (dx, dy) = delta;
                wants |= handler.input(Input::WheelAt { step, dx, dy, target }, &under);
                follow(handler, edge, &mut cursor);
            }
            ToUi::Render(FromRender::Drawn(f)) => link.feedback = *f,
            ToUi::Render(FromRender::Spare(c)) => link.spare = Some(c),
            ToUi::Render(FromRender::Lost) => {
                handler.renderer_lost();
                wants = true;
            }
            ToUi::Render(FromRender::Failed(why)) => {
                platform(ToPlatform::Quit(why));
                return;
            }
            ToUi::Access(AccessEvent::Initial) => {
                access = true;
                handler.access_reset();
                platform(ToPlatform::Access(Box::new(handler.access_update(scale, title))));
            }
            ToUi::Access(AccessEvent::Action(request)) => {
                if let Some(last) = held.flush() {
                    wants |= send_move(handler, last);
                }
                wants |= handler.input(Input::Act(request), &shown);
                follow(handler, edge, &mut cursor);
            }
            ToUi::Access(AccessEvent::Deactivated) => {
                access = false;
                handler.access_reset();
            }
            ToUi::Focused => wants |= handler.focused(),
            ToUi::Visible(visible) => {
                crate::environment::set_window_visible(visible);
                wants = true;
            }
            ToUi::Resized(s, k) => {
                size = s;
                scale = k;
                if k > 0.0 {
                    let logical = |p: u32| f32::from(u16::try_from(p).unwrap_or(u16::MAX)) / k;
                    crate::environment::set_window_size((logical(s.0), logical(s.1)));
                }
                wants = true;
            }
            ToUi::Chrome(c) => {
                wants |= handler.chrome(c.clone());
                chrome = c;
            }
            ToUi::Mail => {}
            ToUi::Quit => return,
        }
    }
}

/// Send a held move to `handler`, under the frame it came under.
fn send_move(handler: &mut dyn WindowHandler, ((x, y), shown): HeldMove) -> bool {
    handler.input(Input::PointerMoved { x, y }, &shown)
}

/// The threads of an app, and what links them.
struct Threads {
    to_ui: Sender<ToUi>,
    to_render: Sender<ToRender>,
    /// What the device's lost callback wakes the render thread through.
    #[expect(dead_code, reason = "held: the lost callback holds it weakly")]
    waker: Arc<Sender<ToRender>>,
    presented: Arc<Mutex<Arc<Presented>>>,
    /// How the OS schedules each thread, as each set it when it started.
    priorities: Arc<Mutex<Vec<(&'static str, Applied)>>>,
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
    let presented = Arc::new(Mutex::new(Arc::new(Presented::default())));
    let priorities = Arc::new(Mutex::new(Vec::new()));
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
        Compositor::new(Glyphs::for_os()),
        gpu,
        render_rx,
        Box::new(move |m| {
            ui.send(ToUi::Render(m)).ok();
        }),
        Arc::clone(&presented),
        interval,
        pace,
        log,
    );
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
    };
    let mail = to_ui.clone();
    let render_quit = to_render.clone();
    let noted = Arc::clone(&priorities);
    std::thread::Builder::new()
        .name("gui ui".to_owned())
        .stack_size(crate::layout::STACK)
        .spawn(move || {
            note(&noted, "ui", priority::set_current(ThreadType::Ui));
            crate::mailbox::wake_with(Arc::new(move || {
                mail.send(ToUi::Mail).ok();
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
    })
}

/// Keep what thread `name` was given, for reports.
fn note(to: &Mutex<Vec<(&'static str, Applied)>>, name: &'static str, applied: Applied) {
    to.lock().unwrap_or_else(PoisonError::into_inner).push((name, applied));
}

struct WindowState {
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
    /// Logs the input as it comes (`GUI_RECORD_INPUT`).
    recorder: Option<Recorder>,
    /// Where the input method was last told it may type.
    ime: Option<ImeTarget>,
    proxy: EventLoopProxy<UserEvent>,
    /// What the adapter last said: whether a screen reader listens.
    listening: bool,
    /// The render thread gave up: the app exits with an error.
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
        recorder: None,
        ime: None,
        proxy: event_loop.create_proxy(),
        listening: false,
        failed: false,
        chrome: Chrome::default(),
        forced: forced_decorations(),
        focused: true,
        displays: None,
        told: Told::default(),
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

impl Platform {
    /// Log `event` if recording, and send what it becomes on: the wheel to
    /// the render thread, the rest to the UI thread, stamped now.
    fn forward(&mut self, event: input_log::Event) {
        let at = Instant::now();
        if let Some(r) = &self.recorder {
            r.record(at, &event);
        }
        match self.router.route(event) {
            Routed::Ui(input) => self.send(input),
            Routed::Wheel {
                point,
                delta,
                precise,
                began,
                ended,
            } => {
                if let Some(s) = &self.state {
                    // Stamped with the frame on screen as it came: the
                    // render thread routes it in that frame.
                    let shown = Arc::clone(&s.threads.presented.lock().unwrap_or_else(PoisonError::into_inner));
                    s.threads
                        .to_render
                        .send(ToRender::Wheel {
                            point,
                            delta,
                            at,
                            precise,
                            began,
                            ended,
                            shown,
                        })
                        .ok();
                }
            }
        }
    }

    /// Forward `input` to the UI thread, stamped.
    fn send(&self, input: Input) {
        if let Some(s) = &self.state {
            let shown = Arc::clone(&s.threads.presented.lock().unwrap_or_else(PoisonError::into_inner));
            s.threads.to_ui.send(ToUi::Input(input, Instant::now(), shown)).ok();
        }
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
            s.threads.to_ui.send(ToUi::Chrome(chrome.clone())).ok();
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
        if let Some(s) = &self.state {
            s.threads.to_ui.send(ToUi::Quit).ok();
        }
        event_loop.exit();
    }

    /// The window's size or scale changed: the render thread configures
    /// its surface to the size at once (a swapchain not the window's size
    /// is out of date on Vulkan), showing the last frame at its own size
    /// until the UI commits one laid out at the new size and scale.
    fn resized(&self) {
        let Some(s) = &self.state else { return };
        let size = s.window.inner_size();
        let size = (size.width, size.height);
        s.threads.to_render.send(ToRender::Resize(size)).ok();
        s.threads.to_ui.send(ToUi::Resized(size, self.scale())).ok();
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
        self.recorder = record_input(&window);
        // The OS's displays, where they are read: the wake has this thread
        // poll them. Where they are not (Wayland), winit's rate and the
        // frame callbacks stand, as before.
        let wake = self.proxy.clone();
        self.displays = gui_display::Displays::connect(&window, move || {
            wake.send_event(UserEvent::Display).ok();
        })
        .ok();
        self.state = Some(WindowState {
            window,
            access,
            threads,
        });
        // The monitor's own rate, as reported: what bounds the fit.
        self.follow_display();
        // Sent before the size, so the first frame is laid out with them.
        self.chrome.client = client;
        self.follow_chrome();
        self.resized();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
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
                if focused && let Some(s) = &self.state {
                    s.threads.to_ui.send(ToUi::Focused).ok();
                }
                // Losing focus loses the pointers' capture: what they held
                // is cancelled, in order with the input before it.
                if !focused {
                    self.forward(input_log::Event::Blur);
                }
                self.follow_chrome();
            }
            WindowEvent::ThemeChanged(_) => self.follow_chrome(),
            WindowEvent::Occluded(hidden) => {
                if let Some(s) = &self.state {
                    s.threads.to_render.send(ToRender::Visible(!hidden)).ok();
                    s.threads.to_ui.send(ToUi::Visible(!hidden)).ok();
                }
            }
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
            event => {
                if let Some(input) = input_log::from_winit(event, scale) {
                    self.forward(input);
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
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
                let Some(s) = &self.state else { return };
                let to_ui = &s.threads.to_ui;
                match event.window_event {
                    accesskit_winit::WindowEvent::InitialTreeRequested => {
                        self.listening = true;
                        to_ui.send(ToUi::Access(AccessEvent::Initial)).ok();
                    }
                    accesskit_winit::WindowEvent::ActionRequested(request) => {
                        to_ui.send(ToUi::Access(AccessEvent::Action(request))).ok();
                    }
                    accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                        self.listening = false;
                        to_ui.send(ToUi::Access(AccessEvent::Deactivated)).ok();
                    }
                }
            }
            // Taken in above.
            UserEvent::Display => {}
            UserEvent::Ui(ToPlatform::Cursor(c)) => {
                if let Some(s) = &self.state {
                    s.window.set_cursor(c);
                }
            }
            UserEvent::Ui(ToPlatform::Window(action)) => self.act(event_loop, action),
            UserEvent::Ui(ToPlatform::Ime(target)) => self.follow_ime(target),
            UserEvent::Ui(ToPlatform::Quit(why)) => {
                eprintln!("{why}");
                self.failed = true;
                event_loop.exit();
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
        threads.to_ui.send(ToUi::Resized(size, scale)).ok();
        Ok(Self {
            threads,
            log,
            cursor: (0.0, 0.0),
        })
    }

    /// Send `input` as the platform thread does, stamped now.
    pub fn input(&mut self, input: Input) {
        if let Input::PointerMoved { x, y } | Input::MouseDown { x, y } | Input::MouseUp { x, y } = input {
            self.cursor = (x, y);
        }
        let shown = self.presented();
        self.threads.to_ui.send(ToUi::Input(input, Instant::now(), shown)).ok();
    }

    /// Turn the wheel at the pointer by `delta` (a trackpad's when `precise`).
    pub fn wheel(&mut self, delta: (f32, f32), precise: bool) {
        self.threads
            .to_render
            .send(ToRender::Wheel {
                point: self.cursor,
                delta,
                at: Instant::now(),
                precise,
                began: false,
                ended: false,
                shown: self.presented(),
            })
            .ok();
    }

    /// What is on screen now.
    #[must_use]
    pub fn presented(&self) -> Arc<Presented> {
        Arc::clone(&self.threads.presented.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The frames the render thread drew since the last call.
    #[must_use]
    pub fn take_frames(&self) -> Vec<FrameStat> {
        std::mem::take(&mut *self.log.lock().unwrap_or_else(PoisonError::into_inner))
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

    /// Make the render thread panic, as a bug in it would (invariant 7: it
    /// starts afresh from a full commit of the UI's).
    pub fn panic_render(&self) {
        self.threads.to_render.send(ToRender::Panic).ok();
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
                UserEvent::Ui(super::ToPlatform::Quit(String::new())),
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

    impl super::WindowHandler for Seen {
        fn frame(&mut self, _: &mut dyn super::Compose, _: super::MainFrame, _: (u32, u32), _: f32) {}
        fn input(&mut self, input: super::Input, presented: &std::sync::Arc<super::Presented>) -> bool {
            self.0.push(format!("{input:?} @{}", presented.frame));
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
        fn renderer_lost(&mut self) {}
        fn chrome(&mut self, _: super::Chrome) -> bool {
            false
        }
        fn window_actions(&mut self) -> Vec<super::WindowAction> {
            Vec::new()
        }
    }

    #[test]
    fn held_moves_keep_the_frame_they_came_under_and_go_before_what_follows() {
        use std::sync::{Arc, Mutex, mpsc};

        use super::{FromRender, Input, MainFrame, Presented, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: Arc::new(Mutex::new(Arc::new(Presented::default()))),
            spare: None,
        };
        let shown = |frame| {
            Arc::new(Presented {
                frame,
                ..Presented::default()
            })
        };
        let (a, b) = (shown(1), shown(2));
        let now = std::time::Instant::now();
        let input = |i, p: &Arc<Presented>| ToUi::Input(i, now, Arc::clone(p));
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
        use std::sync::{Arc, Mutex, mpsc};

        use super::{FromRender, Input, Presented, ToUi};
        let (to_ui, rx) = mpsc::channel();
        let (to_render, _render) = mpsc::channel();
        let mut link = super::RenderLink {
            tx: to_render,
            feedback: super::Feedback::default(),
            presented: Arc::new(Mutex::new(Arc::new(Presented::default()))),
            spare: None,
        };
        let shown = |frame| {
            Arc::new(Presented {
                frame,
                ..Presented::default()
            })
        };
        let now = std::time::Instant::now();
        // The pointer last moved under frame 1 at (10, 10); the compositor
        // then forwards a wheel it found at (50, 60) under frame 2: the
        // wheel lands there, under that frame, after the held move.
        for msg in [
            ToUi::Input(Input::MouseDown { x: 10.0, y: 10.0 }, now, shown(1)),
            ToUi::Input(Input::PointerMoved { x: 10.0, y: 10.0 }, now, shown(1)),
            ToUi::Render(FromRender::Wheel {
                step: crate::compositor::WheelStep {
                    point: (50.0, 60.0),
                    ..crate::compositor::WheelStep::default()
                },
                delta: (0.0, 40.0),
                shown: shown(2),
                target: None,
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
                "WheelAt { step: WheelStep { point: (50.0, 60.0), at: 0.0, precise: false, began: false, ended: false }, dx: 0.0, dy: 40.0, target: None } @2"
            ]
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
