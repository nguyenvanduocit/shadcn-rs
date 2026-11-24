//! X11: RandR through the toolkit's own connection, on the window's screen.
//!
//! - **The connection** is the one the toolkit handed over: Xlib's
//!   `Display` (its XCB connection, which Xlib makes at open) or XCB's,
//!   adopted and never closed here. So the outputs read are those of the
//!   server the window is on, whatever `$DISPLAY` says. Two sites
//!   ([`xcb_of`], [`adopt`]). The connection stays open while the window
//!   lives, and [`crate::Displays`] keeps the window until after this
//!   backend is dropped. A handle with no connection in it (raw-window-
//!   handle's "the default display") gets one of our own.
//! - **The screen** is the root of the window, where a window handle was
//!   given and its root is one of the server's; else the screen the
//!   display handle names ([`root_of`]), also on a connection of our own
//!   (chosen apart from the connection, [`target`]).
//! - **RandR 1.3** is required (`GetScreenResourcesCurrent`, which reads
//!   the server's state without probing the hardware); `connect` returns
//!   `Unsupported` below it.
//! - **Outputs** are the connected RandR outputs that drive a CRTC, every
//!   one of them, mirrored outputs on one CRTC included. The handle is the
//!   CRTC, as winit reports a monitor. The interval is the mode's pixel
//!   clock over its whole frame; identity, name, primaries and the
//!   vertical range come from the EDID property.
//! - **A read that fails** (a request the server refused, or the
//!   configuration changing under it) fails the whole probe: the displays
//!   keep what they last read whole and read again at the next poll, so a
//!   failed request never reads as outputs pulled out.
//! - **Variable refresh.** X11 tells no range. An output whose driver sets
//!   `vrr_capable` varies between the rates its EDID's range limits give.
//! - **Colour.** The gamut is the EDID's primaries; X11 has no HDR, so the
//!   headroom is none.
//! - **Changes.** The toolkit owns its connection's event queue, so
//!   RandR's notifications are read on a connection of our own (the
//!   [`Doorbell`]), to `$DISPLAY`, kept only when its server is the
//!   toolkit's ([`same_server`]); a thread waits on it, sets the flag and
//!   wakes the owner. It reads nothing else: every output is read through
//!   the toolkit's connection. Where `$DISPLAY` is another server, there is
//!   no doorbell; the backend then says it does not notify, and every poll
//!   reads the outputs afresh, so a change is seen at the next poll the
//!   window's own events ask for.
//! - **Vsync**: none. The Present extension reports a window's presents,
//!   not an output's vblanks (ADR-0029 leaves it open with Wayland's
//!   presentation feedback). Callers that want a cadence take
//!   [`crate::Displays::synthesized`].

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::time::Duration;

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use x11_dl::xlib_xcb::Xlib_xcb;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{self, ConnectionExt as _, ModeFlag, NotifyMask};
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, Setup, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::xcb_ffi::XCBConnection;

use crate::ticks::{Feed, Source};
use crate::{
    Backend, Colour, Description, Error, Gamut, Geometry, Headroom, Notice, OutputKey, Refresh, RefreshRange, edid,
    gamut,
};

fn os(e: impl std::fmt::Display) -> Error {
    Error::Os(e.to_string())
}

pub(crate) struct X11 {
    /// The toolkit's connection (or, given none, our own): every request
    /// goes through it.
    conn: XCBConnection,
    root: Window,
    edid: u32,
    vrr_capable: u32,
    /// Where RandR's notifications come from, if `$DISPLAY` is the
    /// toolkit's server.
    doorbell: Option<Doorbell>,
}

