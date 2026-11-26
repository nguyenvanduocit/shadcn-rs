//! A log of the input the platform thread stamps (ADR-0015), and the one
//! routing both a window and a replay go through.
//!
//! The platform thread turns each winit event it forwards into an
//! [`Event`]: what the platform gave, before `gui` makes anything of it
//! (a wheel's raw delta, line or pixel, in winit's sign, with its phase).
//! [`Router::route`] then makes of it what the UI or the render thread is
//! sent, keeping the pointer position and the modifiers, as the platform
//! thread does. A window run with `GUI_RECORD_INPUT=<file>` writes every
//! [`Event`] with the moment it arrived to that file ([`Recorder`]); the
//! compositor's replay tests feed such a log through [`Router::route`], the
//! compositor and a UI pipeline on a fake display clock.
//!
//! The log is text, one line an event, so a log can be read, cut and
//! written by hand:
//!
//! ```text
//! gui-input-log 2
//! platform macos
//! window 480 320 2
//! source synthesized from ...
//! 0.000 move 150 160
//! 12.500 wheel pixels 0 -3 began
//! 20.833 wheel pixels 0 -7.5 changed
//! 40.000 down
//! 41.250 key named Enter
//! 42.000 key char "a" "a"
//! ```
//!
//! - The header: the format's version; the platform the events came from
//!   (`macos`, `windows`, `wayland`, `x11`, `other`); the window's logical
//!   size and scale when recording began; free text on where the log came
//!   from. Lines starting with `#` are comments, anywhere.
//! - An event line starts with its time, milliseconds since recording
//!   began (written to the nanosecond: a gesture's resampling and its
//!   fling read event times, so a rounded time replays to another
//!   offset), then its kind:
//!   - `move <x> <y>`: the pointer, logical window pixels; `leave`;
//!   - `down`, `up` (the primary button), `secondary` (the secondary
//!     button went down), at the last pointer position;
//!   - `wheel lines|pixels <dx> <dy> <phase>`: a line delta (a notched
//!     wheel) or a pixel delta in logical pixels (a trackpad's), in
//!     winit's sign (positive `dy`: the content moves down), with its
//!     place in the platform's gesture ([`WheelPhase`]): `none`, `began`,
//!     `changed`, `ended`, `cancelled`, `momentum-began`, `momentum`,
//!     `momentum-ended` (on macOS `NSEvent`'s own phase and momentum
//!     phase; elsewhere what winit tells);
//!   - `mods <shift+ctrl+alt+cmd>` or `mods none`;
//!   - `key <key> [text]` (`repeat <key> [text]` for a key held down): a
//!     key pressed, its logical key and the text it types, the key one of
//!     `named <Name>` (every winit named key, by its name), `char <text>`,
//!     `dead "<char>"|none`, `native unidentified|<kind> <code>`;
//!   - `preedit <text> <start> <end>` or `preedit <text> -`, `commit <text>`:
//!     the input method;
//!   - `act <action> <node> <tree> [data]`: an assistive technology's
//!     action request (AccessKit's), its tree `root` or a hex id, its data
//!     one of `custom <n>`, `value <text>`, `number <n>`, `unit item|page`,
//!     `hint <hint>`, `point <x> <y>`, `offset <x> <y>`, `selection
//!     <node> <index> <node> <index>`.
//!
//!   Texts are quoted, with `\\`, `\"`, `\n`, `\r`, `\t` and `\u{…}`
//!   escaped.
//! - An observation line (a recorded log's, [`Observation`]) starts with
//!   its time, then `obs` and what was decided between the events, in its
//!   order (ADR-0033): `grant [<ms>]` (the UI was granted a main frame,
//!   for the present at `<ms>`, where a held drag is resampled), `commit`
//!   (the UI's commit was taken in), `abort` (one was dropped), `drawn <n>
//!   [<ms>]` (frame `n` was drawn, for its display at `<ms>`), `seen
//!   <n>|none` (the frame on screen when the events after it came, which
//!   they are routed in), `diverted` and `restored` (input went straight
//!   to the UI from there, and through the render thread again),
//!   `uncommitted` (the log began before the render thread had a commit:
//!   a wheel finds no container until the first `commit`), `settled` (no
//!   frame drew, so the gesture's resampled motion and fling were applied
//!   whole before the input after).
//! - What the platform told of its window is a line of its own, in order
//!   with the events: `resize <w> <h> <scale>`, `focus`, `visible`,
//!   `hidden`, `chrome <flags> "<title>"`, `access on|off`, `quit`. A replay of a log
//!   that has them follows them in sequence ([`super::replay`]).
//!
//! A window's log is written where each input is taken: by the render
//! thread, which every input goes through (ADR-0033), or by the platform
//! while the render thread cannot serve, under the one lock that switches
//! the two ([`super::thread::Forwarding`]). Its lines are in the one order
//! the events were taken, frames granted and drawn and commits taken in,
//! and the UI handles what it is handed in that order too.

use std::io::Write as _;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Instant;

use winit::keyboard::{Key, NamedKey, NativeKey};

use super::WheelPhase;
use crate::edit::Preedit;
use crate::events::{ImeEvent, KeyEvent, Modifiers, TouchEvent, TouchPhase};
use crate::interact::LINE_SCROLL;
use crate::layer::{PointerButton, PointerKind};
use crate::window::Input;

/// The log format's version, the header's first line.
const MAGIC: &str = "gui-input-log 2";

/// A wheel phase's name in the log, and back.
const WHEEL_PHASES: [(WheelPhase, &str); 9] = [
    (WheelPhase::None, "none"),
    (WheelPhase::Touched, "touched"),
    (WheelPhase::Began, "began"),
    (WheelPhase::Changed, "changed"),
    (WheelPhase::Ended, "ended"),
    (WheelPhase::Cancelled, "cancelled"),
    (WheelPhase::MomentumBegan, "momentum-began"),
    (WheelPhase::Momentum, "momentum"),
    (WheelPhase::MomentumEnded, "momentum-ended"),
];

/// What the platform thread got, as it got it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The pointer moved to `(x, y)`, logical window pixels.
    Moved(f32, f32),
    Left,
    /// The window lost the keyboard focus: the pointers down are cancelled.
    Blur,
    /// The primary button went down (`true`) or up, where the pointer is.
    Primary(bool),
    /// A button other than the primary went down, where the pointer is
    /// (`ButtonDown(Primary)` is `Primary(true)`).
    ButtonDown(PointerButton),
    /// The wheel or a trackpad scrolled, at its place in the platform's
    /// gesture as the platform told it.
    Wheel {
        delta: WheelDelta,
        phase: WheelPhase,
    },
    Modifiers(Modifiers),
    /// A key went down: its logical key and the text it types.
    Key {
        key: Key,
        text: Option<String>,
        /// The platform sent it again for a key held down.
        repeat: bool,
    },
    /// The input method composes.
    Preedit(Preedit),
    /// The input method commits text.
    Commit(String),
    /// A finger (or a pen, when `pen`) touched, moved, lifted or was
    /// cancelled at `(x, y)`, logical window pixels.
    Touch {
        id: u64,
        pen: bool,
        phase: Phase,
        x: f32,
        y: f32,
    },
    /// An assistive technology asked a node to act.
    Act(accesskit::ActionRequest),
    /// The platform told the UI of its window ([`Told`]).
    Told(Told),
}

/// What the platform tells the UI of its window besides input: sent in one
/// order with input (ADR-0033's one producer), through the render thread
/// while it serves, logged where it is taken, and replayed from the log.
#[derive(Clone, Debug, PartialEq)]
pub enum Told {
    /// The window is `size` physical pixels at `scale` (physical pixels
    /// per logical pixel): its size or its scale changed.
    Resized { size: (u32, u32), scale: f32 },
    /// The window took the keyboard focus (losing it is input: `Blur`).
    Focused,
    /// The window can (or cannot) be seen.
    Visible(bool),
    /// The window's decorations changed.
    Chrome(crate::window::Chrome),
    /// A screen reader asked for the whole tree (`true`), or went away.
    Access(bool),
    /// The window closes.
    Quit,
}

/// A wheel's delta as winit gives it, positive moving the content down and
/// right.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDelta {
    /// Lines (a notched wheel, a Windows precision touchpad, a wheel on
    /// macOS that has no precise deltas).
    Lines(f32, f32),
    /// Logical pixels (a trackpad on macOS and Wayland).
    Pixels(f32, f32),
}

/// A wheel event's phase, winit's `TouchPhase`: on macOS the gesture's
/// `NSEvent` phase, or its momentum phase while momentum runs; on Wayland
/// `started` on the first event and `ended` on the axis stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Started,
    Moved,
    Ended,
    Cancelled,
}

