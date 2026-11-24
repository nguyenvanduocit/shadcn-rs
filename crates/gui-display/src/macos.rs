//! macOS: AppKit's screens, Core Graphics' display modes and Core Video's
//! display link.
//!
//! - **Outputs** are `NSScreen.screens`, read on the main thread: frame,
//!   backing scale, refresh range (`minimumRefreshInterval` and its
//!   siblings), colour space (its ICC profile) and the three EDR headroom
//!   values. The current mode's rate is Core Graphics'; some built-in
//!   panels report 0 there, and the screen's `maximumFramesPerSecond`
//!   stands in.
//! - **Changes**: AppKit posts
//!   `NSApplicationDidChangeScreenParametersNotification` on the main
//!   thread for a plug, a pull, a mode or arrangement change, and for a
//!   change of EDR headroom. The observer sets the flag and wakes the
//!   owner; the outputs are read at its poll.
//! - **Vsync** is one `CVDisplayLink` per display for the whole process,
//!   shared by every reader of that display. Its callback runs on a Core
//!   Video thread and is given the host time at which the frame being made
//!   will be shown: a forecast, so every tick is [`Stamp::Predicted`]. The
//!   link is the OS's model of the display, not a vblank interrupt, and on
//!   a variable-refresh panel it reports the nominal cadence.
//!
//! A display link has no safe teardown: a stop returns before the link's
//! thread has left. Two things follow (GPUI crashed on both). A link is
//! never released, and is made once per display per process ([`LINKS`]),
//! not once per connection, so reconnecting leaks nothing more. And the
//! callback's context is not a pointer: Core Video hands back the `void *`
//! it was given without reading it, so it carries a number, looked up in
//! [`FEEDS`]. A callback that runs after a stop finds no feed and returns;
//! no lifetime depends on when Core Video stops calling.
//!
//! **The oldest macOS read is 11**; `connect` refuses an
//! older one. Every selector and function called here exists there, but
//! for the refresh range and `maximumFramesPerSecond` (macOS 12), read
//! behind `available!`. `tests/sites.rs` holds the table of what is called
//! and since when, and fails for a call it does not list or one newer than
//! the minimum that is not behind its check (NSScreen's own
//! `CGDirectDisplayID` is macOS 26: the id is read from the screen's
//! device description, as on every version).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2::{MainThreadMarker, available};
use objc2_app_kit::{
    NSApplicationDidChangeScreenParametersNotification, NSScreen, NSScreenColorSpaceDidChangeNotification,
};
use objc2_core_foundation::{CFRetained, CFUUID};
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayCopyDisplayMode, CGDisplayMode};
use objc2_core_video::{
    CVDisplayLink, CVGetCurrentHostTime, CVGetHostClockFrequency, CVOptionFlags, CVReturn, CVTimeStamp,
    CVTimeStampFlags,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName, NSNumber, ns_string};

use crate::ticks::{Asked, Feed, Source};
use crate::{
    Backend, Colour, Description, Error, Gamut, Geometry, Headroom, Notice, OutputKey, Refresh, RefreshRange, Stamp,
    gamut,
};

/// `kCVReturnSuccess`.
const CV_SUCCESS: CVReturn = 0;

pub(crate) struct MacOs {
    mtm: MainThreadMarker,
    /// The observers of the notifications that can change an output
    /// ([`changes`]), removed when this is dropped.
    observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    gamuts: Gamuts,
}

/// Every notification AppKit posts when something an [`crate::Output`]
/// holds may have changed:
///
/// - `NSApplicationDidChangeScreenParametersNotification`: a display
///   plugged, pulled, moved, its mode or scale changed, and (NSScreen.h) a
///   change of `maximumExtendedDynamicRangeColorComponentValue`;
/// - `NSScreenColorSpaceDidChangeNotification` (macOS 10.6): a screen's
///   colour profile changed, so its gamut.
///
/// The potential and reference EDR values and the refresh range have no
/// notification of their own; they move with a display's configuration,
/// which the first one covers (ADR-0029 says this is not verified).
fn changes() -> [&'static NSNotificationName; 2] {
    [screens_changed(), colour_space_changed()]
}

