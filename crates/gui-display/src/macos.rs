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
//! - **Vsync** is a `CVDisplayLink` per subscription. Its callback runs on
//!   a Core Video thread and is given the host time at which the frame
//!   being made will be shown: a forecast, so every tick is
//!   [`Stamp::Predicted`]. The link is the OS's model of the display, not a
//!   vblank interrupt, and on a variable-refresh panel it reports the
//!   nominal cadence.
//!
//! A display link has no safe teardown: a stop returns before the link's
//! thread has left. Two things follow (GPUI crashed on both). A link is
//! never released ([`DisplayLink`]). And the callback's context is not a
//! pointer: Core Video hands back the `void *` it was given without
//! reading it, so it carries a number, looked up in [`FEEDS`]. A callback
//! that runs after a stop finds no feed and returns; no lifetime depends on
//! when Core Video stops calling.

use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, ProtocolObject};
use objc2_app_kit::{NSApplicationDidChangeScreenParametersNotification, NSScreen};
use objc2_core_foundation::{CFRetained, CFUUID};
use objc2_core_graphics::{CGDirectDisplayID, CGDisplayCopyDisplayMode, CGDisplayMode};
use objc2_core_video::{
    CVDisplayLink, CVGetCurrentHostTime, CVGetHostClockFrequency, CVOptionFlags, CVReturn, CVTimeStamp,
    CVTimeStampFlags,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};

use crate::ticks::{Feed, Source};
use crate::{
    Backend, Colour, Description, Error, Gamut, Geometry, Headroom, Notice, OutputKey, Refresh, RefreshRange, Stamp,
    gamut,
};

/// `kCVReturnSuccess`.
const CV_SUCCESS: CVReturn = 0;

pub(crate) struct MacOs {
    mtm: MainThreadMarker,
    /// The observer of screen changes, removed when this is dropped.
    observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    /// Each display's link by its handle, made at its first subscription.
    links: HashMap<CGDirectDisplayID, Rc<DisplayLink>>,
}

impl MacOs {
    pub(crate) fn new(notice: Notice) -> Result<Self, Error> {
        let mtm = MainThreadMarker::new().ok_or(Error::NotMainThread)?;
        let block = RcBlock::new(move |_: NonNull<NSNotification>| notice.changed());
        let observer = observe(&NSNotificationCenter::defaultCenter(), screens_changed(), &block);
        Ok(Self {
            mtm,
            observer,
            links: HashMap::new(),
        })
    }
}

impl Drop for MacOs {
    fn drop(&mut self) {
        unobserve(&NSNotificationCenter::defaultCenter(), &self.observer);
    }
}