impl From<winit::event::TouchPhase> for Phase {
    fn from(p: winit::event::TouchPhase) -> Self {
        use winit::event::TouchPhase;
        match p {
            TouchPhase::Started => Self::Started,
            TouchPhase::Moved => Self::Moved,
            TouchPhase::Ended => Self::Ended,
            TouchPhase::Cancelled => Self::Cancelled,
        }
    }
}

/// Where a log's events came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    Wayland,
    X11,
    Other,
}

impl Platform {
    const NAMES: [(Self, &'static str); 5] = [
        (Self::MacOs, "macos"),
        (Self::Windows, "windows"),
        (Self::Wayland, "wayland"),
        (Self::X11, "x11"),
        (Self::Other, "other"),
    ];

    fn name(self) -> &'static str {
        Self::NAMES.iter().find(|(p, _)| *p == self).map_or("other", |(_, n)| n)
    }

    /// Whether the platform sends a trackpad's momentum itself, so `gui`
    /// does not fling: macOS (momentum-phase events) and Windows (the
    /// precision touchpad's inertia, as more wheel messages).
    #[must_use]
    pub fn flings_itself(self) -> bool {
        matches!(self, Self::MacOs | Self::Windows)
    }
}

/// What a log says of where it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub platform: Platform,
    /// The window's logical size when recording began.
    pub size: (u32, u32),
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Where the log came from: recorded on which machine, or synthesized
    /// from which documented sequence.
    pub source: String,
}

/// An event and when it arrived, seconds since recording began.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamped {
    pub at: f64,
    pub event: Event,
}

/// What the render thread did between the events it took (ADR-0033), in
/// its order: what a replay needs, beyond the events, to run what the live
/// threads ran. Where a main frame fell among the inputs, and when the
/// commit answering it was taken in, depend on how the UI's work fell
/// against the render thread's, which the events alone do not say.
///
/// Each is a nondeterministic input the execution consumed, recorded where
/// it was consumed (rr's and ReVirt's rule: what a replay cannot derive is
/// logged at a point it reproduces): where the frames fell, which frame
/// each input was routed in, and when each main frame is presented, which
/// its resampled drag is sampled at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation {
    /// It granted the UI a main frame, pulling what it scrolled, to be
    /// presented at this time (seconds since recording began; `None` in a
    /// log written by hand: when the replay grants it).
    Grant(Option<f64>),
    /// It took in the UI's commit answering the last main frame.
    Committed,
    /// It dropped a commit (a delta while it awaits a full one, ADR-0032 I4).
    Aborted,
    /// It drew the frame numbered so for its display at this time (seconds
    /// since recording began, what the compositor ticked its scroll and
    /// motion to; `None` in a log written by hand: when the replay draws
    /// it). Input from its present on is routed in it.
    Drawn(u64, Option<f64>),
    /// Input went straight to the UI from here (ADR-0033's fallback): what
    /// the compositor scrolled, its gesture settled, was handed on, with
    /// the gesture's latch; the UI scrolls the wheels after it.
    Diverted,
    /// Input went through the render thread again from here: its
    /// compositor took the UI's full commit and can scroll.
    Restored,
    /// The log began before the render thread had any commit: a wheel it
    /// takes finds no scroll container until the first `commit`.
    Uncommitted,
    /// No frame is drawn (no device, the window hidden), so none ticks the
    /// gesture: its resampled motion and fling were applied whole here,
    /// before the input after is handed on with them (Opus r12 P2).
    Settled,
    /// The UI took one value another thread sent to the mailbox at this
    /// address (its maker's site hash and number, [`crate::mailbox`]),
    /// here among the input: a nondeterministic input of its own, logged
    /// by the one producer where the UI takes it. A replay delivers that
    /// mailbox's next value here, and no mail anywhere else; the value is
    /// what its sender sends the replay (the log holds where, not what).
    Mail(u64, u64),
    /// The events from here on were routed in the frame numbered so, as
    /// the platform found it on screen as each came; `None`: before the
    /// first.
    Seen(Option<u64>),
}

/// An observation and when it was made, seconds since recording began,
/// after the log's first `after` events.
#[derive(Clone, Debug, PartialEq)]
pub struct Observed {
    pub after: usize,
    pub at: f64,
    pub what: Observation,
}

/// A log of input.
#[derive(Clone, Debug, PartialEq)]
pub struct InputLog {
    pub header: Header,
    pub events: Vec<Stamped>,
    /// What the threads observed between the events, in order
    /// ([`Observation`]): none in a log synthesized by hand.
    pub observed: Vec<Observed>,
}

impl InputLog {
    /// Read a log.
    ///
    /// # Errors
    ///
    /// The line that cannot be read, and why.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, l)| (i.saturating_add(1), l.trim()))
            .filter(|(_, l)| !l.is_empty() && !l.starts_with('#'));
        match lines.next() {
            Some((_, l)) if l == MAGIC => {}
            _ => return Err(format!("not an input log: the first line is not {MAGIC:?}")),
        }
        let mut header = Header {
            platform: Platform::Other,
            size: (800, 600),
            scale: 1.0,
            source: String::new(),
        };
        let mut events = Vec::new();
        let mut observed = Vec::new();
        for (n, line) in lines {
            let fail = |why: &str| format!("line {n}: {why}: {line:?}");
            let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
            if let Ok(ms) = head.parse::<f64>() {
                if let Some(what) = rest.strip_prefix("obs ") {
                    observed.push(Observed {
                        after: events.len(),
                        at: ms / 1000.0,
                        what: parse_observation(what).map_err(|e| fail(&e))?,
                    });
                    continue;
                }
                let event = parse_event(rest).map_err(|e| fail(&e))?;
                events.push(Stamped { at: ms / 1000.0, event });
                continue;
            }
            match head {
                "platform" => {
                    header.platform = Platform::NAMES
                        .iter()
                        .find(|(_, name)| *name == rest)
                        .map(|(p, _)| *p)
                        .ok_or_else(|| fail("unknown platform"))?;
                }
                "window" => {
                    let v: Vec<&str> = rest.split_whitespace().collect();
                    let [w, h, s] = v.as_slice() else {
                        return Err(fail("want `window <width> <height> <scale>`"));
                    };
                    header.size = (
                        w.parse().map_err(|_| fail("width"))?,
                        h.parse().map_err(|_| fail("height"))?,
                    );
                    header.scale = s.parse().map_err(|_| fail("scale"))?;
                }
                "source" => {
                    if !header.source.is_empty() {
                        header.source.push(' ');
                    }
                    header.source.push_str(rest);
                }
                _ => return Err(fail("neither a header line nor an event")),
            }
        }
        Ok(Self {
            header,
            events,
            observed,
        })
    }

    /// The log as text, which [`Self::parse`] reads back the same.
    #[must_use]
    pub fn write(&self) -> String {
        let mut out = header_text(&self.header);
        let mut observed = self.observed.iter().peekable();
        for (i, e) in self.events.iter().enumerate() {
            while let Some(o) = observed.next_if(|o| o.after <= i) {
                out.push_str(&observation_line(o.at, o.what));
                out.push('\n');
            }
            out.push_str(&event_line(e.at, &e.event));
            out.push('\n');
        }
        for o in observed {
            out.push_str(&observation_line(o.at, o.what));
            out.push('\n');
        }
        out
    }
}

/// One observation's line, `at` seconds since recording began.
fn observation_line(at: f64, what: Observation) -> String {
    let what = match what {
        Observation::Grant(None) => "grant".to_owned(),
        Observation::Grant(Some(p)) => format!("grant {:.6}", p * 1000.0),
        Observation::Committed => "commit".to_owned(),
        Observation::Aborted => "abort".to_owned(),
        Observation::Drawn(n, None) => format!("drawn {n}"),
        Observation::Drawn(n, Some(p)) => format!("drawn {n} {:.6}", p * 1000.0),
        Observation::Diverted => "diverted".to_owned(),
        Observation::Restored => "restored".to_owned(),
        Observation::Uncommitted => "uncommitted".to_owned(),
        Observation::Settled => "settled".to_owned(),
        Observation::Mail(site, nth) => format!("mail {site:016x} {nth}"),
        Observation::Seen(Some(n)) => format!("seen {n}"),
        Observation::Seen(None) => "seen none".to_owned(),
    };
    format!("{:.6} obs {what}", at * 1000.0)
}