impl MacOs {
    pub(crate) fn new(notice: Notice) -> Result<Self, Error> {
        let mtm = MainThreadMarker::new().ok_or(Error::NotMainThread)?;
        // The oldest macOS read (ADR-0029; `tests/sites.rs` holds it too):
        // Rust's own floor for Apple silicon.
        if !available!(macos = 11.0) {
            return Err(Error::Unsupported("macOS before 11"));
        }
        let block = RcBlock::new(move |_: NonNull<NSNotification>| notice.changed());
        let centre = NSNotificationCenter::defaultCenter();
        let observers = changes()
            .into_iter()
            .map(|name| observe(&centre, name, &block))
            .collect();
        Ok(Self {
            mtm,
            observers,
            gamuts: Gamuts::default(),
        })
    }
}

impl Drop for MacOs {
    fn drop(&mut self) {
        let centre = NSNotificationCenter::defaultCenter();
        for observer in &self.observers {
            unobserve(&centre, observer);
        }
    }
}

/// The name of the notification AppKit posts when the screens change.
#[expect(unsafe_code, reason = "reads an extern static of AppKit (ADR-0029, site 1)")]
fn screens_changed() -> &'static NSNotificationName {
    // SAFETY: AppKit defines the constant and initializes it when the
    // framework loads, before any code of ours runs; it is never written.
    unsafe { NSApplicationDidChangeScreenParametersNotification }
}

/// The name of the notification AppKit posts when a screen's colour
/// profile changes.
#[expect(unsafe_code, reason = "reads an extern static of AppKit (ADR-0029, site 12)")]
fn colour_space_changed() -> &'static NSNotificationName {
    // SAFETY: AppKit defines the constant (macOS 10.6, below this crate's
    // minimum) and initializes it when the framework loads, before any code
    // of ours runs; it is never written.
    unsafe { NSScreenColorSpaceDidChangeNotification }
}

/// Each display's gamut, with a hash of the colour profile it was read
/// from: a probe at every notification reads the profile's bytes again,
/// and parses them only when they changed. One entry per display handle.
#[derive(Default)]
struct Gamuts {
    by_display: HashMap<CGDirectDisplayID, (u64, Gamut)>,
    /// How many profiles were parsed (tests).
    #[cfg(test)]
    parsed: usize,
}

impl Gamuts {
    fn of(&mut self, display: CGDirectDisplayID, icc: Option<&[u8]>) -> Gamut {
        use std::hash::{Hash, Hasher};
        let Some(icc) = icc else {
            self.by_display.remove(&display);
            return Gamut::Unknown;
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        icc.hash(&mut hasher);
        let hash = hasher.finish();
        if let Some((known, gamut)) = self.by_display.get(&display)
            && *known == hash
        {
            return *gamut;
        }
        let gamut = gamut::of_icc(icc);
        #[cfg(test)]
        {
            self.parsed = self.parsed.saturating_add(1);
        }
        self.by_display.insert(display, (hash, gamut));
        gamut
    }
}

/// Have `block` called for every notification named `name`, on the thread
/// that posts it.
#[expect(
    unsafe_code,
    reason = "registers a block with the notification centre (ADR-0029, site 2)"
)]
fn observe(
    centre: &NSNotificationCenter,
    name: &NSNotificationName,
    block: &block2::DynBlock<dyn Fn(NonNull<NSNotification>)>,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    // SAFETY: the block is `Send + Sync` in all it captures (an
    // `Arc<AtomicBool>` and an `Arc<dyn Fn() + Send + Sync>`), so it may run
    // on whatever thread posts; no object is filtered on and no queue
    // given, so neither has a type or a threading rule to keep. The centre
    // copies the block and keeps it until the observer is removed.
    unsafe { centre.addObserverForName_object_queue_usingBlock(Some(name), None, None, block) }
}

/// Remove an observer [`observe`] returned.
#[expect(
    unsafe_code,
    reason = "removes the observer from the notification centre (ADR-0029, site 3)"
)]
fn unobserve(centre: &NSNotificationCenter, observer: &ProtocolObject<dyn NSObjectProtocol>) {
    let observer: &AnyObject = observer.as_ref();
    // SAFETY: `observer` is the token `addObserverForName:…` returned from
    // this centre, the type `removeObserver:` takes for a block observer.
    unsafe { centre.removeObserver(observer) }
}

