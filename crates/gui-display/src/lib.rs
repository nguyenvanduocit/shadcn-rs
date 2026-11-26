//! The OS display boundary (ADR-0029).
//!
//! What a toolkit must ask the OS about its displays, behind a safe API:
//!
//! - **topology**: the outputs there are, where each is, and when one is
//!   plugged in, pulled out or changed ([`Displays::poll`], [`Event`]);
//! - **refresh**: each output's interval and, where it varies, the range
//!   it varies in ([`Refresh`]);
//! - **colour**: each output's gamut and HDR headroom ([`Colour`]);
//! - **vsync**: each output's ticks, every time labelled as predicted,
//!   presented or synthesized ([`Ticks`], [`Stamp`]).
//!
//! This is the one crate of the workspace that may hold `unsafe`. Each site
//! is one function holding one unsafe operation, lifted from the crate's
//! `deny(unsafe_code)` by an `#[expect]` that says why, with a `// SAFETY:`
//! at the block; `tests/sites.rs` pins the list.
//!
//! **Threads.** A [`Displays`] belongs to the thread that made it (on macOS
//! the main thread: AppKit's screens are read only there) and is neither
//! `Send` nor `Sync`. The OS tells of a change on a thread of its choosing;
//! all that happens there is a flag set and the owner's `wake` called. The
//! owner then calls [`Displays::poll`], which reads the outputs afresh and
//! returns the difference from what it knew. So an event never describes a
//! half-made reconfiguration: it is what the OS said at one moment, and a
//! later notification gives a later difference. [`Output`]s are plain data
//! and [`Ticks`] may go to any thread.

pub mod edid;
pub mod gamut;
#[cfg(target_os = "macos")]
mod macos;
mod scroll;
mod ticks;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod x11;

pub use scroll::{Scroll, ScrollModifiers, ScrollPhase, ScrollTap, tap_scrolls};

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use gamut::Gamut;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use ticks::{Feed, Link, Settle, Source, Wake};
pub use ticks::{IDLE, Stamp, Tick, Ticks};

/// An output for as long as it stays connected in this session. Never used
/// again for another output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OutputId(u64);

/// What stays the same for one monitor across sessions and replugging: on
/// macOS the display's UUID, on X11 its EDID identity with its connector's
/// name. An app keys what it remembers per monitor by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OutputKey(pub [u8; 16]);

/// Where an output is, in the OS's global coordinates and units (macOS:
/// points, origin at the primary screen's bottom left; X11: pixels, origin
/// top left), and how many device pixels it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub pixels: (u32, u32),
}

/// The range a variable-refresh output varies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RefreshRange {
    /// The shortest and the longest time one frame stays on screen.
    pub shortest: Duration,
    pub longest: Duration,
    /// The step a frame's time on screen moves in, when the OS says (macOS);
    /// none where it is not told or is continuous.
    pub granularity: Option<Duration>,
}

/// How often an output refreshes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refresh {
    /// The interval of its current mode.
    pub interval: Duration,
    /// The range it varies in, for a variable-refresh output.
    pub range: Option<RefreshRange>,
}

/// How much brighter than SDR white an output can show, as a multiple of
/// it: 1 is no headroom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Headroom {
    /// What it can show now (it moves with the brightness setting and with
    /// what is on screen).
    pub current: f32,
    /// The most it could show.
    pub potential: f32,
    /// The headroom of its reference mode, for a reference display.
    pub reference: Option<f32>,
}

impl Headroom {
    /// An output with no headroom: SDR.
    pub const SDR: Self = Self {
        current: 1.0,
        potential: 1.0,
        reference: None,
    };
}

/// An output's colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colour {
    pub gamut: Gamut,
    pub headroom: Headroom,
}

/// Everything about an output but its id: what the OS says of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub key: OutputKey,
    /// The OS's own handle for it, which windowing toolkits report too
    /// (winit's `native_id`): macOS's `CGDirectDisplayID`, X11's RandR CRTC.
    /// It can change while the output stays (a laptop switching GPUs).
    pub native: u32,
    pub name: String,
    pub geometry: Geometry,
    pub refresh: Refresh,
    pub colour: Colour,
}

/// An output as last read.
#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    pub id: OutputId,
    pub description: Description,
}

impl std::ops::Deref for Output {
    type Target = Description;

    fn deref(&self) -> &Description {
        &self.description
    }
}