/// An observation from what follows `obs` on its line.
fn parse_observation(rest: &str) -> Result<Observation, String> {
    let frame = |n: &str| n.parse::<u64>().map_err(|_| format!("not a frame number: {n}"));
    let time = |ms: &str| Ok::<f64, String>(ms.parse::<f64>().map_err(|_| format!("not a time: {ms}"))? / 1000.0);
    let words: Vec<&str> = rest.split_whitespace().collect();
    Ok(match words.as_slice() {
        ["grant"] => Observation::Grant(None),
        ["grant", ms] => Observation::Grant(Some(time(ms)?)),
        ["commit"] => Observation::Committed,
        ["abort"] => Observation::Aborted,
        ["drawn", n] => Observation::Drawn(frame(n)?, None),
        ["drawn", n, ms] => Observation::Drawn(frame(n)?, Some(time(ms)?)),
        ["diverted"] => Observation::Diverted,
        ["restored"] => Observation::Restored,
        ["uncommitted"] => Observation::Uncommitted,
        ["settled"] => Observation::Settled,
        ["mail", site, nth] => Observation::Mail(
            u64::from_str_radix(site, 16).map_err(|_| format!("not a mailbox site: {site}"))?,
            nth.parse().map_err(|_| format!("not a mailbox number: {nth}"))?,
        ),
        ["seen", "none"] => Observation::Seen(None),
        ["seen", n] => Observation::Seen(Some(frame(n)?)),
        _ => return Err("not an observation".to_owned()),
    })
}