impl Backend for MacOs {
    fn probe(&mut self) -> Result<Vec<Description>, Error> {
        Ok(NSScreen::screens(self.mtm)
            .iter()
            .filter_map(|s| describe(&s, &mut self.gamuts))
            .collect())
    }

    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error> {
        let number = link_for(self.mtm, output.native)?;
        Ok(Box::new(Reader {
            mtm: self.mtm,
            display: output.native,
            number,
            id: READERS.fetch_add(1, Ordering::Relaxed),
            feed,
            on: false,
        }))
    }

    fn tidy(&mut self) {
        links(self.mtm, |l| l.values_mut().for_each(DisplayLink::settle));
    }
}

/// A float as the nearest `f32` (headroom values are small).
#[expect(clippy::as_conversions, reason = "f64 to f32, rounding to nearest as intended")]
fn narrow(v: f64) -> f32 {
    v as f32
}

/// A length in device pixels: rounded, and clamped into `u32` as `as` does.
#[expect(
    clippy::as_conversions,
    reason = "f64 to u32, saturating as intended for a pixel count"
)]
fn pixels(v: f64) -> u32 {
    v.round() as u32
}

/// A time the OS gave in seconds, when it is one a display can have.
fn seconds(v: f64) -> Option<Duration> {
    (v.is_finite() && v > 0.0 && v < 10.0).then(|| Duration::from_secs_f64(v))
}

/// The screen's display id. Read from its device description, as on every
/// macOS: the `CGDirectDisplayID` property of `NSScreen` is macOS 26's, an
/// unrecognized selector before it.
fn display_id(screen: &NSScreen) -> Option<CGDirectDisplayID> {
    let number = screen
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))?
        .downcast::<NSNumber>()
        .ok()?;
    Some(number.unsignedIntValue())
}

/// What a screen says of its refresh (macOS 12): its shortest and longest
/// frame time, their step, and its highest rate. None of it before 12.
struct Cadence {
    shortest: Option<Duration>,
    longest: Option<Duration>,
    granularity: Option<Duration>,
    fastest: Option<Duration>,
}

fn cadence(screen: &NSScreen) -> Cadence {
    if !available!(macos = 12.0) {
        return Cadence {
            shortest: None,
            longest: None,
            granularity: None,
            fastest: None,
        };
    }
    Cadence {
        shortest: seconds(screen.minimumRefreshInterval()),
        longest: seconds(screen.maximumRefreshInterval()),
        granularity: seconds(screen.displayUpdateGranularity()),
        fastest: i32::try_from(screen.maximumFramesPerSecond())
            .ok()
            .and_then(|fps| seconds(1.0 / f64::from(fps))),
    }
}

fn describe(screen: &NSScreen, gamuts: &mut Gamuts) -> Option<Description> {
    let native = display_id(screen).filter(|n| *n != 0)?;
    let frame = screen.frame();
    let scale = screen.backingScaleFactor();
    let geometry = Geometry {
        x: frame.origin.x,
        y: frame.origin.y,
        width: frame.size.width,
        height: frame.size.height,
        pixels: (pixels(frame.size.width * scale), pixels(frame.size.height * scale)),
    };
    let cadence = cadence(screen);
    let mode = CGDisplayCopyDisplayMode(native);
    let interval = seconds(1.0 / CGDisplayMode::refresh_rate(mode.as_deref()))
        .or(cadence.fastest)
        .or(cadence.shortest)
        .unwrap_or(Duration::from_nanos(16_666_667));
    // The two are equal on a display that does not vary.
    let range = match (cadence.shortest, cadence.longest) {
        (Some(shortest), Some(longest)) if longest > shortest.mul_f64(1.01) => Some(RefreshRange {
            shortest,
            longest,
            granularity: cadence.granularity,
        }),
        _ => None,
    };
    let icc = screen
        .colorSpace()
        .and_then(|c| c.ICCProfileData())
        .map(|icc| icc.to_vec());
    let gamut = gamuts.of(native, icc.as_deref());
    let reference = screen.maximumReferenceExtendedDynamicRangeColorComponentValue();
    let headroom = Headroom {
        current: narrow(screen.maximumExtendedDynamicRangeColorComponentValue().max(1.0)),
        potential: narrow(
            screen
                .maximumPotentialExtendedDynamicRangeColorComponentValue()
                .max(1.0),
        ),
        reference: (reference > 0.0).then(|| narrow(reference)),
    };
    Some(Description {
        key: key(native),
        native,
        name: screen.localizedName().to_string(),
        geometry,
        refresh: Refresh { interval, range },
        colour: Colour { gamut, headroom },
    })
}