/// What changed between two polls. Every event names its output.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Added(Output),
    /// The output as it was last known.
    Removed(Output),
    /// Something of `output` differs from `was`: compare the parts.
    Changed {
        output: Output,
        was: Output,
    },
}

impl Event {
    /// The output it is about.
    #[must_use]
    pub fn output(&self) -> OutputId {
        match self {
            Self::Added(o) | Self::Removed(o) | Self::Changed { output: o, .. } => o.id,
        }
    }
}

#[derive(Debug)]
pub enum Error {
    /// The displays of this windowing system are not read yet (Wayland,
    /// Windows: see ADR-0029), or there is none (headless).
    Unsupported(&'static str),
    /// On macOS the displays are read on the main thread only.
    NotMainThread,
    /// No output with that id is connected.
    NoSuchOutput,
    /// The OS refused, and what it said.
    Os(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(what) => write!(f, "displays are not read on {what}"),
            Self::NotMainThread => f.write_str("displays are read on the main thread only"),
            Self::NoSuchOutput => f.write_str("no such output"),
            Self::Os(why) => write!(f, "the OS refused: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// One windowing system's way to its displays.
trait Backend {
    /// Every connected output as the OS describes it now. An error when
    /// any part of that could not be read: a partial answer would read as
    /// outputs pulled out, so the poll keeps what it knew and asks again.
    fn probe(&mut self) -> Result<Vec<Description>, Error>;
    /// A vsync source for the output with this native handle, stopped,
    /// writing into `feed`.
    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error>;
    /// Bring what runs back to what is wanted, where the OS refused a
    /// stop before (macOS: a display link nobody reads that still runs).
    /// Called at every poll.
    fn tidy(&mut self) {}
    /// Whether the OS tells of every change ([`Notice::changed`]). A
    /// backend that cannot listen (X11 with no notification connection)
    /// says not, and every poll reads the outputs afresh: a poll the
    /// window's events ask for is then how a change is seen.
    fn notifies(&self) -> bool {
        true
    }
}

/// The flag an OS notification sets, and the owner's wake.
#[derive(Clone)]
struct Notice {
    dirty: Arc<AtomicBool>,
    wake: Wake,
}

impl Notice {
    /// The OS said something about the displays changed.
    fn changed(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        (self.wake)();
    }
}

struct Subscription {
    link: Link,
    source: Box<dyn Source>,
}

/// The displays of this session. See the crate's documentation for the
/// threading rules.
pub struct Displays {
    backend: Box<dyn Backend>,
    outputs: Vec<Output>,
    next: u64,
    notice: Notice,
    subscriptions: Vec<Subscription>,
    /// Owned by its thread: AppKit's screens, and the vsync sources.
    not_send: PhantomData<*const ()>,
    /// The window [`Self::connect`] was given, kept alive while the
    /// backend may use the toolkit's display connection it holds (X11).
    /// Declared last: dropped after the backend.
    _owner: Option<Owner>,
}

/// What keeps a toolkit's window alive.
type Owner = Arc<dyn std::any::Any + Send + Sync>;

impl std::fmt::Debug for Displays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Displays")
            .field("outputs", &self.outputs)
            .finish_non_exhaustive()
    }
}

impl Displays {
    /// The displays of the windowing system `window` is of. On X11 they
    /// are read through the toolkit's own connection, on the screen the
    /// window is on; `window` is kept alive for as long as these displays
    /// are, which keeps that connection open (raw-window-handle's contract:
    /// a handle is valid while what gave it lives).
    ///
    /// `wake` is called, on any thread, when [`Self::poll`] has something
    /// to do. It should have the owner's thread call it, and do nothing
    /// else; it **must not block**: it may run on the OS's vsync thread,
    /// whose next vblank waits for it. It is never called under a lock of
    /// this crate's, so a wake that takes a lock of the caller's cannot
    /// close a cycle with one of ours, but it can still delay a vblank.
    ///
    /// # Errors
    /// [`Error::Unsupported`] where this crate does not read the displays
    /// (Wayland, Windows, macOS before 11, an X server without RandR 1.3),
    /// [`Error::NotMainThread`] on macOS off the main thread, and
    /// [`Error::Os`] when the OS refuses the connection.
    pub fn connect<W>(window: &Arc<W>, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Error>
    where
        W: HasDisplayHandle + HasWindowHandle + Send + Sync + 'static,
    {
        let notice = Notice {
            dirty: Arc::new(AtomicBool::new(true)),
            wake: Arc::new(wake),
        };
        let display = window.display_handle().map_err(|e| Error::Os(e.to_string()))?.as_raw();
        // A toolkit may give no window handle (an event loop, a test); the
        // backend then takes the display's default screen.
        let surface = window.window_handle().ok().map(|w| w.as_raw());
        let backend = system(display, surface, &notice)?;
        let owner: Owner = Arc::<W>::clone(window);
        Ok(Self::with(backend, notice, Some(owner)))
    }

    fn with(backend: Box<dyn Backend>, notice: Notice, owner: Option<Owner>) -> Self {
        let mut displays = Self {
            backend,
            outputs: Vec::new(),
            next: 1,
            notice,
            subscriptions: Vec::new(),
            not_send: PhantomData,
            _owner: owner,
        };
        // What is connected at the start is known, not news.
        displays.poll();
        displays
    }

    /// Displays a test scripts: outputs plugged, changed and pulled, and
    /// ticks, through the [`Script`]. Everything above the OS runs as it
    /// does on a real one.
    #[must_use]
    pub fn scripted(wake: impl Fn() + Send + Sync + 'static) -> (Self, Script) {
        let notice = Notice {
            dirty: Arc::new(AtomicBool::new(true)),
            wake: Arc::new(wake),
        };
        let state = Arc::new(Mutex::new(Scripted::default()));
        let script = Script {
            state: Arc::clone(&state),
            notice: notice.clone(),
        };
        (Self::with(Box::new(ScriptedBackend(state)), notice, None), script)
    }

    /// The outputs as of the last poll.
    #[must_use]
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    #[must_use]
    pub fn output(&self, id: OutputId) -> Option<&Output> {
        self.outputs.iter().find(|o| o.id == id)
    }

    /// The output with the OS handle `native` ([`Description::native`]).
    #[must_use]
    pub fn by_native(&self, native: u32) -> Option<&Output> {
        self.outputs.iter().find(|o| o.native == native)
    }

    /// Take in what the OS said since the last poll: the outputs are read
    /// afresh when it said anything, and the difference returned; vsync
    /// sources are started and stopped as their readers want. Cheap when
    /// nothing happened.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        // Not `||`'s short cut: the flag is cleared either way.
        let told = self.notice.dirty.swap(false, Ordering::SeqCst);
        if told || !self.backend.notifies() {
            match self.backend.probe() {
                Ok(probed) => events = reconcile(&mut self.outputs, probed, &mut self.next),
                // What could not be read changes nothing: the outputs stay
                // as last read whole, and the next poll reads them again.
                Err(_) => self.notice.dirty.store(true, Ordering::SeqCst),
            }
            for e in &events {
                match e {
                    // A changed handle is another display link, another
                    // CRTC: the old source ticks for nothing this output is on.
                    Event::Removed(o) => self.end(o.id),
                    Event::Changed { output, was } if output.native != was.native => self.end(output.id),
                    // Another interval: its ticks are of the next
                    // generation, and a timer ticks at the new interval.
                    // (A consumer told the new interval takes only those.)
                    Event::Changed { output, was } if output.refresh.interval != was.refresh.interval => {
                        for s in self.subscriptions.iter().filter(|s| s.link.output() == output.id) {
                            s.link.remode(output.refresh.interval);
                        }
                    }
                    _ => {}
                }
            }
        }
        let now = Instant::now();
        self.subscriptions.retain_mut(|s| match s.link.settle(now) {
            Settle::Keep => true,
            // A source that would not start again ends its subscription:
            // the reader sees it ended, and a new one tries afresh.
            Settle::Start => match s.source.start() {
                Ok(()) => true,
                Err(_) => {
                    s.link.end();
                    false
                }
            },
            Settle::Stop => {
                s.source.stop();
                true
            }
            Settle::Close => {
                s.source.stop();
                false
            }
        });
        self.backend.tidy();
        events
    }

    /// End the subscriptions to `output`.
    fn end(&self, output: OutputId) {
        for s in self.subscriptions.iter().filter(|s| s.link.output() == output) {
            s.link.end();
        }
    }

    /// Subscribe to the vsync of `output`: its source is started now.
    ///
    /// # Errors
    /// [`Error::NoSuchOutput`], or [`Error::Unsupported`] where the OS has
    /// no vsync source this crate reads (X11), or [`Error::Os`], also when
    /// the source would not start: nothing is kept of it then, and a later
    /// call tries again.
    pub fn ticks(&mut self, output: OutputId) -> Result<Ticks, Error> {
        let described = self.output(output).ok_or(Error::NoSuchOutput)?.description.clone();
        let (ticks, link) = ticks::subscription(output, described.refresh.interval, Arc::clone(&self.notice.wake));
        let source = self.backend.source(&described, link.feed())?;
        self.subscribe(ticks, link, source)
    }

    /// Ticks from a timer at the nominal refresh of `output`, each a
    /// [`Stamp::Synthesized`]: for a caller that wants one cadence whether
    /// the OS has a vsync source or not. They carry no phase, and follow
    /// the output's mode as the polls read it.
    ///
    /// # Errors
    /// [`Error::NoSuchOutput`], or [`Error::Os`] when no thread can start.
    pub fn synthesized(&mut self, output: OutputId) -> Result<Ticks, Error> {
        let interval = self.output(output).ok_or(Error::NoSuchOutput)?.refresh.interval;
        let (ticks, link) = ticks::subscription(output, interval, Arc::clone(&self.notice.wake));
        let source = Box::new(ticks::Timer::spawn(link.feed()).map_err(|e| Error::Os(e.to_string()))?);
        self.subscribe(ticks, link, source)
    }

    /// Start a new subscription's source and keep it; on an error, drop it.
    fn subscribe(&mut self, ticks: Ticks, link: Link, mut source: Box<dyn Source>) -> Result<Ticks, Error> {
        if link.settle(Instant::now()) == Settle::Start
            && let Err(e) = source.start()
        {
            link.end();
            return Err(e);
        }
        self.subscriptions.push(Subscription { link, source });
        Ok(ticks)
    }
}

impl Drop for Displays {
    fn drop(&mut self) {
        for s in &mut self.subscriptions {
            s.link.end();
            s.source.stop();
        }
    }
}

/// The backend for the windowing system behind `display`, and the window
/// `surface` when the toolkit gave one.
#[cfg(target_os = "macos")]
fn system(
    display: raw_window_handle::RawDisplayHandle,
    _: Option<raw_window_handle::RawWindowHandle>,
    notice: &Notice,
) -> Result<Box<dyn Backend>, Error> {
    match display {
        raw_window_handle::RawDisplayHandle::AppKit(_) => Ok(Box::new(macos::MacOs::new(notice.clone())?)),
        _ => Err(Error::Unsupported("this windowing system")),
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
fn system(
    display: raw_window_handle::RawDisplayHandle,
    surface: Option<raw_window_handle::RawWindowHandle>,
    notice: &Notice,
) -> Result<Box<dyn Backend>, Error> {
    use raw_window_handle::RawDisplayHandle;
    match display {
        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => {
            Ok(Box::new(x11::X11::new(display, surface, notice.clone())?))
        }
        RawDisplayHandle::Wayland(_) => Err(Error::Unsupported("Wayland")),
        _ => Err(Error::Unsupported("this windowing system")),
    }
}

#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "android")))))]
fn system(
    _: raw_window_handle::RawDisplayHandle,
    _: Option<raw_window_handle::RawWindowHandle>,
    _: &Notice,
) -> Result<Box<dyn Backend>, Error> {
    Err(Error::Unsupported("this windowing system"))
}

/// How many native vsync sources (macOS display links) this process has
/// made. They are never released (ADR-0029): one per physical display for
/// the life of the process, however many readers and connections come and
/// go, and this lets a check prove it. Zero where there are none.
#[must_use]
pub fn native_links_made() -> usize {
    #[cfg(target_os = "macos")]
    {
        macos::links_made()
    }
    #[cfg(not(target_os = "macos"))]
    {
        0
    }
}

/// Bring `known` to what was `probed`, and say what changed. An output is
/// the one known before when its key is the same; among several with one
/// key (two monitors of a model without serial numbers), the one with the
/// same OS handle first. So a handle that changes under a display that
/// stays (a GPU switch) is a change, not a removal and an addition.
fn reconcile(known: &mut Vec<Output>, probed: Vec<Description>, next: &mut u64) -> Vec<Event> {
    let mut before: Vec<Option<Output>> = std::mem::take(known).into_iter().map(Some).collect();
    let mut events = Vec::new();
    let mut take = |d: &Description, exact: bool| -> Option<Output> {
        before
            .iter_mut()
            .find(|o| {
                o.as_ref()
                    .is_some_and(|o| o.key == d.key && (!exact || o.native == d.native))
            })
            .and_then(Option::take)
    };
    // Exact matches first, so a twin does not take its sibling's id.
    let mut matched: Vec<(Description, Option<Output>)> = probed
        .into_iter()
        .map(|d| {
            let was = take(&d, true);
            (d, was)
        })
        .collect();
    for (d, was) in &mut matched {
        if was.is_none() {
            *was = take(d, false);
        }
    }
    for (description, was) in matched {
        match was {
            Some(was) => {
                let output = Output {
                    id: was.id,
                    description,
                };
                if output != was {
                    events.push(Event::Changed {
                        output: output.clone(),
                        was,
                    });
                }
                known.push(output);
            }
            None => {
                let output = Output {
                    id: OutputId(*next),
                    description,
                };
                *next = next.saturating_add(1);
                events.push(Event::Added(output.clone()));
                known.push(output);
            }
        }
    }
    // Removals first: a consumer moves off a dead output before it hears
    // of the ones that came.
    let mut all: Vec<Event> = before.into_iter().flatten().map(Event::Removed).collect();
    all.append(&mut events);
    all
}

#[derive(Default)]
struct Scripted {
    outputs: Vec<Description>,
    /// Each source made: its output's native handle, its feed, whether it runs.
    sources: Vec<(u32, Feed, Arc<AtomicBool>)>,
    /// How many of the next probes fail.
    probes_failing: usize,
    /// How many of the next source starts fail.
    starts_failing: usize,
    /// The OS tells of nothing (X11 with no notification connection).
    silent: bool,
}

struct ScriptedBackend(Arc<Mutex<Scripted>>);

struct ScriptedSource {
    state: Arc<Mutex<Scripted>>,
    running: Arc<AtomicBool>,
}

impl Source for ScriptedSource {
    fn start(&mut self) -> Result<(), Error> {
        let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(left) = s.starts_failing.checked_sub(1) {
            s.starts_failing = left;
            return Err(Error::Os("a scripted start failed".to_owned()));
        }
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Backend for ScriptedBackend {
    fn probe(&mut self) -> Result<Vec<Description>, Error> {
        let mut s = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(left) = s.probes_failing.checked_sub(1) {
            s.probes_failing = left;
            return Err(Error::Os("a scripted probe failed".to_owned()));
        }
        Ok(s.outputs.clone())
    }

    fn notifies(&self) -> bool {
        !self.0.lock().unwrap_or_else(PoisonError::into_inner).silent
    }

    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error> {
        let running = Arc::new(AtomicBool::new(false));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sources
            .push((output.native, feed, Arc::clone(&running)));
        Ok(Box::new(ScriptedSource {
            state: Arc::clone(&self.0),
            running,
        }))
    }
}

/// The OS of [`Displays::scripted`], played by a test from any thread.
#[derive(Clone)]
pub struct Script {
    state: Arc<Mutex<Scripted>>,
    notice: Notice,
}

impl Script {
    /// A plain SDR output at 60 Hz, to be changed as the test needs.
    #[must_use]
    pub fn output(native: u32, name: &str) -> Description {
        let mut key = [0u8; 16];
        for (k, b) in key.iter_mut().zip(name.bytes()) {
            *k = b;
        }
        Description {
            key: OutputKey(key),
            native,
            name: name.to_owned(),
            geometry: Geometry {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0,
                pixels: (1920, 1080),
            },
            refresh: Refresh {
                interval: Duration::from_nanos(16_666_667),
                range: None,
            },
            colour: Colour {
                gamut: Gamut::Srgb,
                headroom: Headroom::SDR,
            },
        }
    }

    /// Change the outputs. The test's closure runs under no lock of the
    /// script's (it may read the script), on a copy put back after it. A
    /// silent OS ([`Self::silence`]) tells nobody.
    fn edit(&self, f: impl FnOnce(&mut Vec<Description>)) {
        let mut outputs = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .outputs
            .clone();
        f(&mut outputs);
        let silent = {
            let mut s = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            s.outputs = outputs;
            s.silent
        };
        if !silent {
            self.notice.changed();
        }
    }

    /// From now on the OS tells of no change, as X11 with no notification
    /// connection: the outputs change unannounced.
    pub fn silence(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).silent = true;
    }

    /// Plug an output in, as the OS would tell of it.
    pub fn plug(&self, output: Description) {
        self.edit(|o| o.push(output));
    }

    /// Pull out the output with this handle.
    pub fn unplug(&self, native: u32) {
        self.edit(|o| o.retain(|d| d.native != native));
    }

    /// Change the output with this handle.
    pub fn change(&self, native: u32, f: impl FnOnce(&mut Description)) {
        self.edit(|o| {
            if let Some(d) = o.iter_mut().find(|d| d.native == native) {
                f(d);
            }
        });
    }

    /// A vsync of the output with this handle, from every source made for
    /// it that runs (a stopped one calls nothing back).
    pub fn tick(&self, native: u32, target: Stamp, interval: Option<Duration>) {
        let feeds: Vec<Feed> = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sources
            .iter()
            .filter(|(n, _, running)| *n == native && running.load(Ordering::SeqCst))
            .map(|(_, feed, _)| feed.clone())
            .collect();
        // The script's lock is released: a wake may run.
        for feed in feeds {
            feed.tick_now(target, interval).send();
        }
    }

    /// The next `n` source starts fail, as an OS may refuse one.
    pub fn fail_starts(&self, n: usize) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).starts_failing = n;
    }

    /// The next `n` reads of the outputs fail part way, as a request to
    /// the X server may; the OS is said to have changed, so the owner polls.
    pub fn fail_probes(&self, n: usize) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).probes_failing = n;
        self.notice.changed();
    }

    /// How many sources for the output with this handle run now.
    #[must_use]
    pub fn running(&self, native: u32) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sources
            .iter()
            .filter(|(n, _, running)| *n == native && running.load(Ordering::SeqCst))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;

    fn scripted() -> (Displays, Script, Arc<AtomicUsize>) {
        let woken = Arc::new(AtomicUsize::new(0));
        let theirs = Arc::clone(&woken);
        let (displays, script) = Displays::scripted(move || {
            theirs.fetch_add(1, Ordering::SeqCst);
        });
        (displays, script, woken)
    }

    #[test]
    fn plugging_changing_and_pulling_an_output_are_events_that_name_it() {
        let (mut displays, script, woken) = scripted();
        assert!(displays.outputs().is_empty());
        script.plug(Script::output(1, "built-in"));
        assert_eq!(woken.load(Ordering::SeqCst), 1, "the owner is woken to poll");
        assert!(displays.outputs().is_empty(), "nothing is taken in before the poll");
        let events = displays.poll();
        let id = displays.by_native(1).unwrap().id;
        assert!(matches!(events.as_slice(), [Event::Added(o)] if o.id == id && o.name == "built-in"));
        assert_eq!(displays.poll(), vec![], "nothing happened since");

        script.change(1, |d| d.refresh.interval = Duration::from_nanos(8_333_333));
        let events = displays.poll();
        match events.as_slice() {
            [Event::Changed { output, was }] => {
                assert_eq!((output.id, was.id), (id, id));
                assert_eq!(output.refresh.interval, Duration::from_nanos(8_333_333));
                assert_eq!(was.refresh.interval, Duration::from_nanos(16_666_667));
                assert_eq!(events.first().map(Event::output), Some(id));
            }
            other => panic!("{other:?}"),
        }

        script.unplug(1);
        let events = displays.poll();
        assert!(matches!(events.as_slice(), [Event::Removed(o)] if o.id == id));
        assert_eq!(displays.output(id), None);
    }

    #[test]
    fn headroom_and_gamut_changes_are_changes_of_their_output_alone() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        script.plug(Script::output(2, "external"));
        displays.poll();
        let hdr = displays.by_native(1).unwrap().id;
        script.change(1, |d| {
            d.colour = Colour {
                gamut: Gamut::DisplayP3,
                headroom: Headroom {
                    current: 2.0,
                    potential: 16.0,
                    reference: None,
                },
            };
        });
        let events = displays.poll();
        assert_eq!(events.len(), 1);
        assert_eq!(events.first().map(Event::output), Some(hdr));
        let o = displays.output(hdr).unwrap();
        assert_eq!((o.colour.gamut, o.colour.headroom.current), (Gamut::DisplayP3, 2.0));
    }