fn header_text(h: &Header) -> String {
    let mut out = format!(
        "{MAGIC}\nplatform {}\nwindow {} {} {}\n",
        h.platform.name(),
        h.size.0,
        h.size.1,
        h.scale
    );
    for line in h.source.lines() {
        out.push_str("source ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// One event's line, `at` seconds since recording began.
fn event_line(at: f64, event: &Event) -> String {
    let mut s = format!("{:.6} ", at * 1000.0);
    match event {
        Event::Moved(x, y) => {
            s.push_str(&format!("move {x} {y}"));
        }
        Event::Left => s.push_str("leave"),
        Event::Blur => s.push_str("blur"),
        Event::Primary(true) => s.push_str("down"),
        Event::Primary(false) => s.push_str("up"),
        Event::ButtonDown(PointerButton::Secondary) => s.push_str("secondary"),
        Event::ButtonDown(PointerButton::Middle) => s.push_str("middle"),
        Event::ButtonDown(PointerButton::Other(n)) => s.push_str(&format!("button {n}")),
        // Not logged as a button: the primary is `down`.
        Event::ButtonDown(PointerButton::Primary) => s.push_str("down"),
        Event::Wheel { delta, phase } => {
            let (kind, x, y) = match delta {
                WheelDelta::Lines(x, y) => ("lines", x, y),
                WheelDelta::Pixels(x, y) => ("pixels", x, y),
            };
            let phase = WHEEL_PHASES.iter().find(|(p, _)| p == phase).map_or("none", |(_, n)| n);
            s.push_str(&format!("wheel {kind} {x} {y} {phase}"));
        }
        Event::Modifiers(m) => {
            let names: Vec<&str> = [
                (m.shift, "shift"),
                (m.control, "ctrl"),
                (m.alt, "alt"),
                (m.command, "cmd"),
            ]
            .into_iter()
            .filter_map(|(on, n)| on.then_some(n))
            .collect();
            s.push_str(&format!(
                "mods {}",
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join("+")
                }
            ));
        }
        Event::Key { key, text, repeat } => {
            let verb = if *repeat { "repeat" } else { "key" };
            s.push_str(&format!("{verb} {}", key_text(key)));
            if let Some(t) = text {
                s.push(' ');
                s.push_str(&quote(t));
            }
        }
        Event::Preedit(p) => {
            s.push_str(&format!("preedit {}", quote(&p.text)));
            match p.cursor {
                Some((a, b)) => {
                    s.push_str(&format!(" {a} {b}"));
                }
                None => s.push_str(" -"),
            }
        }
        Event::Commit(t) => {
            s.push_str(&format!("commit {}", quote(t)));
        }
        Event::Touch { id, pen, phase, x, y } => {
            let what = if *pen { "pen" } else { "finger" };
            let phase = match phase {
                Phase::Started => "started",
                Phase::Moved => "moved",
                Phase::Ended => "ended",
                Phase::Cancelled => "cancelled",
            };
            s.push_str(&format!("touch {id} {what} {phase} {x} {y}"));
        }
        Event::Act(r) => s.push_str(&act_text(r)),
        Event::Told(t) => s.push_str(&told_text(t)),
    }
    s
}

/// The flags of a window's decorations, by their names in the log.
const CHROME_FLAGS: [&str; 6] = ["client", "maximized", "fullscreen", "active", "dark", "resizable"];

/// What the platform told, as a log line names it.
fn told_text(t: &Told) -> String {
    match t {
        Told::Resized { size, scale } => format!("resize {} {} {scale}", size.0, size.1),
        Told::Focused => "focus".to_owned(),
        Told::Visible(true) => "visible".to_owned(),
        Told::Visible(false) => "hidden".to_owned(),
        Told::Chrome(c) => {
            let on = [c.client, c.maximized, c.fullscreen, c.active, c.dark, c.resizable];
            let flags: Vec<&str> = CHROME_FLAGS
                .iter()
                .zip(on)
                .filter_map(|(n, on)| on.then_some(*n))
                .collect();
            let flags = if flags.is_empty() {
                "none".to_owned()
            } else {
                flags.join("+")
            };
            format!("chrome {flags} {}", quote(&c.title))
        }
        Told::Access(true) => "access on".to_owned(),
        Told::Access(false) => "access off".to_owned(),
        Told::Quit => "quit".to_owned(),
    }
}

/// What the platform told, from its log line's words, if it is that.
fn parse_told(w: &[&str]) -> Result<Option<Told>, String> {
    Ok(Some(match w {
        ["resize", width, height, scale] => Told::Resized {
            size: (
                width.parse().map_err(|_| format!("not a width: {width}"))?,
                height.parse().map_err(|_| format!("not a height: {height}"))?,
            ),
            scale: scale.parse().map_err(|_| format!("not a scale: {scale}"))?,
        },
        ["focus"] => Told::Focused,
        ["visible"] => Told::Visible(true),
        ["hidden"] => Told::Visible(false),
        ["chrome", flags, title] => {
            let mut c = crate::window::Chrome {
                title: (*title).to_owned(),
                ..crate::window::Chrome::default()
            };
            for flag in flags.split('+').filter(|f| *f != "none") {
                let on = match flag {
                    "client" => &mut c.client,
                    "maximized" => &mut c.maximized,
                    "fullscreen" => &mut c.fullscreen,
                    "active" => &mut c.active,
                    "dark" => &mut c.dark,
                    "resizable" => &mut c.resizable,
                    f => return Err(format!("unknown decoration flag {f}")),
                };
                *on = true;
            }
            Told::Chrome(c)
        }
        ["access", "on"] => Told::Access(true),
        ["access", "off"] => Told::Access(false),
        ["quit"] => Told::Quit,
        _ => return Ok(None),
    }))
}

/// The actions an assistive technology asks, by their names in the log.
const ACTIONS: [(accesskit::Action, &str); 22] = {
    use accesskit::Action as A;
    [
        (A::Click, "click"),
        (A::Focus, "focus"),
        (A::Blur, "blur"),
        (A::Collapse, "collapse"),
        (A::Expand, "expand"),
        (A::CustomAction, "custom"),
        (A::Decrement, "decrement"),
        (A::Increment, "increment"),
        (A::HideTooltip, "hide-tooltip"),
        (A::ShowTooltip, "show-tooltip"),
        (A::ReplaceSelectedText, "replace-selected-text"),
        (A::ScrollDown, "scroll-down"),
        (A::ScrollLeft, "scroll-left"),
        (A::ScrollRight, "scroll-right"),
        (A::ScrollUp, "scroll-up"),
        (A::ScrollIntoView, "scroll-into-view"),
        (A::ScrollToPoint, "scroll-to-point"),
        (A::SetScrollOffset, "set-scroll-offset"),
        (A::SetTextSelection, "set-text-selection"),
        (A::SetSequentialFocusNavigationStartingPoint, "set-focus-start"),
        (A::SetValue, "set-value"),
        (A::ShowContextMenu, "show-context-menu"),
    ]
};

/// The scroll hints a request names, by their names in the log.
const HINTS: [(accesskit::ScrollHint, &str); 6] = {
    use accesskit::ScrollHint as H;
    [
        (H::TopLeft, "top-left"),
        (H::BottomRight, "bottom-right"),
        (H::TopEdge, "top-edge"),
        (H::BottomEdge, "bottom-edge"),
        (H::LeftEdge, "left-edge"),
        (H::RightEdge, "right-edge"),
    ]
};

/// An action request's words: `act <action> <node> <tree> [data]`, the
/// tree `root` or its id in hex.
fn act_text(r: &accesskit::ActionRequest) -> String {
    use accesskit::{ActionData as D, ScrollUnit};
    let action = ACTIONS.iter().find(|(a, _)| *a == r.action).map_or("click", |(_, n)| n);
    let tree = if r.target_tree == accesskit::TreeId::ROOT {
        "root".to_owned()
    } else {
        format!("{:x}", r.target_tree.0.as_u128())
    };
    let mut s = format!("act {action} {} {tree}", r.target_node.0);
    let data = match &r.data {
        None => String::new(),
        Some(D::CustomAction(n)) => format!(" custom {n}"),
        Some(D::Value(v)) => format!(" value {}", quote(v)),
        Some(D::NumericValue(v)) => format!(" number {v}"),
        Some(D::ScrollUnit(ScrollUnit::Item)) => " unit item".to_owned(),
        Some(D::ScrollUnit(ScrollUnit::Page)) => " unit page".to_owned(),
        Some(D::ScrollHint(h)) => {
            format!(
                " hint {}",
                HINTS.iter().find(|(k, _)| k == h).map_or("top-left", |(_, n)| n)
            )
        }
        Some(D::ScrollToPoint(p)) => format!(" point {} {}", p.x, p.y),
        Some(D::SetScrollOffset(p)) => format!(" offset {} {}", p.x, p.y),
        Some(D::SetTextSelection(t)) => format!(
            " selection {} {} {} {}",
            t.anchor.node.0, t.anchor.character_index, t.focus.node.0, t.focus.character_index
        ),
    };
    s.push_str(&data);
    s
}

/// An action request from its words after `act`.
fn parse_act(w: &[&str]) -> Result<Event, String> {
    use accesskit::{ActionData as D, NodeId, Point, ScrollUnit, TextPosition, TextSelection};
    let [action, node, tree, data @ ..] = w else {
        return Err("want `act <action> <node> <tree> [data]`".to_owned());
    };
    let action = ACTIONS
        .iter()
        .find(|(_, n)| n == action)
        .map(|(a, _)| *a)
        .ok_or_else(|| format!("unknown action {action}"))?;
    let id = |s: &str| s.parse::<u64>().map(NodeId).map_err(|_| format!("not a node: {s}"));
    let n64 = |s: &str| s.parse::<f64>().map_err(|_| format!("not a number: {s}"));
    let index = |s: &str| s.parse::<usize>().map_err(|_| format!("not an index: {s}"));
    let target_tree = match *tree {
        "root" => accesskit::TreeId::ROOT,
        hex => accesskit::TreeId(accesskit::Uuid::from_u128(
            u128::from_str_radix(hex, 16).map_err(|_| format!("not a tree: {hex}"))?,
        )),
    };
    let data = match data {
        [] => None,
        ["custom", n] => Some(D::CustomAction(n.parse().map_err(|_| format!("not a number: {n}"))?)),
        ["value", v] => Some(D::Value((*v).into())),
        ["number", v] => Some(D::NumericValue(n64(v)?)),
        ["unit", "item"] => Some(D::ScrollUnit(ScrollUnit::Item)),
        ["unit", "page"] => Some(D::ScrollUnit(ScrollUnit::Page)),
        ["hint", h] => Some(D::ScrollHint(
            HINTS
                .iter()
                .find(|(_, n)| n == h)
                .map(|(k, _)| *k)
                .ok_or_else(|| format!("unknown hint {h}"))?,
        )),
        ["point", x, y] => Some(D::ScrollToPoint(Point::new(n64(x)?, n64(y)?))),
        ["offset", x, y] => Some(D::SetScrollOffset(Point::new(n64(x)?, n64(y)?))),
        ["selection", an, ai, fnode, fi] => Some(D::SetTextSelection(TextSelection {
            anchor: TextPosition {
                node: id(an)?,
                character_index: index(ai)?,
            },
            focus: TextPosition {
                node: id(fnode)?,
                character_index: index(fi)?,
            },
        })),
        _ => return Err("unknown action data".to_owned()),
    };
    Ok(Event::Act(accesskit::ActionRequest {
        action,
        target_tree,
        target_node: id(node)?,
        data,
    }))
}

fn parse_event(rest: &str) -> Result<Event, String> {
    let words = tokens(rest)?;
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let num = |s: &str| s.parse::<f32>().map_err(|_| format!("not a number: {s}"));
    if let Some(told) = parse_told(&w)? {
        return Ok(Event::Told(told));
    }
    Ok(match w.as_slice() {
        ["move", x, y] => Event::Moved(num(x)?, num(y)?),
        ["leave"] => Event::Left,
        ["blur"] => Event::Blur,
        ["down"] => Event::Primary(true),
        ["up"] => Event::Primary(false),
        ["secondary"] => Event::ButtonDown(PointerButton::Secondary),
        ["middle"] => Event::ButtonDown(PointerButton::Middle),
        ["button", n] => Event::ButtonDown(PointerButton::Other(
            n.parse().map_err(|_| format!("not a button number: {n}"))?,
        )),
        ["touch", id, what, phase, x, y] => Event::Touch {
            id: id.parse().map_err(|_| format!("not a touch id: {id}"))?,
            pen: match *what {
                "pen" => true,
                "finger" => false,
                w => return Err(format!("a touch is a finger or a pen, not {w}")),
            },
            phase: match *phase {
                "started" => Phase::Started,
                "moved" => Phase::Moved,
                "ended" => Phase::Ended,
                "cancelled" => Phase::Cancelled,
                p => return Err(format!("unknown phase {p}")),
            },
            x: num(x)?,
            y: num(y)?,
        },
        ["wheel", kind, x, y, phase] => {
            let (x, y) = (num(x)?, num(y)?);
            let delta = match *kind {
                "lines" => WheelDelta::Lines(x, y),
                "pixels" => WheelDelta::Pixels(x, y),
                k => return Err(format!("a wheel delta is lines or pixels, not {k}")),
            };
            let phase = WHEEL_PHASES
                .iter()
                .find(|(_, n)| n == phase)
                .map(|(p, _)| *p)
                .ok_or_else(|| format!("unknown wheel phase {phase}"))?;
            Event::Wheel { delta, phase }
        }
        ["mods", list] => {
            let mut m = Modifiers::default();
            for name in list.split('+').filter(|n| *n != "none") {
                match name {
                    "shift" => m.shift = true,
                    "ctrl" => m.control = true,
                    "alt" => m.alt = true,
                    "cmd" => m.command = true,
                    n => return Err(format!("unknown modifier {n}")),
                }
            }
            Event::Modifiers(m)
        }
        [verb @ ("key" | "repeat"), rest @ ..] => {
            let (key, text) = parse_key(rest)?;
            let text = match text {
                [] => None,
                [t] => Some((*t).to_owned()),
                _ => return Err("a key types one text".to_owned()),
            };
            Event::Key {
                key,
                text,
                repeat: *verb == "repeat",
            }
        }
        ["preedit", text, "-"] => Event::Preedit(Preedit {
            text: (*text).to_owned(),
            cursor: None,
        }),
        ["preedit", text, a, b] => Event::Preedit(Preedit {
            text: (*text).to_owned(),
            cursor: Some((
                a.parse().map_err(|_| format!("not a byte offset: {a}"))?,
                b.parse().map_err(|_| format!("not a byte offset: {b}"))?,
            )),
        }),
        ["commit", text] => Event::Commit((*text).to_owned()),
        ["act", rest @ ..] => parse_act(rest)?,
        _ => return Err("unknown event".to_owned()),
    })
}

/// Every named key winit has (0.30, `keyboard.rs`, in its order), each
/// logged by its own name (its `Debug`, which is the W3C `key` value bar
/// winit's few exceptions), so every key an app may read reads back as
/// itself (Codex r11 4). A test holds the list to winit's whole enum.
const NAMED_KEYS: [NamedKey; 306] = {
    use NamedKey as K;
    [
        K::Alt,
        K::AltGraph,
        K::CapsLock,
        K::Control,
        K::Fn,
        K::FnLock,
        K::NumLock,
        K::ScrollLock,
        K::Shift,
        K::Symbol,
        K::SymbolLock,
        K::Meta,
        K::Hyper,
        K::Super,
        K::Enter,
        K::Tab,
        K::Space,
        K::ArrowDown,
        K::ArrowLeft,
        K::ArrowRight,
        K::ArrowUp,
        K::End,
        K::Home,
        K::PageDown,
        K::PageUp,
        K::Backspace,
        K::Clear,
        K::Copy,
        K::CrSel,
        K::Cut,
        K::Delete,
        K::EraseEof,
        K::ExSel,
        K::Insert,
        K::Paste,
        K::Redo,
        K::Undo,
        K::Accept,
        K::Again,
        K::Attn,
        K::Cancel,
        K::ContextMenu,
        K::Escape,
        K::Execute,
        K::Find,
        K::Help,
        K::Pause,
        K::Play,
        K::Props,
        K::Select,
        K::ZoomIn,
        K::ZoomOut,
        K::BrightnessDown,
        K::BrightnessUp,
        K::Eject,
        K::LogOff,
        K::Power,
        K::PowerOff,
        K::PrintScreen,
        K::Hibernate,
        K::Standby,
        K::WakeUp,
        K::AllCandidates,
        K::Alphanumeric,
        K::CodeInput,
        K::Compose,
        K::Convert,
        K::FinalMode,
        K::GroupFirst,
        K::GroupLast,
        K::GroupNext,
        K::GroupPrevious,
        K::ModeChange,
        K::NextCandidate,
        K::NonConvert,
        K::PreviousCandidate,
        K::Process,
        K::SingleCandidate,
        K::HangulMode,
        K::HanjaMode,
        K::JunjaMode,
        K::Eisu,
        K::Hankaku,
        K::Hiragana,
        K::HiraganaKatakana,
        K::KanaMode,
        K::KanjiMode,
        K::Katakana,
        K::Romaji,
        K::Zenkaku,
        K::ZenkakuHankaku,
        K::Soft1,
        K::Soft2,
        K::Soft3,
        K::Soft4,
        K::ChannelDown,
        K::ChannelUp,
        K::Close,
        K::MailForward,
        K::MailReply,
        K::MailSend,
        K::MediaClose,
        K::MediaFastForward,
        K::MediaPause,
        K::MediaPlay,
        K::MediaPlayPause,
        K::MediaRecord,
        K::MediaRewind,
        K::MediaStop,
        K::MediaTrackNext,
        K::MediaTrackPrevious,
        K::New,
        K::Open,
        K::Print,
        K::Save,
        K::SpellCheck,
        K::Key11,
        K::Key12,
        K::AudioBalanceLeft,
        K::AudioBalanceRight,
        K::AudioBassBoostDown,
        K::AudioBassBoostToggle,
        K::AudioBassBoostUp,
        K::AudioFaderFront,
        K::AudioFaderRear,
        K::AudioSurroundModeNext,
        K::AudioTrebleDown,
        K::AudioTrebleUp,
        K::AudioVolumeDown,
        K::AudioVolumeUp,
        K::AudioVolumeMute,
        K::MicrophoneToggle,
        K::MicrophoneVolumeDown,
        K::MicrophoneVolumeUp,
        K::MicrophoneVolumeMute,
        K::SpeechCorrectionList,
        K::SpeechInputToggle,
        K::LaunchApplication1,
        K::LaunchApplication2,
        K::LaunchCalendar,
        K::LaunchContacts,
        K::LaunchMail,
        K::LaunchMediaPlayer,
        K::LaunchMusicPlayer,
        K::LaunchPhone,
        K::LaunchScreenSaver,
        K::LaunchSpreadsheet,
        K::LaunchWebBrowser,
        K::LaunchWebCam,
        K::LaunchWordProcessor,
        K::BrowserBack,
        K::BrowserFavorites,
        K::BrowserForward,
        K::BrowserHome,
        K::BrowserRefresh,
        K::BrowserSearch,
        K::BrowserStop,
        K::AppSwitch,
        K::Call,
        K::Camera,
        K::CameraFocus,
        K::EndCall,
        K::GoBack,
        K::GoHome,
        K::HeadsetHook,
        K::LastNumberRedial,
        K::Notification,
        K::MannerMode,
        K::VoiceDial,
        K::TV,
        K::TV3DMode,
        K::TVAntennaCable,
        K::TVAudioDescription,
        K::TVAudioDescriptionMixDown,
        K::TVAudioDescriptionMixUp,
        K::TVContentsMenu,
        K::TVDataService,
        K::TVInput,
        K::TVInputComponent1,
        K::TVInputComponent2,
        K::TVInputComposite1,
        K::TVInputComposite2,
        K::TVInputHDMI1,
        K::TVInputHDMI2,
        K::TVInputHDMI3,
        K::TVInputHDMI4,
        K::TVInputVGA1,
        K::TVMediaContext,
        K::TVNetwork,
        K::TVNumberEntry,
        K::TVPower,
        K::TVRadioService,
        K::TVSatellite,
        K::TVSatelliteBS,
        K::TVSatelliteCS,
        K::TVSatelliteToggle,
        K::TVTerrestrialAnalog,
        K::TVTerrestrialDigital,
        K::TVTimer,
        K::AVRInput,
        K::AVRPower,
        K::ColorF0Red,
        K::ColorF1Green,
        K::ColorF2Yellow,
        K::ColorF3Blue,
        K::ColorF4Grey,
        K::ColorF5Brown,
        K::ClosedCaptionToggle,
        K::Dimmer,
        K::DisplaySwap,
        K::DVR,
        K::Exit,
        K::FavoriteClear0,
        K::FavoriteClear1,
        K::FavoriteClear2,
        K::FavoriteClear3,
        K::FavoriteRecall0,
        K::FavoriteRecall1,
        K::FavoriteRecall2,
        K::FavoriteRecall3,
        K::FavoriteStore0,
        K::FavoriteStore1,
        K::FavoriteStore2,
        K::FavoriteStore3,
        K::Guide,
        K::GuideNextDay,
        K::GuidePreviousDay,
        K::Info,
        K::InstantReplay,
        K::Link,
        K::ListProgram,
        K::LiveContent,
        K::Lock,
        K::MediaApps,
        K::MediaAudioTrack,
        K::MediaLast,
        K::MediaSkipBackward,
        K::MediaSkipForward,
        K::MediaStepBackward,
        K::MediaStepForward,
        K::MediaTopMenu,
        K::NavigateIn,
        K::NavigateNext,
        K::NavigateOut,
        K::NavigatePrevious,
        K::NextFavoriteChannel,
        K::NextUserProfile,
        K::OnDemand,
        K::Pairing,
        K::PinPDown,
        K::PinPMove,
        K::PinPToggle,
        K::PinPUp,
        K::PlaySpeedDown,
        K::PlaySpeedReset,
        K::PlaySpeedUp,
        K::RandomToggle,
        K::RcLowBattery,
        K::RecordSpeedNext,
        K::RfBypass,
        K::ScanChannelsToggle,
        K::ScreenModeNext,
        K::Settings,
        K::SplitScreenToggle,
        K::STBInput,
        K::STBPower,
        K::Subtitle,
        K::Teletext,
        K::VideoModeNext,
        K::Wink,
        K::ZoomToggle,
        K::F1,
        K::F2,
        K::F3,
        K::F4,
        K::F5,
        K::F6,
        K::F7,
        K::F8,
        K::F9,
        K::F10,
        K::F11,
        K::F12,
        K::F13,
        K::F14,
        K::F15,
        K::F16,
        K::F17,
        K::F18,
        K::F19,
        K::F20,
        K::F21,
        K::F22,
        K::F23,
        K::F24,
        K::F25,
        K::F26,
        K::F27,
        K::F28,
        K::F29,
        K::F30,
        K::F31,
        K::F32,
        K::F33,
        K::F34,
        K::F35,
    ]
};

/// A key as a log names it: `named <Name>`, `char <text>`, `dead "<char>"`
/// or `dead none` (a quoted character is read back as one character,
/// `none` never: no character is ambiguous), or `native <kind> [<code>]` (a key winit could not name: its platform
/// code, `unidentified`, `android`, `macos`, `windows`, `xkb` or `web`).
fn key_text(key: &Key) -> String {
    match key {
        Key::Named(n) => format!("named {n:?}"),
        Key::Character(c) => format!("char {}", quote(c)),
        Key::Dead(Some(c)) => format!("dead {}", quote(&c.to_string())),
        // A word no one character reads as (Codex r12 3: `-` was one).
        Key::Dead(None) => "dead none".to_owned(),
        Key::Unidentified(native) => match native {
            NativeKey::Unidentified => "native unidentified".to_owned(),
            NativeKey::Android(c) => format!("native android {c}"),
            NativeKey::MacOS(c) => format!("native macos {c}"),
            NativeKey::Windows(c) => format!("native windows {c}"),
            NativeKey::Xkb(c) => format!("native xkb {c}"),
            NativeKey::Web(s) => format!("native web {}", quote(s)),
        },
    }
}

/// A key from its words after `key` or `repeat` ([`key_text`]), and the
/// words after it.
fn parse_key<'a, 'w>(words: &'a [&'w str]) -> Result<(Key, &'a [&'w str]), String> {
    let code = |c: &str| c.parse::<u32>().map_err(|_| format!("not a key code: {c}"));
    let short = |c: &str| c.parse::<u16>().map_err(|_| format!("not a key code: {c}"));
    Ok(match words {
        ["named", name, rest @ ..] => {
            let named = NAMED_KEYS
                .iter()
                .find(|k| format!("{k:?}") == *name)
                .ok_or_else(|| format!("unknown key {name}"))?;
            (Key::Named(*named), rest)
        }
        ["char", c, rest @ ..] => (Key::Character((*c).into()), rest),
        ["dead", "none", rest @ ..] => (Key::Dead(None), rest),
        ["dead", c, rest @ ..] => {
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => (Key::Dead(Some(ch)), rest),
                _ => return Err(format!("a dead key is one character, not {c}")),
            }
        }
        ["native", "unidentified", rest @ ..] => (Key::Unidentified(NativeKey::Unidentified), rest),
        ["native", kind, c, rest @ ..] => {
            let native = match *kind {
                "android" => NativeKey::Android(code(c)?),
                "macos" => NativeKey::MacOS(short(c)?),
                "windows" => NativeKey::Windows(short(c)?),
                "xkb" => NativeKey::Xkb(code(c)?),
                "web" => NativeKey::Web((*c).into()),
                k => return Err(format!("unknown native key kind {k}")),
            };
            (Key::Unidentified(native), rest)
        }
        _ => return Err("a key is named, char, dead or native".to_owned()),
    })
}