#[link(name = "ColorSync", kind = "framework")]
#[expect(
    unsafe_code,
    reason = "declares one ColorSync function no binding crate in the tree has"
)]
unsafe extern "C" {
    /// `CFUUIDRef CGDisplayCreateUUIDFromDisplayID(uint32_t displayID)`
    /// (ColorSyncDevice.h): a new UUID object, or null for no such display.
    fn CGDisplayCreateUUIDFromDisplayID(display: u32) -> *mut CFUUID;
}

/// The display's UUID as ColorSync makes it, or null.
#[expect(unsafe_code, reason = "calls ColorSync for the display's UUID (ADR-0029, site 4)")]
fn uuid_raw(display: CGDirectDisplayID) -> *mut CFUUID {
    // SAFETY: the function takes any display id by value and returns null
    // for one it does not know; the declaration above is its C signature.
    unsafe { CGDisplayCreateUUIDFromDisplayID(display) }
}

/// Take ownership of a UUID a Create function returned.
#[expect(unsafe_code, reason = "adopts a +1 Core Foundation object (ADR-0029, site 5)")]
fn adopt(uuid: NonNull<CFUUID>) -> CFRetained<CFUUID> {
    // SAFETY: `uuid` comes from `CGDisplayCreateUUIDFromDisplayID`, which
    // follows the Create rule: a valid `CFUUID` with a retain count this
    // caller owns, released once when the `CFRetained` drops.
    unsafe { CFRetained::from_raw(uuid) }
}

/// What stays the same for a display across sessions: its UUID. A display
/// the OS gives none (not seen in practice) is keyed by its id.
fn key(display: CGDirectDisplayID) -> OutputKey {
    let Some(uuid) = NonNull::new(uuid_raw(display)).map(adopt) else {
        let mut bytes = [0u8; 16];
        for (b, d) in bytes.iter_mut().zip(display.to_be_bytes()) {
            *b = d;
        }
        return OutputKey(bytes);
    };
    let b = uuid.uuid_bytes();
    OutputKey([
        b.byte0, b.byte1, b.byte2, b.byte3, b.byte4, b.byte5, b.byte6, b.byte7, b.byte8, b.byte9, b.byte10, b.byte11,
        b.byte12, b.byte13, b.byte14, b.byte15,
    ])
}

/// The feeds of the readers that want ticks now, by the number their
/// link's callback is given, each with its reader's own number.
///
/// Lock order: the callback takes this lock on Core Video's thread, maybe
/// under locks of Core Video's own. So no Core Video function is ever
/// called while it is held, and a link is started and stopped only on the
/// main thread. Nor is any code of the caller's run under it: the owner's
/// wake is sent after it is released ([`Asked`]). `std`'s mutex on macOS
/// donates priority, so the owner holding it does not hold back the
/// display-priority thread for long.
static FEEDS: Mutex<Option<Feeds>> = Mutex::new(None);
/// The number the next link's callback is given.
static NEXT: AtomicUsize = AtomicUsize::new(1);
/// The number the next reader is known by among its link's.
static READERS: AtomicU64 = AtomicU64::new(1);
/// How many display links this process made ([`crate::native_links_made`]).
static MADE: AtomicUsize = AtomicUsize::new(0);

type Feeds = HashMap<usize, Vec<(u64, Feed)>>;

fn feeds<R>(f: impl FnOnce(&mut Feeds) -> R) -> R {
    f(FEEDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new))
}

/// Have the link numbered `number` tick into `feed` too, as reader `id`.
fn register(number: usize, id: u64, feed: Feed) {
    feeds(|f| f.entry(number).or_default().push((id, feed)));
}

/// Have it tick no more into reader `id`'s feed. A callback on its way
/// after this finds the feed gone.
fn unregister(number: usize, id: u64) {
    feeds(|f| {
        if let Some(feeds) = f.get_mut(&number) {
            feeds.retain(|(reader, _)| *reader != id);
        }
    });
}

pub(crate) fn links_made() -> usize {
    MADE.load(Ordering::Relaxed)
}

thread_local! {
    /// Every display link this process made, by its display: one per
    /// display for the life of the process, whatever number of
    /// [`crate::Displays`] come and go. A thread's own, read only on the
    /// main thread (each use takes its marker): there is one per process,
    /// and a `Displays` is made, used and dropped there alone. Never
    /// emptied; the links in it are never released.
    static LINKS: RefCell<HashMap<CGDirectDisplayID, DisplayLink>> = RefCell::new(HashMap::new());
}