    #[test]
    fn an_id_is_never_used_again_and_a_replugged_monitor_keeps_its_key() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "external"));
        displays.poll();
        let first = displays.by_native(1).unwrap().clone();
        script.unplug(1);
        displays.poll();
        script.plug(Script::output(9, "external"));
        displays.poll();
        let second = displays.by_native(9).unwrap();
        assert_ne!(second.id, first.id);
        assert_eq!(second.key, first.key);
    }

    #[test]
    fn a_handle_that_changes_under_a_display_that_stays_is_a_change() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.ticks(id).unwrap();
        // A laptop switched GPUs: the same panel under another handle.
        script.change(1, |d| d.native = 5);
        let events = displays.poll();
        assert!(matches!(events.as_slice(), [Event::Changed { output, was }] if output.id == id && was.native == 1));
        assert_eq!(displays.by_native(5).map(|o| o.id), Some(id));
        assert!(ticks.ended(), "the old handle's source is no longer this output's");
        assert_eq!(script.running(1), 0);
    }

    #[test]
    fn twins_with_one_key_keep_their_own_ids() {
        let (mut displays, script, _) = scripted();
        let twin = |native| Description {
            native,
            ..Script::output(0, "twin")
        };
        script.plug(twin(1));
        script.plug(twin(2));
        displays.poll();
        let (a, b) = (displays.by_native(1).unwrap().id, displays.by_native(2).unwrap().id);
        assert_ne!(a, b);
        // The OS lists them the other way round, and one changed.
        script.edit(|o| {
            o.reverse();
            if let Some(d) = o.first_mut() {
                d.geometry.x = 1920.0;
            }
        });
        let events = displays.poll();
        assert_eq!(events.len(), 1);
        assert_eq!(events.first().map(Event::output), Some(b));
        assert_eq!(
            (displays.by_native(1).unwrap().id, displays.by_native(2).unwrap().id),
            (a, b)
        );
    }

    #[test]
    fn a_pulled_output_is_told_before_the_one_that_came() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "old"));
        displays.poll();
        script.edit(|o| {
            o.clear();
            o.push(Script::output(2, "new"));
        });
        let events = displays.poll();
        assert!(
            matches!(events.as_slice(), [Event::Removed(_), Event::Added(_)]),
            "{events:?}"
        );
    }

    #[test]
    fn ticks_reach_the_reader_labelled_and_stop_with_their_output() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.ticks(id).unwrap();
        assert_eq!(script.running(1), 1, "started at once");
        let at = Instant::now();
        script.tick(1, Stamp::Predicted(at), Some(Duration::from_millis(8)));
        let t = ticks.latest().unwrap();
        assert_eq!((t.output, t.target, t.sequence), (id, Stamp::Predicted(at), 1));

        script.unplug(1);
        displays.poll();
        assert!(ticks.ended());
        assert_eq!(script.running(1), 0, "its source is stopped");
        assert!(matches!(displays.ticks(id), Err(Error::NoSuchOutput)));
    }

    #[test]
    fn a_source_nobody_wants_is_stopped_at_a_poll_and_started_when_wanted() {
        let (mut displays, script, woken) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.ticks(id).unwrap();
        ticks.run(false);
        displays.poll();
        assert_eq!(script.running(1), 1, "kept for the idle time");
        std::thread::sleep(IDLE + Duration::from_millis(20));
        let before = woken.load(Ordering::SeqCst);
        script.tick(1, Stamp::Predicted(Instant::now()), None);
        assert_eq!(
            woken.load(Ordering::SeqCst),
            before + 1,
            "the callback asks the owner to stop it"
        );
        displays.poll();
        assert_eq!(script.running(1), 0);
        ticks.run(true);
        assert_eq!(woken.load(Ordering::SeqCst), before + 2);
        displays.poll();
        assert_eq!(script.running(1), 1);
        drop(ticks);
        displays.poll();
        assert_eq!(script.running(1), 0, "a dropped reader's source is closed");
    }

    #[test]
    fn synthesized_ticks_are_labelled_so() {
        let (mut displays, script, _) = scripted();
        let mut fast = Script::output(1, "built-in");
        fast.refresh.interval = Duration::from_millis(4);
        script.plug(fast);
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.synthesized(id).unwrap();
        let t = ticks.wait(0, Duration::from_secs(5)).unwrap();
        assert!(matches!(t.target, Stamp::Synthesized(_)));
        assert_eq!(t.target.of_the_display(), None);
        assert_eq!(t.interval, Some(Duration::from_millis(4)));
    }

    #[test]
    fn synthesized_ticks_follow_the_outputs_mode() {
        let (mut displays, script, _) = scripted();
        let mut fast = Script::output(1, "built-in");
        fast.refresh.interval = Duration::from_millis(4);
        script.plug(fast);
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.synthesized(id).unwrap();
        let first = ticks.wait(0, Duration::from_secs(5)).unwrap();
        assert_eq!((first.interval, first.generation), (Some(Duration::from_millis(4)), 0));
        // The mode changes again and again, each time while the timer
        // sleeps toward a tick of the old one: the poll reports it, and the
        // first tick after the poll is of the new mode, at its interval.
        for round in 1..=6_u64 {
            let interval = Duration::from_millis(if round % 2 == 1 { 6 } else { 4 });
            script.change(1, |d| d.refresh.interval = interval);
            assert!(!displays.poll().is_empty());
            assert_eq!(ticks.generation(), round);
            let after = ticks.latest().map_or(0, |t| t.sequence);
            let t = ticks.wait(after, Duration::from_secs(5)).unwrap();
            assert_eq!(
                (t.generation, t.interval),
                (round, Some(interval)),
                "the first tick after the change, round {round}"
            );
            let next = ticks.wait(t.sequence, Duration::from_secs(5)).unwrap();
            assert!(
                next.target.at().duration_since(t.target.at()) >= interval,
                "ticks the new interval apart"
            );
        }
    }

    #[test]
    fn with_no_notifications_every_poll_reads_the_outputs() {
        let (mut displays, script, woken) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        script.silence();
        let before = woken.load(Ordering::SeqCst);
        // A mode change nobody is told of; the window's next event polls.
        script.change(1, |d| d.refresh.interval = Duration::from_millis(8));
        assert_eq!(woken.load(Ordering::SeqCst), before, "no notification");
        let events = displays.poll();
        assert!(matches!(events.as_slice(), [Event::Changed { .. }]), "{events:?}");
        // And the next change too: the flag is never what gates it.
        script.unplug(1);
        assert!(matches!(displays.poll().as_slice(), [Event::Removed(_)]));
        assert_eq!(displays.poll(), vec![]);
    }

    #[test]
    fn a_scripted_change_runs_its_closure_under_no_lock_of_the_scripts() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let _ticks = displays.ticks(id).unwrap();
        let (done, finished) = std::sync::mpsc::channel();
        let theirs = script;
        std::thread::spawn(move || {
            let reader = theirs.clone();
            theirs.change(1, |d| d.name = format!("running {}", reader.running(1)));
            done.send(()).ok();
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the closure waited for the script's own lock"
        );
        displays.poll();
        assert_eq!(displays.output(id).map(|o| o.name.as_str()), Some("running 1"));
    }

    #[test]
    fn a_source_that_fails_to_start_is_not_kept_and_a_later_reader_starts_it() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        script.fail_starts(1);
        assert!(matches!(displays.ticks(id), Err(Error::Os(_))));
        assert_eq!(script.running(1), 0);
        displays.poll();
        assert!(displays.subscriptions.is_empty(), "nothing kept of it");
        let ticks = displays.ticks(id).unwrap();
        assert_eq!(script.running(1), 1, "the next reader starts it");

        // Stopped while unwanted, then wanted again and refused: the
        // reader sees its ticks end, and a new one starts afresh.
        ticks.run(false);
        displays.subscriptions.iter().for_each(|s| {
            s.link
                .unwanted_since(Instant::now().checked_sub(IDLE + Duration::from_millis(1)).unwrap());
        });
        displays.poll();
        assert_eq!(script.running(1), 0);
        script.fail_starts(1);
        ticks.run(true);
        displays.poll();
        assert!(ticks.ended());
        assert!(displays.subscriptions.is_empty());
        let again = displays.ticks(id).unwrap();
        assert!(!again.ended());
        assert_eq!(script.running(1), 1);
    }

    #[test]
    fn a_probe_that_fails_keeps_the_outputs_and_is_tried_again() {
        let (mut displays, script, _) = scripted();
        script.plug(Script::output(1, "built-in"));
        displays.poll();
        let id = displays.by_native(1).unwrap().id;
        let ticks = displays.ticks(id).unwrap();
        // The OS changed a mode, and the read of it fails part way.
        script.fail_probes(1);
        for d in &mut script.state.lock().unwrap().outputs {
            d.refresh.interval = Duration::from_millis(8);
        }
        assert_eq!(displays.poll(), vec![], "no output is pulled out by a failed read");
        assert_eq!(displays.output(id).map(|o| o.native), Some(1));
        assert!(!ticks.ended());
        // Nothing told of anything since; the next poll reads again.
        let events = displays.poll();
        assert!(
            matches!(events.as_slice(), [Event::Changed { output, .. }] if output.refresh.interval == Duration::from_millis(8)),
            "{events:?}"
        );
        assert_eq!(displays.poll(), vec![], "and then not again");
    }
}