/// `s` quoted, as [`tokens`] reads it back.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{{{:x}}}", u32::from(c)));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The words of a line: split at spaces, a quoted text one word.
fn tokens(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let mut word = String::new();
        if c == '"' {
            chars.next();
            loop {
                match chars.next() {
                    None => return Err("a text is not closed".to_owned()),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some('\\') => word.push('\\'),
                        Some('"') => word.push('"'),
                        Some('n') => word.push('\n'),
                        Some('r') => word.push('\r'),
                        Some('t') => word.push('\t'),
                        Some('u') => {
                            if chars.next() != Some('{') {
                                return Err("want \\u{…}".to_owned());
                            }
                            let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                            let c = u32::from_str_radix(&hex, 16)
                                .ok()
                                .and_then(char::from_u32)
                                .ok_or_else(|| format!("not a character: \\u{{{hex}}}"))?;
                            word.push(c);
                        }
                        other => return Err(format!("unknown escape {other:?}")),
                    },
                    Some(c) => word.push(c),
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                word.push(c);
                chars.next();
            }
        }
        out.push(word);
    }
    Ok(out)
}

/// The input in a winit window event at `scale` (physical pixels per
/// logical pixel), as the platform thread logs and routes it; `None` for
/// what is not input (or input `gui` takes no notice of: the release of a
/// button other than the primary, key releases).
#[must_use]
pub fn from_winit(event: winit::event::WindowEvent, scale: f32) -> Option<Event> {
    use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::keyboard::ModifiersState;

    use crate::convert::f64_to_f32;
    Some(match event {
        WindowEvent::CursorMoved { position, .. } => {
            Event::Moved(f64_to_f32(position.x) / scale, f64_to_f32(position.y) / scale)
        }
        WindowEvent::CursorLeft { .. } => Event::Left,
        WindowEvent::MouseInput {
            state,
            button: MouseButton::Left,
            ..
        } => Event::Primary(state == ElementState::Pressed),
        // Every other button counts as it goes down (a press outside a layer
        // dismisses it whichever button it is: ADR-0024).
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button,
            ..
        } => Event::ButtonDown(match button {
            MouseButton::Left => PointerButton::Primary,
            MouseButton::Right => PointerButton::Secondary,
            MouseButton::Middle => PointerButton::Middle,
            // Back and forward are buttons 3 and 4, as the DOM numbers them.
            MouseButton::Back => PointerButton::Other(3),
            MouseButton::Forward => PointerButton::Other(4),
            MouseButton::Other(n) => PointerButton::Other(n),
        }),
        // What winit tells of the phase: a notched wheel's lines have none;
        // a trackpad's pixels start, move and end (on Wayland, where the
        // compositor sends no momentum; on X11 they only move, which the
        // router takes as unphased). On macOS winit folds the gesture's
        // phase and its momentum's into one, and the window reads the
        // `NSEvent`'s own instead ([`gui_display::current_scroll`]).
        WindowEvent::MouseWheel { delta, phase, .. } => match delta {
            MouseScrollDelta::LineDelta(x, y) => Event::Wheel {
                delta: WheelDelta::Lines(x, y),
                phase: WheelPhase::None,
            },
            MouseScrollDelta::PixelDelta(p) => Event::Wheel {
                delta: WheelDelta::Pixels(f64_to_f32(p.x) / scale, f64_to_f32(p.y) / scale),
                phase: match phase {
                    winit::event::TouchPhase::Started => WheelPhase::Began,
                    winit::event::TouchPhase::Moved => WheelPhase::Changed,
                    winit::event::TouchPhase::Ended => WheelPhase::Ended,
                    winit::event::TouchPhase::Cancelled => WheelPhase::Cancelled,
                },
            },
        },
        WindowEvent::ModifiersChanged(m) => {
            let s = m.state();
            Event::Modifiers(Modifiers {
                shift: s.contains(ModifiersState::SHIFT),
                control: s.contains(ModifiersState::CONTROL),
                alt: s.contains(ModifiersState::ALT),
                command: s.contains(ModifiersState::SUPER),
            })
        }
        WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => Event::Key {
            key: event.logical_key,
            text: event.text.map(|t| t.to_string()),
            repeat: event.repeat,
        },
        WindowEvent::Ime(Ime::Preedit(text, cursor)) => Event::Preedit(Preedit { text, cursor }),
        WindowEvent::Ime(Ime::Commit(text)) => Event::Commit(text),
        // An Apple Pencil reports its altitude; a finger does not.
        WindowEvent::Touch(t) => Event::Touch {
            id: t.id,
            pen: matches!(
                t.force,
                Some(winit::event::Force::Calibrated {
                    altitude_angle: Some(_),
                    ..
                })
            ),
            phase: t.phase.into(),
            x: f64_to_f32(t.location.x) / scale,
            y: f64_to_f32(t.location.y) / scale,
        },
        // Another input source was chosen mid-composition: what it held is gone.
        WindowEvent::Ime(Ime::Disabled) => Event::Preedit(Preedit::default()),
        _ => return None,
    })
}