/// The name of the notification AppKit posts when the screens change.
#[expect(unsafe_code, reason = "reads an extern static of AppKit (ADR-0029, site 1)")]
fn screens_changed() -> &'static NSNotificationName {
    // SAFETY: AppKit defines the constant and initializes it when the
    // framework loads, before any code of ours runs; it is never written.
    unsafe { NSApplicationDidChangeScreenParametersNotification }
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
    fn probe(&mut self) -> Vec<Description> {
        NSScreen::screens(self.mtm)
            .iter()
            .filter_map(|s| describe(&s))
            .collect()
    }

    fn source(&mut self, output: &Description, feed: Feed) -> Result<Box<dyn Source>, Error> {
        let link = match self.links.get(&output.native) {
            Some(link) => Rc::clone(link),
            None => {
                let link = Rc::new(DisplayLink::new(output.native)?);
                self.links.insert(output.native, Rc::clone(&link));
                link
            }
        };
        let id = link.readers.get().saturating_add(1);
        link.readers.set(id);
        Ok(Box::new(Reader {
            link,
            id,
            feed,
            on: false,
        }))
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

fn describe(screen: &NSScreen) -> Option<Description> {
    let native: CGDirectDisplayID = screen.CGDirectDisplayID();
    if native == 0 {
        return None;
    }
    let frame = screen.frame();
    let scale = screen.backingScaleFactor();
    let geometry = Geometry {
        x: frame.origin.x,
        y: frame.origin.y,
        width: frame.size.width,
        height: frame.size.height,
        pixels: (pixels(frame.size.width * scale), pixels(frame.size.height * scale)),
    };
    let shortest = seconds(screen.minimumRefreshInterval());
    let longest = seconds(screen.maximumRefreshInterval());
    let mode = CGDisplayCopyDisplayMode(native);
    let interval = seconds(1.0 / CGDisplayMode::refresh_rate(mode.as_deref()))
        .or_else(|| {
            let fps = i32::try_from(screen.maximumFramesPerSecond()).ok()?;
            seconds(1.0 / f64::from(fps))
        })
        .or(shortest)
        .unwrap_or(Duration::from_nanos(16_666_667));
    // The two are equal on a display that does not vary.
    let range = match (shortest, longest) {
        (Some(shortest), Some(longest)) if longest > shortest.mul_f64(1.01) => Some(RefreshRange {
            shortest,
            longest,
            granularity: seconds(screen.displayUpdateGranularity()),
        }),
        _ => None,
    };
    let gamut = screen
        .colorSpace()
        .and_then(|c| c.ICCProfileData())
        .map_or(Gamut::Unknown, |icc| gamut::of_icc(&icc.to_vec()));
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
/// owner's thread. `std`'s mutex on macOS donates priority, so the owner
/// holding it does not hold back the display-priority thread for long.
static FEEDS: Mutex<Option<Feeds>> = Mutex::new(None);
static NEXT: AtomicUsize = AtomicUsize::new(1);

type Feeds = HashMap<usize, Vec<(u64, Feed)>>;

fn feeds<R>(f: impl FnOnce(&mut Feeds) -> R) -> R {
    f(FEEDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new))
}

/// One display's link, made at the first subscription to it and never
/// released.
///
/// `CVDisplayLinkStop` returns before the link's thread has left, and
/// nothing tells when it has: a link released after a stop can be read by
/// that thread once more (GPUI crashed so, in `CVHWTime::reset`). So the
/// link outlives every stop: it is kept for the display's handle while the
/// displays live, shared by its readers, running while one wants ticks,
/// and leaked with them. A handle the OS gives to another display later
/// reuses it; a link reads its display's timing as it runs.
struct DisplayLink {
    link: ManuallyDrop<CFRetained<CVDisplayLink>>,
    number: usize,
    /// How many readers have it started.
    started: Cell<usize>,
    readers: Cell<u64>,
}

impl DisplayLink {
    fn new(display: CGDirectDisplayID) -> Result<Self, Error> {
        let mut out: *mut CVDisplayLink = std::ptr::null_mut();
        let made = create(display, NonNull::from(&mut out));
        let link = NonNull::new(out)
            .filter(|_| made == CV_SUCCESS)
            .map(adopt_link)
            .ok_or_else(|| Error::Os(format!("CVDisplayLinkCreateWithCGDisplay({display}): {made}")))?;
        let link = Self {
            link: ManuallyDrop::new(link),
            number: NEXT.fetch_add(1, Ordering::Relaxed),
            started: Cell::new(0),
            readers: Cell::new(0),
        };
        let set = set_callback(&link.link, link.number);
        if set != CV_SUCCESS {
            return Err(Error::Os(format!("CVDisplayLinkSetOutputCallback: {set}")));
        }
        Ok(link)
    }
}

/// One subscription's use of its display's link.
struct Reader {
    link: Rc<DisplayLink>,
    id: u64,
    feed: Feed,
    on: bool,
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
extern "C-unwind" fn output(
    _link: NonNull<CVDisplayLink>,
    _now: NonNull<CVTimeStamp>,
    output_time: NonNull<CVTimeStamp>,
    _flags_in: CVOptionFlags,
    _flags_out: NonNull<CVOptionFlags>,
    context: *mut c_void,
) -> CVReturn {
    let number = context.addr();
    let time = stamp(output_time);
    // Nothing may unwind into Core Video.
    std::panic::catch_unwind(|| ticked(number, &time)).ok();
    CV_SUCCESS
}

fn ticked(number: usize, time: &CVTimeStamp) {
    // A stamp without a host time tells no vblank: no tick, rather than
    // the callback's own time passed off as the display's.
    if time.version != 0 || time.flags & CVTimeStampFlags::HostTimeValid.0 == 0 {
        return;
    }
    let Some(at) = instant(
        Instant::now(),
        CVGetCurrentHostTime(),
        time.hostTime,
        CVGetHostClockFrequency(),
    ) else {
        return;
    };
    let interval = (time.flags & CVTimeStampFlags::VideoRefreshPeriodValid.0 != 0 && time.videoTimeScale > 0)
        .then(|| {
            let period = i32::try_from(time.videoRefreshPeriod).ok()?;
            seconds(f64::from(period) / f64::from(time.videoTimeScale))
        })
        .flatten();
    // Under the lock, so nothing is allocated at a vblank; a feed's tick
    // takes its own short lock and may send the owner's wake, neither of
    // which takes this one.
    feeds(|f| {
        for (_, feed) in f.get(&number).into_iter().flatten() {
            feed.tick(Stamp::Predicted(at), interval);
        }
    });
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
    #[expect(deprecated, reason = "CVDisplayLink, as at `create`")]
    fn start(&mut self) {
        if std::mem::replace(&mut self.on, true) {
            return;
        }
        feeds(|f| {
            f.entry(self.link.number)
                .or_default()
                .push((self.id, self.feed.clone()))
        });
        let before = self.link.started.get();
        self.link.started.set(before.saturating_add(1));
        // The lock is released: Core Video is called under none of ours.
        if before == 0 {
            self.link.link.start();
        }
    }

    #[expect(deprecated, reason = "CVDisplayLink, as at `create`")]
    fn stop(&mut self) {
        if !std::mem::replace(&mut self.on, false) {
            return;
        }
        // Unreachable from the callback first: one still on its way after
        // the stop below finds no feed of this reader's.
        feeds(|f| {
            if let Some(feeds) = f.get_mut(&self.link.number) {
                feeds.retain(|(id, _)| *id != self.id);
            }
        });
        let left = self.link.started.get().saturating_sub(1);
        self.link.started.set(left);
        if left == 0 {
            self.link.link.stop();
        }
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

    #[test]
    fn a_callback_for_a_closed_link_finds_nothing() {
        let time = CVTimeStamp {
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
        };
        // No link has this number: nothing is read, nothing panics.
        ticked(usize::MAX, &time);
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