impl X11 {
    pub(crate) fn new(
        display: RawDisplayHandle,
        surface: Option<RawWindowHandle>,
        notice: Notice,
    ) -> Result<Self, Error> {
        let (conn, screen) = connection(display)?;
        let window = surface.and_then(|s| match s {
            RawWindowHandle::Xlib(w) => u32::try_from(w.window).ok(),
            RawWindowHandle::Xcb(w) => Some(w.window.get()),
            _ => None,
        });
        let window_root = match window {
            Some(w) => Some(conn.get_geometry(w).map_err(os)?.reply().map_err(os)?.root),
            None => None,
        };
        let roots: Vec<Window> = conn.setup().roots.iter().map(|s| s.root).collect();
        let root = root_of(&roots, window_root, screen).ok_or_else(|| os("no such screen"))?;
        let version = conn.randr_query_version(1, 5).map_err(os)?.reply().map_err(os)?;
        if !supported(version.major_version, version.minor_version) {
            return Err(Error::Unsupported("an X server without RandR 1.3"));
        }
        let atom = |name: &[u8]| -> Result<u32, Error> {
            Ok(conn.intern_atom(false, name).map_err(os)?.reply().map_err(os)?.atom)
        };
        let (edid, vrr_capable) = (atom(b"EDID")?, atom(b"vrr_capable")?);
        let doorbell = Doorbell::ring(conn.setup(), root, notice);
        Ok(Self {
            conn,
            root,
            edid,
            vrr_capable,
            doorbell,
        })
    }

    /// The bytes of an output's property: none when it has none, an error
    /// when the server refused the request.
    fn property(&self, output: randr::Output, atom: u32) -> Result<Option<Vec<u8>>, Error> {
        let reply = self
            .conn
            .randr_get_output_property(output, atom, AtomEnum::ANY, 0, 256, false, false)
            .map_err(os)?
            .reply()
            .map_err(os)?;
        Ok((!reply.data.is_empty()).then_some(reply.data))
    }
}

/// The toolkit's connection, adopted, and the screen its handle names; or,
/// for a handle with none, a connection of our own and its screen.
fn connection(display: RawDisplayHandle) -> Result<(XCBConnection, i32), Error> {
    let (theirs, screen) = target(display)?;
    let conn = match theirs {
        Theirs::Xlib(d) => adopt(xcb_of_xlib(d)?)?,
        Theirs::Xcb(c) => adopt(c)?,
        Theirs::None => XCBConnection::connect(None).map_err(os)?.0,
    };
    Ok((conn, screen))
}

/// The connection a display handle gives.
#[derive(Debug, PartialEq, Eq)]
enum Theirs {
    Xlib(NonNull<c_void>),
    Xcb(NonNull<c_void>),
    /// None: the default display, on a connection of our own.
    None,
}

/// What a display handle names: the toolkit's connection, and its screen.
/// The two are chosen apart: a handle with no connection still names its
/// screen, which is kept, not the default screen of the connection made
/// for it.
fn target(display: RawDisplayHandle) -> Result<(Theirs, i32), Error> {
    match display {
        RawDisplayHandle::Xlib(h) => Ok((h.display.map_or(Theirs::None, Theirs::Xlib), h.screen)),
        RawDisplayHandle::Xcb(h) => Ok((h.connection.map_or(Theirs::None, Theirs::Xcb), h.screen)),
        _ => Err(Error::Unsupported("this windowing system")),
    }
}

/// The XCB connection under an Xlib display.
fn xcb_of_xlib(display: NonNull<c_void>) -> Result<NonNull<c_void>, Error> {
    let lib = Xlib_xcb::open().map_err(os)?;
    NonNull::new(xcb_of(&lib, display)).ok_or_else(|| os("XGetXCBConnection: no connection"))
}

/// Ask Xlib for the XCB connection it made for `display`.
#[expect(
    unsafe_code,
    reason = "asks Xlib for the XCB connection of the toolkit's display (ADR-0029, site 10)"
)]
fn xcb_of(lib: &Xlib_xcb, display: NonNull<c_void>) -> *mut c_void {
    // SAFETY: `display` is the toolkit's `Display *` from its display
    // handle, open while the window that gave it lives (raw-window-handle's
    // contract), which `Displays` keeps for as long as it uses it. The
    // function reads the pointer it is given and returns the connection
    // Xlib made at open; its signature is x11-dl's, from Xlib-xcb.h.
    unsafe { (lib.XGetXCBConnection)(display.as_ptr().cast()) }
}

/// Use the toolkit's XCB connection, without ever closing it.
#[expect(
    unsafe_code,
    reason = "adopts the toolkit's XCB connection, not to be closed (ADR-0029, site 11)"
)]
fn adopt(connection: NonNull<c_void>) -> Result<XCBConnection, Error> {
    // SAFETY: `connection` is a live `xcb_connection_t *`: the toolkit's,
    // from its display handle or from Xlib's display, open while the window
    // lives, which `Displays` keeps until after this backend (the
    // connection's only user here) is dropped. `false`: it is not
    // disconnected when dropped. XCB is thread-safe, so requests made here
    // alongside the toolkit's are allowed; no event is read from it.
    unsafe { XCBConnection::from_raw_xcb_connection(connection.as_ptr(), false) }.map_err(os)
}