/// What an event becomes: input for the UI thread, or a wheel step for the
/// render thread.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Routed {
    Ui(Input),
    /// The wheel at `point` (logical window pixels) moved `delta` (positive
    /// moves the content up and left): a trackpad's (`precise`) is
    /// resampled, a notched wheel's applied; `phase`: its place in the
    /// platform's gesture.
    Wheel {
        point: (f32, f32),
        delta: (f32, f32),
        precise: bool,
        phase: WheelPhase,
    },
    /// What the platform told of its window, in order with the input.
    Told(Told),
}

/// The platform thread's state for routing: where the pointer is and which
/// modifiers are held.
#[derive(Clone, Debug, Default)]
pub(crate) struct Router {
    cursor: (f32, f32),
    modifiers: Modifiers,
    /// The platform told a gesture's beginning and has not told its end:
    /// its moves are phased. Moves with none open (X11's, which winit
    /// tells only as moves) are unphased.
    phased: bool,
    /// What of the wheel's deltas was finer than scroll motion's unit,
    /// carried to the next ([`super::motion::Carry`]): the one place a
    /// pixel amount becomes units, for both routes alike.
    carry: super::motion::Carry,
}

impl Router {
    /// Where the pointer is, as the events routed so far put it.
    pub(crate) fn cursor(&self) -> (f32, f32) {
        self.cursor
    }