/// Use the process's display links; on the main thread only.
fn links<R>(_: MainThreadMarker, f: impl FnOnce(&mut HashMap<CGDirectDisplayID, DisplayLink>) -> R) -> R {
    LINKS.with(|l| f(&mut l.borrow_mut()))
}

/// The number of `display`'s link, made now if it has none.
fn link_for(mtm: MainThreadMarker, display: CGDirectDisplayID) -> Result<usize, Error> {
    links(mtm, |l| {
        if let Some(link) = l.get(&display) {
            return Ok(link.number);
        }
        let link = DisplayLink::new(display)?;
        let number = link.number;
        l.insert(display, link);
        Ok(number)
    })
}

/// One display's link, made at the first subscription to it in the
/// process and never released.
///
/// `CVDisplayLinkStop` returns before the link's thread has left, and
/// nothing tells when it has: a link released after a stop can be read by
/// that thread once more (GPUI crashed so, in `CVHWTime::reset`). So the
/// link outlives every stop: it is kept for the display's handle for the
/// life of the process ([`LINKS`]), shared by its readers, and running
/// while one wants ticks. A handle the OS gives to another display later
/// would reuse it; GPUI reports that a link reads its display's timing as
/// it runs, which would make that right, but it is not verified here.
struct DisplayLink {
    link: ManuallyDrop<CFRetained<CVDisplayLink>>,
    number: usize,
    /// How many readers have it started.
    readers: usize,
    /// Whether it runs, as far as Core Video said.
    running: bool,
}

impl DisplayLink {
    fn new(display: CGDirectDisplayID) -> Result<Self, Error> {
        Self::install(made(display)?, set_callback)
    }

    /// Give `link` its callback with `set`, and only then make it the
    /// process's for good. A link whose callback could not be set was never
    /// given one and never started: nothing of Core Video's can call into it
    /// or run it, so it is released when it drops here.
    fn install(
        link: CFRetained<CVDisplayLink>,
        set: impl FnOnce(&CVDisplayLink, usize) -> CVReturn,
    ) -> Result<Self, Error> {
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let set = set(&link, number);
        if set != CV_SUCCESS {
            return Err(Error::Os(format!("CVDisplayLinkSetOutputCallback: {set}")));
        }
        MADE.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            link: ManuallyDrop::new(link),
            number,
            readers: 0,
            running: false,
        })
    }

    /// Start it for one more reader. On an error nothing is counted.
    #[expect(deprecated, reason = "CVDisplayLink, as at `create`")]
    fn start(&mut self, display: CGDirectDisplayID) -> Result<(), Error> {
        if !self.running {
            let started = self.link.start();
            if started != CV_SUCCESS {
                // What Core Video says runs is what is kept.
                self.running = self.link.is_running();
                return Err(Error::Os(format!("CVDisplayLinkStart({display}): {started}")));
            }
            self.running = true;
        }
        self.readers = self.readers.saturating_add(1);
        Ok(())
    }

    /// Stop it when nobody reads it and it runs. A stop Core Video refuses
    /// leaves what it says runs, and is tried again at the next poll
    /// ([`Backend::tidy`]).
    #[expect(deprecated, reason = "CVDisplayLink, as at `create`")]
    fn settle(&mut self) {
        if self.readers == 0 && self.running {
            self.running = self.link.stop() != CV_SUCCESS && self.link.is_running();
        }
    }
}

/// One subscription's use of its display's link.
struct Reader {
    mtm: MainThreadMarker,
    display: CGDirectDisplayID,
    /// Its link's number.
    number: usize,
    id: u64,
    feed: Feed,
    on: bool,
}

/// A new display link for `display`, owned, with no callback yet.
fn made(display: CGDirectDisplayID) -> Result<CFRetained<CVDisplayLink>, Error> {
    let mut out: *mut CVDisplayLink = std::ptr::null_mut();
    let made = create(display, NonNull::from(&mut out));
    NonNull::new(out)
        .filter(|_| made == CV_SUCCESS)
        .map(adopt_link)
        .ok_or_else(|| Error::Os(format!("CVDisplayLinkCreateWithCGDisplay({display}): {made}")))
}