/// The root the outputs are read on: the window's, when it is one of the
/// server's screens; else the root of the screen the display handle names.
fn root_of(roots: &[Window], window_root: Option<Window>, screen: i32) -> Option<Window> {
    window_root
        .filter(|r| roots.contains(r))
        .or_else(|| roots.get(usize::try_from(screen).ok()?).copied())
}

/// Whether a server's RandR is one this backend reads: 1.3, for
/// `GetScreenResourcesCurrent`.
fn supported(major: u32, minor: u32) -> bool {
    (major, minor) >= (1, 3)
}

/// Whether two connections' setups are of one server: the same vendor and
/// release, and the same screens.
fn same_server(a: &Setup, b: &Setup) -> bool {
    let screens = |s: &Setup| -> Vec<(Window, u16, u16, u16, u16, u32)> {
        s.roots
            .iter()
            .map(|r| {
                (
                    r.root,
                    r.width_in_pixels,
                    r.height_in_pixels,
                    r.width_in_millimeters,
                    r.height_in_millimeters,
                    r.root_visual,
                )
            })
            .collect()
    };
    a.vendor == b.vendor && a.release_number == b.release_number && screens(a) == screens(b)
}

/// An output as the server's replies describe it, or none for one that
/// shows nothing (no mode on its CRTC).
fn describe(
    info: &randr::GetOutputInfoReply,
    crtc: &randr::GetCrtcInfoReply,
    modes: &[randr::ModeInfo],
    edid: Option<&[u8]>,
    vrr_capable: Option<&[u8]>,
) -> Option<Description> {
    let mode = modes.iter().find(|m| m.id == crtc.mode)?;
    let interval = edid::refresh_interval(
        mode.dot_clock,
        mode.htotal,
        mode.vtotal,
        mode.mode_flags.contains(ModeFlag::INTERLACE),
        mode.mode_flags.contains(ModeFlag::DOUBLE_SCAN),
    )
    .unwrap_or(Duration::from_nanos(16_666_667));
    let connector = String::from_utf8_lossy(&info.name).into_owned();
    let edid = edid.and_then(edid::parse);
    // Any byte set: drivers store the flag in 8 or 32 bits.
    let varies = vrr_capable.is_some_and(|b| b.iter().any(|b| *b != 0));
    let range = edid
        .as_ref()
        .and_then(|e| e.vertical)
        .filter(|(lowest, highest)| varies && highest > lowest)
        .map(|(lowest, highest)| RefreshRange {
            shortest: Duration::from_secs_f64(1.0 / f64::from(highest)),
            longest: Duration::from_secs_f64(1.0 / f64::from(lowest)),
            granularity: None,
        });
    Some(Description {
        key: key(edid.as_ref().map(|e| e.identity), &connector),
        native: info.crtc,
        name: edid
            .as_ref()
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| connector.clone()),
        geometry: Geometry {
            x: f64::from(crtc.x),
            y: f64::from(crtc.y),
            width: f64::from(crtc.width),
            height: f64::from(crtc.height),
            pixels: (u32::from(crtc.width), u32::from(crtc.height)),
        },
        refresh: Refresh { interval, range },
        colour: Colour {
            gamut: edid.as_ref().map_or(Gamut::Unknown, |e| gamut::of_native(&e.primaries)),
            headroom: Headroom::SDR,
        },
    })
}

/// The EDID's identity, then the connector's name: two monitors of one
/// model without serial numbers differ by where they are plugged in.
fn key(identity: Option<[u8; 8]>, connector: &str) -> OutputKey {
    let mut bytes = [0u8; 16];
    let (id, name) = bytes.split_at_mut(8);
    if let Some(identity) = identity {
        id.copy_from_slice(&identity);
    }
    for (b, c) in name.iter_mut().zip(connector.bytes().rev()) {
        *b = c;
    }
    OutputKey(bytes)
}

/// A connection of our own that only hears RandR's notifications, and the
/// window nobody sees which its watcher is told to stop through.
struct Doorbell {
    conn: Arc<RustConnection>,
    door: Window,
}