    /// The modifiers held, as the events routed so far told them.
    pub(crate) fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// What `event` becomes.
    pub(crate) fn route(&mut self, event: Event) -> Routed {
        let (x, y) = self.cursor;
        Routed::Ui(match event {
            Event::Moved(x, y) => {
                self.cursor = (x, y);
                Input::PointerMoved { x, y }
            }
            Event::Left => Input::PointerLeft,
            Event::Blur => Input::Blur,
            Event::Primary(true) => Input::MouseDown { x, y },
            Event::Primary(false) => Input::MouseUp { x, y },
            Event::ButtonDown(PointerButton::Primary) => Input::MouseDown { x, y },
            Event::ButtonDown(button) => Input::ButtonDown { x, y, button },
            Event::Wheel { delta, phase } => {
                // In whole units of scroll motion, what is finer carried
                // to the next ([`super::motion`]), on both routes alike: an
                // `f32` holds a whole number of units exactly.
                let (delta, precise) = match delta {
                    WheelDelta::Lines(x, y) => ((-x * LINE_SCROLL, -y * LINE_SCROLL), false),
                    WheelDelta::Pixels(x, y) => ((-x, -y), true),
                };
                let delta = super::motion::px2(self.carry.take(delta));
                // A notched wheel's turns have no phases anywhere.
                let phase = match phase {
                    _ if !precise => WheelPhase::None,
                    WheelPhase::Changed if !self.phased => WheelPhase::None,
                    p => p,
                };
                self.phased = matches!(
                    phase,
                    WheelPhase::Touched
                        | WheelPhase::Began
                        | WheelPhase::Changed
                        | WheelPhase::Ended
                        | WheelPhase::MomentumBegan
                        | WheelPhase::Momentum
                );
                return Routed::Wheel {
                    point: self.cursor,
                    delta,
                    precise,
                    phase,
                };
            }
            Event::Modifiers(m) => {
                self.modifiers = m;
                Input::Modifiers(m)
            }
            Event::Key { key, text, repeat } => {
                let ev = KeyEvent {
                    key,
                    text,
                    modifiers: self.modifiers,
                };
                if repeat { Input::KeyRepeat(ev) } else { Input::Key(ev) }
            }
            Event::Preedit(p) => Input::Ime(ImeEvent::Preedit(p)),
            Event::Commit(t) => Input::Ime(ImeEvent::Commit(t)),
            Event::Act(r) => Input::Act(r),
            Event::Told(t) => return Routed::Told(t),
            Event::Touch { id, pen, phase, x, y } => Input::Touch(TouchEvent {
                id,
                kind: if pen { PointerKind::Pen } else { PointerKind::Touch },
                phase: match phase {
                    Phase::Started => TouchPhase::Started,
                    Phase::Moved => TouchPhase::Moved,
                    Phase::Ended => TouchPhase::Ended,
                    Phase::Cancelled => TouchPhase::Cancelled,
                },
                x,
                y,
            }),
        })
    }
}

/// Writes the events to a log as the render thread takes them, with what
/// it did between them ([`Observation`]). The render thread only sends each
/// line down a channel; a thread of its own formats and writes it, line by
/// line, so a crash loses at most the line being written.
pub struct Recorder {
    tx: Sender<(Instant, Line)>,
}

/// A line the recorder writes: an event, an observation, or a grant for a
/// frame presented then.
enum Line {
    Event(Event),
    Observed(Observation),
    Grant(Instant),
    Drawn(u64, Instant),
}

impl Recorder {
    /// Start a log at `path` with `header`, its time 0 now.
    ///
    /// # Errors
    ///
    /// When the file cannot be made or the writing thread cannot start.
    pub fn start(path: &Path, header: &Header) -> std::io::Result<Self> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        file.write_all(header_text(header).as_bytes())?;
        file.flush()?;
        let (tx, rx) = mpsc::channel::<(Instant, Line)>();
        let start = Instant::now();
        std::thread::Builder::new()
            .name("gui input log".to_owned())
            .spawn(move || {
                for (at, line) in rx {
                    let since = |t: Instant| t.saturating_duration_since(start).as_secs_f64();
                    let t = since(at);
                    let line = match line {
                        Line::Event(event) => event_line(t, &event),
                        Line::Observed(what) => observation_line(t, what),
                        Line::Grant(present) => observation_line(t, Observation::Grant(Some(since(present)))),
                        Line::Drawn(frame, present) => {
                            observation_line(t, Observation::Drawn(frame, Some(since(present))))
                        }
                    };
                    if writeln!(file, "{line}").and_then(|()| file.flush()).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self { tx })
    }

    /// `event` arrived at `at`.
    pub fn record(&self, at: Instant, event: &Event) {
        self.tx.send((at, Line::Event(event.clone()))).ok();
    }

    /// The render thread did `what` now, after the events it recorded.
    pub fn observe(&self, what: Observation) {
        self.observe_at(Instant::now(), what);
    }

    /// The render thread did `what` at `at`: the time it did it by (a
    /// gesture settled to then), which a replay does it at.
    pub fn observe_at(&self, at: Instant, what: Observation) {
        self.tx.send((at, Line::Observed(what))).ok();
    }

    /// The render thread granted a main frame now, to be presented at
    /// `present` ([`Observation::Grant`]).
    pub fn grant(&self, present: Instant) {
        self.tx.send((Instant::now(), Line::Grant(present))).ok();
    }

    /// The render thread drew frame `frame` now, for its display at
    /// `present` ([`Observation::Drawn`]).
    pub fn drawn(&self, frame: u64, present: Instant) {
        self.tx.send((Instant::now(), Line::Drawn(frame, present))).ok();
    }
}

/// Where the platform's input goes, and the one log every path writes it
/// to (ADR-0033), held under one lock ([`super::thread::Forwarding`]): the
/// render thread takes it in while it can scroll, else the UI straight. The
/// event is logged where it is taken, on either path, in the lock's order,
/// with the frame it was routed in when that changed.
#[derive(Debug)]
pub(crate) struct Route {
    /// Input goes through the render thread.
    pub(crate) through: bool,
    /// The render thread applied a commit: it routes wheels in a scene.
    pub(crate) committed: bool,
    recorder: Option<Recorder>,
    /// The frame the last event logged was routed in.
    seen: Option<Option<u64>>,
    /// A wheel went to the UI since input was last diverted
    /// ([`Self::took_wheel`]): a fling the compositor handed on stops as
    /// input comes through again, as a new gesture stops it there.
    wheeled: bool,
}

impl std::fmt::Debug for Recorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Recorder")
    }
}

impl Route {
    /// Input through the render thread, logged nowhere yet.
    pub(crate) fn new() -> Self {
        Self {
            through: true,
            committed: false,
            recorder: None,
            seen: None,
            wheeled: false,
        }
    }

    /// A wheel so phased went straight to the UI, input diverted: one of
    /// another gesture stops a fling handed on
    /// ([`super::WheelPhase::stops_a_fling`]).
    pub(crate) fn took_wheel(&mut self, precise: bool, phase: super::WheelPhase) {
        self.wheeled |= phase.stops_a_fling(precise);
    }

    /// Log from now on with `recorder`.
    pub(crate) fn record_with(&mut self, recorder: Recorder) {
        // A log begun with input going straight to the UI says so first,
        // and one begun before the compositor had a commit to route a
        // wheel in (it scrolls none until then).
        if !self.through {
            recorder.observe(Observation::Diverted);
        }
        if !self.committed {
            recorder.observe(Observation::Uncommitted);
        }
        self.recorder = Some(recorder);
        self.seen = None;
    }

    /// `event`, taken at `at`, routed in `seen`: logged, after the frame
    /// when it is another than the last event's.
    pub(crate) fn record(&mut self, at: Instant, event: &Event, seen: super::Seen) {
        let Some(r) = &self.recorder else { return };
        let seen = match seen {
            super::Seen::Frame(n) => Some(n),
            super::Seen::BeforeFirst => None,
        };
        if self.seen != Some(seen) {
            r.observe(Observation::Seen(seen));
            self.seen = Some(seen);
        }
        r.record(at, event);
    }

    /// The render thread did `what` now.
    pub(crate) fn observe(&self, what: Observation) {
        if let Some(r) = &self.recorder {
            r.observe(what);
        }
    }

    /// The render thread did `what` by the time `at` ([`Recorder::observe_at`]).
    pub(crate) fn observe_at(&self, at: Instant, what: Observation) {
        if let Some(r) = &self.recorder {
            r.observe_at(at, what);
        }
    }

    /// The render thread granted a main frame for `present` now.
    pub(crate) fn grant(&self, present: Instant) {
        if let Some(r) = &self.recorder {
            r.grant(present);
        }
    }

    /// The render thread drew frame `frame` for `present` now.
    pub(crate) fn drawn(&self, frame: u64, present: Instant) {
        if let Some(r) = &self.recorder {
            r.drawn(frame, present);
        }
    }

    /// Input goes straight to the UI from now on, logged so: `hand_on`
    /// first hands the UI what the render thread held (logging the input
    /// it drains), after the log's `diverted`, as a replay does it there.
    /// `at`: the time what the render thread held is settled to, the
    /// `diverted` line's.
    pub(crate) fn divert(&mut self, at: Instant, hand_on: impl FnOnce(&mut Self)) {
        self.observe_at(at, Observation::Diverted);
        self.wheeled = false;
        hand_on(self);
        self.through = false;
    }

