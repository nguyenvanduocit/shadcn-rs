//! X11: RandR over a connection of this crate's own, in safe Rust.
//!
//! - **Outputs** are the connected RandR outputs that drive a CRTC. The
//!   handle is the CRTC, as winit reports a monitor. The interval is the
//!   mode's pixel clock over its whole frame; identity, name, primaries and
//!   the vertical range come from the EDID property.
//! - **Variable refresh.** X11 tells no range. An output whose driver sets
//!   `vrr_capable` varies between the rates its EDID's range limits give.
//! - **Colour.** The gamut is the EDID's primaries; X11 has no HDR, so the
//!   headroom is none.
//! - **Changes**: a thread waits on the connection for RandR's
//!   notifications, sets the flag and wakes the owner.
//! - **Vsync**: none. The Present extension reports a window's presents,
//!   not an output's vblanks, and needs the toolkit's window (ADR-0029
//!   leaves it open with Wayland's presentation feedback). Callers that
//!   want a cadence take [`crate::Displays::synthesized`].

use std::sync::Arc;
use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{self, ConnectionExt as _, ModeFlag, NotifyMask};
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use crate::ticks::{Feed, Source};
use crate::{
    Backend, Colour, Description, Error, Gamut, Geometry, Headroom, Notice, OutputKey, Refresh, RefreshRange, edid,
    gamut,
};

fn os(e: impl std::fmt::Display) -> Error {
    Error::Os(e.to_string())
}

pub(crate) struct X11 {
    conn: Arc<RustConnection>,
    root: Window,
    /// A window nobody sees, which the watcher is told to stop through.
    door: Window,
    edid: u32,
    vrr_capable: u32,
}

impl X11 {
    pub(crate) fn new(notice: Notice) -> Result<Self, Error> {
        let (conn, screen) = x11rb::connect(None).map_err(os)?;
        let conn = Arc::new(conn);
        let root = conn.setup().roots.get(screen).ok_or_else(|| os("no such screen"))?.root;
        // 1.2 has outputs, CRTCs and their notifications; later versions add nothing read here.
        let version = conn.randr_query_version(1, 5).map_err(os)?.reply().map_err(os)?;
        if (version.major_version, version.minor_version) < (1, 2) {
            return Err(Error::Unsupported("an X server without RandR 1.2"));
        }
        let atom = |name: &[u8]| -> Result<u32, Error> {
            Ok(conn.intern_atom(false, name).map_err(os)?.reply().map_err(os)?.atom)
        };
        let (edid, vrr_capable) = (atom(b"EDID")?, atom(b"vrr_capable")?);
        let door = conn.generate_id().map_err(os)?;
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
        .map_err(os)?;
        let mask = NotifyMask::SCREEN_CHANGE
            | NotifyMask::CRTC_CHANGE
            | NotifyMask::OUTPUT_CHANGE
            | NotifyMask::OUTPUT_PROPERTY;
        conn.randr_select_input(root, mask).map_err(os)?;
        conn.flush().map_err(os)?;
        let theirs = Arc::clone(&conn);
        std::thread::Builder::new()
            .name("gui-display x11".to_owned())
            .spawn(move || watch(&theirs, door, &notice))
            .map_err(os)?;
        Ok(Self {
            conn,
            root,
            door,
            edid,
            vrr_capable,
        })
    }

    /// The bytes of an output's property, or none.
    fn property(&self, output: randr::Output, atom: u32) -> Option<Vec<u8>> {
        let reply = self
            .conn
            .randr_get_output_property(output, atom, AtomEnum::ANY, 0, 256, false, false)
            .ok()?
            .reply()
            .ok()?;
        (!reply.data.is_empty()).then_some(reply.data)
    }

    fn describe(
        &self,
        output: randr::Output,
        resources: &randr::GetScreenResourcesCurrentReply,
    ) -> Option<Description> {
        let info = self
            .conn
            .randr_get_output_info(output, resources.config_timestamp)
            .ok()?
            .reply()
            .ok()?;
        if info.connection != randr::Connection::CONNECTED || info.crtc == 0 {
            return None;
        }
        let crtc = self
            .conn
            .randr_get_crtc_info(info.crtc, resources.config_timestamp)
            .ok()?
            .reply()
            .ok()?;
        let mode = resources.modes.iter().find(|m| m.id == crtc.mode)?;
        let interval = edid::refresh_interval(
            mode.dot_clock,
            mode.htotal,
            mode.vtotal,
            mode.mode_flags.contains(ModeFlag::INTERLACE),
            mode.mode_flags.contains(ModeFlag::DOUBLE_SCAN),
        )
        .unwrap_or(Duration::from_nanos(16_666_667));
        let connector = String::from_utf8_lossy(&info.name).into_owned();
        let edid = self.property(output, self.edid).and_then(|b| edid::parse(&b));
        // Any byte set: drivers store the flag in 8 or 32 bits.
        let varies = self
            .property(output, self.vrr_capable)
            .is_some_and(|b| b.iter().any(|b| *b != 0));
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
    fn probe(&mut self) -> Vec<Description> {
        let Some(resources) = self
            .conn
            .randr_get_screen_resources_current(self.root)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            return Vec::new();
        };
        resources
            .outputs
            .iter()
            .filter_map(|o| self.describe(*o, &resources))
            .collect()
    }

    fn source(&mut self, _: &Description, _: Feed) -> Result<Box<dyn Source>, Error> {
        Err(Error::Unsupported("X11 (no vsync source for an output)"))
    }
}

impl Drop for X11 {
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

#[cfg(test)]
mod tests {
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
}