impl Doorbell {
    /// Listen for RandR's notifications on `root`, when `$DISPLAY` is the
    /// server `theirs` is the setup of; none when it is another or cannot
    /// be reached.
    fn ring(theirs: &Setup, root: Window, notice: Notice) -> Option<Self> {
        let (conn, _) = x11rb::connect(None).ok()?;
        if !same_server(theirs, conn.setup()) {
            return None;
        }
        let conn = Arc::new(conn);
        let door = conn.generate_id().ok()?;
        conn.create_window(
            0,
            door,
            root,
            0,
            0,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new(),
        )
        .ok()?;
        let mask = NotifyMask::SCREEN_CHANGE
            | NotifyMask::CRTC_CHANGE
            | NotifyMask::OUTPUT_CHANGE
            | NotifyMask::OUTPUT_PROPERTY;
        conn.randr_select_input(root, mask).ok()?;
        conn.flush().ok()?;
        let theirs = Arc::clone(&conn);
        std::thread::Builder::new()
            .name("gui-display x11".to_owned())
            .spawn(move || watch(&theirs, door, &notice))
            .ok()?;
        Some(Self { conn, door })
    }
}

impl Drop for Doorbell {
    fn drop(&mut self) {
        // The watcher leaves when this reaches it; if it cannot be sent the
        // connection is gone and the watcher has left already.
        let stop = ClientMessageEvent::new(32, self.door, AtomEnum::NOTICE, [0u32; 5]);
        if self
            .conn
            .send_event(false, self.door, EventMask::NO_EVENT, stop)
            .is_ok()
        {
            self.conn.flush().ok();
        }
    }
}