    /// Input goes through the render thread again, logged so: whether a
    /// wheel went to the UI meanwhile ([`Self::took_wheel`]).
    pub(crate) fn restore(&mut self) -> bool {
        if !self.through {
            self.through = true;
            self.observe(Observation::Restored);
        }
        std::mem::take(&mut self.wheeled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_reads_back_as_written() {
        let log = InputLog {
            header: Header {
                platform: Platform::Wayland,
                size: (640, 480),
                scale: 1.5,
                source: "a test".to_owned(),
            },
            events: vec![
                Stamped {
                    at: 0.0,
                    event: Event::Moved(10.5, 20.25),
                },
                Stamped {
                    at: 0.0125,
                    event: Event::Wheel {
                        delta: WheelDelta::Pixels(0.0, -3.75),
                        phase: WheelPhase::Began,
                    },
                },
                Stamped {
                    at: 0.02,
                    event: Event::Wheel {
                        delta: WheelDelta::Lines(-1.0, 0.0),
                        phase: WheelPhase::None,
                    },
                },
                Stamped {
                    at: 0.021,
                    event: Event::Wheel {
                        delta: WheelDelta::Pixels(0.0, -1.5),
                        phase: WheelPhase::MomentumBegan,
                    },
                },
                Stamped {
                    at: 0.03,
                    event: Event::Modifiers(Modifiers {
                        shift: true,
                        command: true,
                        ..Modifiers::default()
                    }),
                },
                Stamped {
                    at: 0.031,
                    event: Event::Key {
                        key: Key::Named(NamedKey::Enter),
                        text: Some("\r".to_owned()),
                        repeat: false,
                    },
                },
                Stamped {
                    at: 0.032,
                    event: Event::Key {
                        key: Key::Character("\"é\"".into()),
                        text: Some("\"é\"\\".to_owned()),
                        repeat: true,
                    },
                },
                Stamped {
                    at: 0.033,
                    event: Event::Preedit(Preedit {
                        text: "viê".to_owned(),
                        cursor: Some((4, 4)),
                    }),
                },
                Stamped {
                    at: 0.034,
                    event: Event::Commit("việt nam".to_owned()),
                },
                Stamped {
                    at: 0.035,
                    event: Event::Primary(true),
                },
                Stamped {
                    at: 0.036,
                    event: Event::Primary(false),
                },
                Stamped {
                    at: 0.037,
                    event: Event::ButtonDown(PointerButton::Secondary),
                },
                Stamped {
                    at: 0.038,
                    event: Event::ButtonDown(PointerButton::Middle),
                },
                Stamped {
                    at: 0.039,
                    event: Event::ButtonDown(PointerButton::Other(3)),
                },
                Stamped {
                    at: 0.040,
                    event: Event::Left,
                },
                Stamped {
                    at: 0.041,
                    event: Event::Blur,
                },
                Stamped {
                    at: 0.042,
                    event: Event::Act(accesskit::ActionRequest {
                        action: accesskit::Action::SetValue,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: accesskit::NodeId(42),
                        data: Some(accesskit::ActionData::Value("a \"b\"".into())),
                    }),
                },
                Stamped {
                    at: 0.043,
                    event: Event::Act(accesskit::ActionRequest {
                        action: accesskit::Action::SetTextSelection,
                        target_tree: accesskit::TreeId(accesskit::Uuid::from_u128(0xabc)),
                        target_node: accesskit::NodeId(7),
                        data: Some(accesskit::ActionData::SetTextSelection(accesskit::TextSelection {
                            anchor: accesskit::TextPosition {
                                node: accesskit::NodeId(8),
                                character_index: 1,
                            },
                            focus: accesskit::TextPosition {
                                node: accesskit::NodeId(9),
                                character_index: 3,
                            },
                        })),
                    }),
                },
                Stamped {
                    at: 0.044,
                    event: Event::Act(accesskit::ActionRequest {
                        action: accesskit::Action::Click,
                        target_tree: accesskit::TreeId::ROOT,
                        target_node: accesskit::NodeId(1),
                        data: None,
                    }),
                },
            ],
            observed: [
                (0, Observation::Seen(None)),
                (0, Observation::Grant(Some(0.016_5))),
                (1, Observation::Drawn(3, Some(0.033))),
                (1, Observation::Drawn(4, None)),
                (1, Observation::Diverted),
                (1, Observation::Restored),
                (1, Observation::Uncommitted),
                (1, Observation::Settled),
                (1, Observation::Mail(0xfedc_ba98_7654_3210, 0)),
                (1, Observation::Mail(1, 7)),
                (1, Observation::Seen(Some(3))),
                (2, Observation::Committed),
                (5, Observation::Grant(None)),
                (6, Observation::Aborted),
                (16, Observation::Grant(Some(0.25))),
                (16, Observation::Committed),
            ]
            .into_iter()
            .map(|(after, what)| Observed { after, at: 0.05, what })
            .collect(),
        };
        let text = log.write();
        assert_eq!(InputLog::parse(&text), Ok(log), "{text}");
    }

    #[test]
    fn a_wheel_routes_as_the_window_routed_it() {
        let mut r = Router::default();
        r.route(Event::Moved(5.0, 6.0));
        assert_eq!(
            r.route(Event::Wheel {
                delta: WheelDelta::Lines(0.0, -1.0),
                phase: WheelPhase::Changed,
            }),
            Routed::Wheel {
                point: (5.0, 6.0),
                delta: (0.0, LINE_SCROLL),
                precise: false,
                phase: WheelPhase::None,
            },
            "a notched wheel's turn has no phase"
        );
        // X11's trackpad: moves only, never a beginning: unphased.
        let pixels = |phase| Event::Wheel {
            delta: WheelDelta::Pixels(2.0, -8.0),
            phase,
        };
        let routed = |phase| Routed::Wheel {
            point: (5.0, 6.0),
            delta: (-2.0, 8.0),
            precise: true,
            phase,
        };
        assert_eq!(r.route(pixels(WheelPhase::Changed)), routed(WheelPhase::None));
        // Wayland's: a beginning, moves, an end.
        assert_eq!(r.route(pixels(WheelPhase::Began)), routed(WheelPhase::Began));
        assert_eq!(r.route(pixels(WheelPhase::Changed)), routed(WheelPhase::Changed));
        assert_eq!(r.route(pixels(WheelPhase::Ended)), routed(WheelPhase::Ended));
        assert_eq!(
            r.route(Event::Primary(true)),
            Routed::Ui(Input::MouseDown { x: 5.0, y: 6.0 })
        );
    }

    /// `event` written to a log and read back.
    fn round_trip(event: Event) -> Event {
        let log = InputLog {
            header: Header {
                platform: Platform::Other,
                size: (64, 64),
                scale: 1.0,
                source: "keys".to_owned(),
            },
            events: vec![Stamped { at: 0.0, event }],
            observed: Vec::new(),
        };
        let text = log.write();
        match InputLog::parse(&text) {
            Ok(mut back) if back.events.len() == 1 => back.events.remove(0).event,
            other => panic!("{text} read back as {other:?}"),
        }
    }

    /// Codex r11 4: a key the app reads (`Redo`, a shortcut) is the same
    /// key read back from a log, not `Unidentified`.
    #[test]
    fn a_redo_key_survives_a_log() {
        let redo = Event::Key {
            key: Key::Named(NamedKey::Redo),
            text: None,
            repeat: false,
        };
        assert_eq!(round_trip(redo.clone()), redo);
    }

    /// Every key winit can give reads back from a log as itself: each of
    /// its named keys (the list is winit's whole enum: as many as it has,
    /// each once, each its own name), a character, a dead key and every
    /// platform code of a key it could not name.
    #[test]
    fn every_key_survives_a_log() {
        let mut names: Vec<String> = NAMED_KEYS.iter().map(|k| format!("{k:?}")).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 306, "winit 0.30's named keys, each once");
        // Codex r12 3: a dead key's character is any character, those the
        // encoding itself uses included ('-' was read as no character).
        let edge = ['-', '"', '\\', ' ', 'n', '\n', '\u{0}', '`', '¨', '😀'];
        let dead = edge.iter().map(|c| Key::Dead(Some(*c)));
        let chars = [
            "-",
            "\"",
            "\\",
            " ",
            "a b",
            "\"quoted\"",
            "\\n",
            "ab",
            "",
            "none",
            "😀👍",
        ]
        .into_iter()
        .map(|c| Key::Character(c.into()));
        let others = [
            Key::Character("é".into()),
            Key::Character("\"".into()),
            Key::Dead(Some('`')),
            Key::Dead(None),
            Key::Unidentified(NativeKey::Web(String::new().into())),
            Key::Unidentified(NativeKey::Web("-".into())),
            Key::Unidentified(NativeKey::Unidentified),
            Key::Unidentified(NativeKey::Android(7)),
            Key::Unidentified(NativeKey::MacOS(63)),
            Key::Unidentified(NativeKey::Windows(255)),
            Key::Unidentified(NativeKey::Xkb(0x1008_ff13)),
            Key::Unidentified(NativeKey::Web("Dead Key".into())),
        ];
        for key in NAMED_KEYS
            .iter()
            .map(|k| Key::Named(*k))
            .chain(others)
            .chain(dead)
            .chain(chars)
        {
            for (text, repeat) in [(None, false), (Some("x".to_owned()), true)] {
                let event = Event::Key {
                    key: key.clone(),
                    text,
                    repeat,
                };
                assert_eq!(round_trip(event.clone()), event);
            }
        }
    }
}
