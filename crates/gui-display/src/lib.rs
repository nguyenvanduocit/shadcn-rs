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
mod ticks;
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
mod x11;

use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use gamut::Gamut;
use raw_window_handle::HasDisplayHandle;
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
    /// Every connected output as the OS describes it now.
    fn probe(&mut self) -> Vec<Description>;
    /// A vsync source for the output with this native handle, stopped,
    /// writing into `feed`.
    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error>;
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
}

impl std::fmt::Debug for Displays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Displays")
            .field("outputs", &self.outputs)
            .finish_non_exhaustive()
    }
}

impl Displays {
    /// The displays of the windowing system `display` is of (a window or an
    /// event loop of any toolkit). `wake` is called, on any thread, when
    /// [`Self::poll`] has something to do; it should have the owner's
    /// thread call it, and do nothing else.
    ///
    /// # Errors
    /// [`Error::Unsupported`] where this crate does not read the displays
    /// yet, [`Error::NotMainThread`] on macOS off the main thread, and
    /// [`Error::Os`] when the OS refuses the connection.
    pub fn connect(display: &impl HasDisplayHandle, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, Error> {
        let notice = Notice {
            dirty: Arc::new(AtomicBool::new(true)),
            wake: Arc::new(wake),
        };
        let handle = display.display_handle().map_err(|e| Error::Os(e.to_string()))?.as_raw();
        let backend = system(handle, &notice)?;
        Ok(Self::with(backend, notice))
    }

    fn with(backend: Box<dyn Backend>, notice: Notice) -> Self {
        let mut displays = Self {
            backend,
            outputs: Vec::new(),
            next: 1,
            notice,
            subscriptions: Vec::new(),
            not_send: PhantomData,
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
        (Self::with(Box::new(ScriptedBackend(state)), notice), script)
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
        if self.notice.dirty.swap(false, Ordering::SeqCst) {
            let probed = self.backend.probe();
            events = reconcile(&mut self.outputs, probed, &mut self.next);
            for e in &events {
                // A changed handle is another display link, another CRTC:
                // the old source ticks for nothing this output is on.
                let gone = match e {
                    Event::Removed(o) => Some(o.id),
                    Event::Changed { output, was } if output.native != was.native => Some(output.id),
                    _ => None,
                };
                if let Some(id) = gone {
                    for s in self.subscriptions.iter().filter(|s| s.link.output() == id) {
                        s.link.end();
                    }
                }
            }
        }
        let now = Instant::now();
        self.subscriptions.retain_mut(|s| match s.link.settle(now) {
            Settle::Keep => true,
            Settle::Start => {
                s.source.start();
                true
            }
            Settle::Stop => {
                s.source.stop();
                true
            }
            Settle::Close => {
                s.source.stop();
                false
            }
        });
        events
    }

    /// Subscribe to the vsync of `output`: its source is started now.
    ///
    /// # Errors
    /// [`Error::NoSuchOutput`], or [`Error::Unsupported`] where the OS has
    /// no vsync source this crate reads (X11), or [`Error::Os`].
    pub fn ticks(&mut self, output: OutputId) -> Result<Ticks, Error> {
        let described = self.output(output).ok_or(Error::NoSuchOutput)?.description.clone();
        let (ticks, link) = ticks::subscription(output, Arc::clone(&self.notice.wake));
        let mut source = self.backend.source(&described, link.feed())?;
        if link.settle(Instant::now()) == Settle::Start {
            source.start();
        }
        self.subscriptions.push(Subscription { link, source });
        Ok(ticks)
    }

    /// Ticks from a timer at the nominal refresh of `output`, each a
    /// [`Stamp::Synthesized`]: for a caller that wants one cadence whether
    /// the OS has a vsync source or not. They carry no phase.
    ///
    /// # Errors
    /// [`Error::NoSuchOutput`], or [`Error::Os`] when no thread can start.
    pub fn synthesized(&mut self, output: OutputId) -> Result<Ticks, Error> {
        let interval = self.output(output).ok_or(Error::NoSuchOutput)?.refresh.interval;
        let (ticks, link) = ticks::subscription(output, Arc::clone(&self.notice.wake));
        let mut source: Box<dyn Source> =
            Box::new(ticks::Timer::spawn(link.feed(), interval).map_err(|e| Error::Os(e.to_string()))?);
        if link.settle(Instant::now()) == Settle::Start {
            source.start();
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

/// The backend for the windowing system behind `handle`.
#[cfg(target_os = "macos")]
fn system(handle: raw_window_handle::RawDisplayHandle, notice: &Notice) -> Result<Box<dyn Backend>, Error> {
    match handle {
        raw_window_handle::RawDisplayHandle::AppKit(_) => Ok(Box::new(macos::MacOs::new(notice.clone())?)),
        _ => Err(Error::Unsupported("this windowing system")),
    }
}

#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
fn system(handle: raw_window_handle::RawDisplayHandle, notice: &Notice) -> Result<Box<dyn Backend>, Error> {
    use raw_window_handle::RawDisplayHandle;
    match handle {
        RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => Ok(Box::new(x11::X11::new(notice.clone())?)),
        RawDisplayHandle::Wayland(_) => Err(Error::Unsupported("Wayland")),
        _ => Err(Error::Unsupported("this windowing system")),
    }
}

#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "android")))))]
fn system(_: raw_window_handle::RawDisplayHandle, _: &Notice) -> Result<Box<dyn Backend>, Error> {
    Err(Error::Unsupported("this windowing system"))
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
}

struct ScriptedBackend(Arc<Mutex<Scripted>>);

struct ScriptedSource(Arc<AtomicBool>);

impl Source for ScriptedSource {
    fn start(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn stop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Backend for ScriptedBackend {
    fn probe(&mut self) -> Vec<Description> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).outputs.clone()
    }

    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error> {
        let running = Arc::new(AtomicBool::new(false));
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sources
            .push((output.native, feed, Arc::clone(&running)));
        Ok(Box::new(ScriptedSource(running)))
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

    fn edit(&self, f: impl FnOnce(&mut Vec<Description>)) {
        f(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner).outputs);
        self.notice.changed();
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
        for feed in feeds {
            feed.tick(target, interval);
        }
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
}