/// Wait for RandR's notifications until told to stop through the door, or
/// the connection ends.
fn watch(conn: &RustConnection, door: Window, notice: &Notice) {
    loop {
        match conn.wait_for_event() {
            Ok(Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_)) => notice.changed(),
            Ok(Event::ClientMessage(m)) if m.window == door => return,
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

impl Backend for X11 {
    fn probe(&mut self) -> Result<Vec<Description>, Error> {
        let resources = self
            .conn
            .randr_get_screen_resources_current(self.root)
            .map_err(os)?
            .reply()
            .map_err(os)?;
        let at = resources.config_timestamp;
        let mut crtcs: HashMap<randr::Crtc, randr::GetCrtcInfoReply> = HashMap::new();
        let mut outputs = Vec::new();
        for &output in &resources.outputs {
            let info = self
                .conn
                .randr_get_output_info(output, at)
                .map_err(os)?
                .reply()
                .map_err(os)?;
            if info.status != randr::SetConfig::SUCCESS {
                return Err(os("the configuration changed while it was read"));
            }
            if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
                continue;
            }
            // Mirrored outputs share a CRTC: read once, each described.
            let crtc = match crtcs.entry(info.crtc) {
                Entry::Occupied(e) => e.into_mut(),
                Entry::Vacant(e) => {
                    let crtc = self
                        .conn
                        .randr_get_crtc_info(info.crtc, at)
                        .map_err(os)?
                        .reply()
                        .map_err(os)?;
                    if crtc.status != randr::SetConfig::SUCCESS {
                        return Err(os("the configuration changed while it was read"));
                    }
                    e.insert(crtc)
                }
            };
            let edid = self.property(output, self.edid)?;
            let vrr = self.property(output, self.vrr_capable)?;
            outputs.extend(describe(&info, crtc, &resources.modes, edid.as_deref(), vrr.as_deref()));
        }
        Ok(outputs)
    }

    fn source(&mut self, _: &Description, _: Feed) -> Result<Box<dyn Source>, Error> {
        Err(Error::Unsupported("X11 (no vsync source for an output)"))
    }

    /// With no doorbell nothing tells of a change: every poll reads.
    fn notifies(&self) -> bool {
        self.doorbell.is_some()
    }
}

#[cfg(test)]
mod tests {
    use x11rb::protocol::xproto::Screen;

    use super::*;

    #[test]
    fn a_key_holds_the_edid_identity_and_the_connector() {
        let a = key(Some([1, 2, 3, 4, 5, 6, 7, 8]), "DP-1");
        let b = key(Some([1, 2, 3, 4, 5, 6, 7, 8]), "DP-2");
        assert_eq!(a.0[..8], [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_ne!(a, b, "twins differ by connector");
        assert_eq!(key(None, "DP-1"), key(None, "DP-1"));
        assert_ne!(
            key(None, "HDMI-A-1"),
            key(None, "HDMI-A-2"),
            "the end of the name tells them apart"
        );
    }

    #[test]
    fn the_screen_read_is_the_windows_then_the_handles() {
        let roots = [0x100, 0x200];
        assert_eq!(root_of(&roots, Some(0x200), 0), Some(0x200), "the window's root first");
        assert_eq!(root_of(&roots, None, 1), Some(0x200), "else the handle's screen");
        assert_eq!(
            root_of(&roots, Some(0x999), 0),
            Some(0x100),
            "a root not the server's is not taken"
        );
        assert_eq!(root_of(&roots, None, 2), None);
        assert_eq!(root_of(&roots, None, -1), None);
    }

    #[test]
    fn a_handle_with_no_connection_keeps_its_screen() {
        use raw_window_handle::{XcbDisplayHandle, XlibDisplayHandle};
        let (theirs, screen) = target(XlibDisplayHandle::new(None, 1).into()).unwrap();
        assert_eq!(
            (theirs, screen),
            (Theirs::None, 1),
            "the handle's screen, not the default"
        );
        let (theirs, screen) = target(XcbDisplayHandle::new(None, 2).into()).unwrap();
        assert_eq!((theirs, screen), (Theirs::None, 2));
        let c = NonNull::<u8>::dangling().cast::<c_void>();
        let (theirs, screen) = target(XcbDisplayHandle::new(Some(c), 0).into()).unwrap();
        assert_eq!((theirs, screen), (Theirs::Xcb(c), 0));
        // The screen named, on the server's roots.
        assert_eq!(root_of(&[0x100, 0x200], None, 1), Some(0x200));
    }

    #[test]
    fn randr_below_1_3_is_not_read() {
        assert!(!supported(1, 2));
        assert!(supported(1, 3));
        assert!(supported(1, 6));
        assert!(supported(2, 0));
    }

    #[test]
    fn a_server_is_known_by_its_vendor_release_and_screens() {
        let screen = |root| Screen {
            root,
            width_in_pixels: 1920,
            height_in_pixels: 1080,
            ..Screen::default()
        };
        let setup = |vendor: &[u8], roots| Setup {
            vendor: vendor.to_vec(),
            release_number: 12_101_004,
            roots,
            ..Setup::default()
        };
        let a = setup(b"The X.Org Foundation", vec![screen(0x100)]);
        let mut ours = a.clone();
        ours.resource_id_base = 0x0420_0000;
        assert!(same_server(&a, &ours), "another client of the same server");
        assert!(!same_server(&a, &setup(b"The X.Org Foundation", vec![screen(0x200)])));
        assert!(!same_server(&a, &setup(b"Xwayland", vec![screen(0x100)])));
    }

    #[test]
    fn mirrored_outputs_on_one_crtc_are_each_an_output() {
        let mode = randr::ModeInfo {
            id: 7,
            width: 1920,
            height: 1080,
            dot_clock: 148_500_000,
            htotal: 2200,
            vtotal: 1125,
            ..randr::ModeInfo::default()
        };
        let crtc = randr::GetCrtcInfoReply {
            width: 1920,
            height: 1080,
            mode: 7,
            ..randr::GetCrtcInfoReply::default()
        };
        let output = |name: &[u8]| randr::GetOutputInfoReply {
            crtc: 63,
            connection: randr::Connection::CONNECTED,
            name: name.to_vec(),
            ..randr::GetOutputInfoReply::default()
        };
        let a = describe(&output(b"HDMI-1"), &crtc, &[mode], None, None).unwrap();
        let b = describe(&output(b"DP-1"), &crtc, &[mode], None, None).unwrap();
        assert_eq!((a.native, b.native), (63, 63));
        assert_ne!(a.key, b.key);
        assert!((a.refresh.interval.as_secs_f64() - 1.0 / 60.0).abs() < 1e-6);
        let off = randr::GetCrtcInfoReply { mode: 0, ..crtc };
        assert!(
            describe(&output(b"DP-2"), &off, &[mode], None, None).is_none(),
            "no mode: nothing shown"
        );
    }
}