/// Make a display link for one display; the link, +1, is written to `out`.
#[expect(
    unsafe_code,
    deprecated,
    reason = "creates the display link (ADR-0029, site 6; why not CADisplayLink is there)"
)]
fn create(display: CGDirectDisplayID, out: NonNull<*mut CVDisplayLink>) -> CVReturn {
    // SAFETY: `out` points at a live, writable `*mut CVDisplayLink` of the
    // caller's for the length of the call, which is all the function needs.
    unsafe { CVDisplayLink::create_with_cg_display(display, out) }
}

/// Take ownership of the link [`create`] wrote.
#[expect(unsafe_code, reason = "adopts a +1 Core Video object (ADR-0029, site 7)")]
fn adopt_link(link: NonNull<CVDisplayLink>) -> CFRetained<CVDisplayLink> {
    // SAFETY: `link` was written by `CVDisplayLinkCreateWithCGDisplay`
    // returning success: a valid link whose one retain this caller owns.
    unsafe { CFRetained::from_raw(link) }
}

/// Have the link call [`output`] with `number` as its context.
#[expect(
    unsafe_code,
    deprecated,
    reason = "sets the display link's callback (ADR-0029, site 8)"
)]
fn set_callback(link: &CVDisplayLink, number: usize) -> CVReturn {
    let context: *mut c_void = std::ptr::without_provenance_mut(number);
    // SAFETY: `output` has the callback's C signature and never unwinds
    // (it catches). The context is a number, not an address: Core Video
    // stores the `void *` and passes it back without reading it, and
    // `output` reads only its address, so nothing is ever dereferenced.
    unsafe { link.set_output_callback(Some(output), context) }
}

/// The time stamp Core Video lends a callback.
#[expect(
    unsafe_code,
    reason = "reads the time stamp the display link passes its callback (ADR-0029, site 9)"
)]
fn stamp(at: NonNull<CVTimeStamp>) -> CVTimeStamp {
    // SAFETY: Core Video passes a pointer to a `CVTimeStamp` that is valid
    // and not written for the length of the callback, which is the only
    // caller; the struct is plain data, copied out here.
    unsafe { at.read() }
}

/// The display link's callback, on a Core Video thread.
#[expect(
    clippy::mem_forget,
    reason = "a caught panic's payload is leaked, not dropped: its `Drop` could unwind into Core Video (site 8)"
)]
extern "C-unwind" fn output(
    _link: NonNull<CVDisplayLink>,
    _now: NonNull<CVTimeStamp>,
    output_time: NonNull<CVTimeStamp>,
    _flags_in: CVOptionFlags,
    _flags_out: NonNull<CVOptionFlags>,
    context: *mut c_void,
) -> CVReturn {
    let number = context.addr();
    // Nothing may unwind into Core Video: a panic's payload is forgotten,
    // not dropped, since its `Drop` could panic again outside the catch.
    if let Err(payload) = std::panic::catch_unwind(|| ticked(number, output_time)) {
        std::mem::forget(payload);
    }
    CV_SUCCESS
}

/// The vblank a callback's stamp tells of, and the refresh period it
/// reports; none for a stamp without a host time, which tells no vblank:
/// no tick, rather than the callback's own time passed off as the
/// display's.
fn vblank(time: &CVTimeStamp) -> Option<(Instant, Option<Duration>)> {
    if time.version != 0 || time.flags & CVTimeStampFlags::HostTimeValid.0 == 0 {
        return None;
    }
    let at = instant(
        Instant::now(),
        CVGetCurrentHostTime(),
        time.hostTime,
        CVGetHostClockFrequency(),
    )?;
    let interval = (time.flags & CVTimeStampFlags::VideoRefreshPeriodValid.0 != 0 && time.videoTimeScale > 0)
        .then(|| {
            let period = i32::try_from(time.videoRefreshPeriod).ok()?;
            seconds(f64::from(period) / f64::from(time.videoTimeScale))
        })
        .flatten();
    Some((at, interval))
}

fn ticked(number: usize, output_time: NonNull<CVTimeStamp>) {
    // Under the lock, so nothing is allocated at a vblank but when a source
    // asks to be stopped (once per idle): a feed's tick takes its own short
    // lock, and the wakes it owes are sent once this lock is released.
    let mut asked: Vec<Asked> = Vec::new();
    feeds(|f| {
        for (_, feed) in f.get(&number).into_iter().flatten() {
            // The mode first, then the time: a poll that moves the output
            // to another mode between the two makes this tick the old
            // mode's, which the feed drops.
            let (_, generation) = feed.mode();
            let Some((at, interval)) = vblank(&stamp(output_time)) else {
                return;
            };
            let a = feed.tick(Stamp::Predicted(at), interval, generation);
            if a.owed() {
                asked.push(a);
            }
        }
    });
    for a in asked {
        a.send();
    }
}

/// A count of host clock units as a float, exact to 2⁵³.
fn float(v: u64) -> f64 {
    let high = u32::try_from(v >> 32).unwrap_or(u32::MAX);
    let low = u32::try_from(v & 0xffff_ffff).unwrap_or(u32::MAX);
    f64::from(high) * 4_294_967_296.0 + f64::from(low)
}

/// The `Instant` of host time `host`, the host clock reading `host_now` at
/// `now` and counting `frequency` units a second. `Instant` and the host
/// clock are one clock on macOS, read in different units; the two "now"s
/// are taken a few nanoseconds apart, which is the error.
fn instant(now: Instant, host_now: u64, host: u64, frequency: f64) -> Option<Instant> {
    if !(frequency.is_finite() && frequency > 0.0) {
        return None;
    }
    let apart = float(host.abs_diff(host_now)) / frequency;
    // A stamp an hour from now is not a vblank.
    if apart.is_nan() || apart >= 3600.0 {
        return None;
    }
    let apart = Duration::from_secs_f64(apart);
    if host >= host_now {
        now.checked_add(apart)
    } else {
        now.checked_sub(apart)
    }
}

impl Source for Reader {
    fn start(&mut self) -> Result<(), Error> {
        if self.on {
            return Ok(());
        }
        // Core Video is called under no lock of ours; the feed is reachable
        // from the callback only once the link runs for it.
        links(self.mtm, |l| {
            l.get_mut(&self.display).ok_or(Error::NoSuchOutput)?.start(self.display)
        })?;
        register(self.number, self.id, self.feed.clone());
        self.on = true;
        Ok(())
    }

    fn stop(&mut self) {
        if !std::mem::replace(&mut self.on, false) {
            return;
        }
        // Unreachable from the callback first: one still on its way after
        // the stop below finds no feed of this reader's.
        unregister(self.number, self.id);
        links(self.mtm, |l| {
            if let Some(link) = l.get_mut(&self.display) {
                link.readers = link.readers.saturating_sub(1);
                link.settle();
            }
        });
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_time_becomes_the_instant_that_far_from_now() {
        let now = Instant::now();
        // 24 000 000 units a second, as on Apple silicon.
        let f = 24_000_000.0;
        assert_eq!(instant(now, 1_000_000, 1_000_000, f), Some(now));
        assert_eq!(
            instant(now, 1_000_000, 1_000_000 + 240_000, f),
            now.checked_add(Duration::from_millis(10))
        );
        assert_eq!(
            instant(now, 1_000_000, 1_000_000 - 240_000, f),
            now.checked_sub(Duration::from_millis(10))
        );
        assert_eq!(instant(now, 1_000_000, u64::MAX, f), None, "far from now is no vblank");
        assert_eq!(instant(now, 1_000_000, 1_000_000, 0.0), None);
        assert_eq!(instant(now, 1_000_000, 1_000_000, f64::NAN), None);
    }

    #[test]
    fn a_host_count_keeps_its_value_as_a_float() {
        assert_eq!(float(0), 0.0);
        assert_eq!(float(u64::from(u32::MAX) + 1), 4_294_967_296.0);
        assert_eq!(float(1 << 52), 4_503_599_627_370_496.0);
    }

    /// A stamp Core Video could give a callback now.
    fn stamp_now() -> CVTimeStamp {
        CVTimeStamp {
            version: 0,
            videoTimeScale: 0,
            videoTime: 0,
            hostTime: CVGetCurrentHostTime(),
            rateScalar: 1.0,
            videoRefreshPeriod: 0,
            smpteTime: objc2_core_video::CVSMPTETime {
                subframes: 0,
                subframeDivisor: 0,
                counter: 0,
                r#type: 0,
                flags: 0,
                hours: 0,
                minutes: 0,
                seconds: 0,
                frames: 0,
            },
            flags: CVTimeStampFlags::HostTimeValid.0,
            reserved: 0,
        }
    }

    #[test]
    fn a_link_whose_callback_could_not_be_set_is_released() {
        let Ok(link) = made(objc2_core_graphics::CGMainDisplayID()) else {
            eprintln!("no display: skipped");
            return;
        };
        let ours = link.clone();
        let before = ours.retain_count();
        let made_before = links_made();
        // Core Video refuses the callback (kCVReturnError).
        assert!(DisplayLink::install(link, |_, _| -6660).is_err());
        assert_eq!(ours.retain_count(), before - 1, "the failed link was released");
        assert_eq!(links_made(), made_before, "and is no link of the process's");
    }

    #[test]
    fn a_colour_profile_is_parsed_once_until_it_changes() {
        let mut gamuts = Gamuts::default();
        let a = vec![1_u8; 200];
        let b = vec![2_u8; 200];
        let first = gamuts.of(1, Some(&a));
        assert_eq!(gamuts.of(1, Some(&a)), first);
        assert_eq!(gamuts.parsed, 1, "the same profile is not parsed again");
        gamuts.of(1, Some(&b));
        assert_eq!(gamuts.parsed, 2, "another profile is");
        gamuts.of(2, Some(&b));
        assert_eq!(gamuts.parsed, 3, "and another display's");
        assert_eq!(gamuts.of(1, None), Gamut::Unknown);
    }

    #[test]
    fn every_notification_that_changes_an_output_is_observed() {
        let names: Vec<String> = changes().iter().map(|n| n.to_string()).collect();
        assert_eq!(
            names,
            [
                "NSApplicationDidChangeScreenParametersNotification",
                "NSScreenColorSpaceDidChangeNotification"
            ]
        );
    }

    #[test]
    fn a_callback_for_a_closed_link_finds_nothing() {
        // No link has this number: nothing is read, nothing panics.
        ticked(usize::MAX, NonNull::from(&stamp_now()));
    }

    #[test]
    fn a_wake_that_waits_for_the_owner_does_not_hold_the_callbacks_lock() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use std::sync::mpsc;

        use crate::ticks::{Settle, Wake, subscription};

        // A link of its own number; two readers of it. The first is idle,
        // so its next tick asks the owner to stop it; the owner's wake takes
        // a lock of the owner's, which the owner holds while it stops the
        // second reader.
        let number = usize::MAX - 1;
        let owners = Arc::new(Mutex::new(()));
        let entered = Arc::new(AtomicBool::new(false));
        let wake: Wake = {
            let (owners, entered) = (Arc::clone(&owners), Arc::clone(&entered));
            // Only the first call (the idle reader's ask) waits; the wakes
            // the readers send when dropped do not.
            Arc::new(move || {
                if !entered.swap(true, Ordering::SeqCst) {
                    drop(owners.lock());
                }
            })
        };
        let hz60 = Duration::from_nanos(16_666_667);
        let (_idle, idle) = subscription(crate::OutputId(1), hz60, Arc::clone(&wake));
        let (_busy, busy) = subscription(crate::OutputId(2), hz60, wake);
        assert_eq!(idle.settle(Instant::now()), Settle::Start);
        assert_eq!(busy.settle(Instant::now()), Settle::Start);
        idle.unwanted_since(Instant::now().checked_sub(crate::IDLE * 2).unwrap());
        register(number, 1, idle.feed());
        register(number, 2, busy.feed());

        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            let held = owners.lock();
            // Core Video ticks, on its own thread, while the owner holds its lock.
            let video = std::thread::spawn(move || ticked(number, NonNull::from(&stamp_now())));
            let by = Instant::now() + Duration::from_secs(5);
            while !entered.load(Ordering::SeqCst) && Instant::now() < by {
                std::thread::sleep(Duration::from_millis(1));
            }
            // The owner stops the other reader, its lock held.
            unregister(number, 2);
            drop(held);
            video.join().ok();
            done.send(()).ok();
        });
        assert!(
            finished.recv_timeout(Duration::from_secs(5)).is_ok(),
            "the callback held its lock while the wake waited for the owner's"
        );
        feeds(|f| f.remove(&number));
    }

    #[test]
    fn off_the_main_thread_the_displays_are_refused() {
        // Tests run on threads of their own.
        let notice = Notice {
            dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            wake: std::sync::Arc::new(|| {}),
        };
        assert!(matches!(MacOs::new(notice), Err(Error::NotMainThread)));
    }
}
