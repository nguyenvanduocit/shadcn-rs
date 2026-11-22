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
//! gui-input-log 1
//! platform macos
//! window 480 320 2
//! source synthesized from ...
//! 0.000 move 150 160
//! 12.500 wheel pixels 0 -3 started
//! 20.833 wheel pixels 0 -7.5 moved
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
//!   began, then its kind:
//!   - `move <x> <y>`: the pointer, logical window pixels; `leave`;
//!   - `down`, `up` (the primary button), `secondary` (the secondary
//!     button went down), at the last pointer position;
//!   - `wheel lines|pixels <dx> <dy> started|moved|ended|cancelled`: a
//!     line delta (a notched wheel) or a pixel delta in logical pixels (a
//!     trackpad's), in winit's sign (positive `dy`: the content moves
//!     down), with winit's touch phase;
//!   - `mods <shift+ctrl+alt+cmd>` or `mods none`;
//!   - `key named <Name> [text]`, `key char <text> [text]`, `key other
//!     <what>`: a key pressed, its logical key and the text it types;
//!   - `preedit <text> <start> <end>` or `preedit <text> -`, `commit <text>`:
//!     the input method.
//!
//!   Texts are quoted, with `\\`, `\"`, `\n`, `\r`, `\t` and `\u{…}`
//!   escaped.

use std::io::Write as _;
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::time::Instant;

use winit::keyboard::{Key, NamedKey, NativeKey};

use crate::edit::Preedit;
use crate::events::{ImeEvent, KeyEvent, Modifiers};
use crate::interact::LINE_SCROLL;
use crate::window::Input;

/// The log format's version, the header's first line.
const MAGIC: &str = "gui-input-log 1";

/// What the platform thread got, as it got it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The pointer moved to `(x, y)`, logical window pixels.
    Moved(f32, f32),
    Left,
    /// The primary button went down (`true`) or up, where the pointer is.
    Primary(bool),
    /// The secondary button went down, where the pointer is.
    SecondaryDown,
    /// The wheel or a trackpad scrolled.
    Wheel {
        delta: WheelDelta,
        phase: Phase,
    },
    Modifiers(Modifiers),
    /// A key went down: its logical key and the text it types.
    Key {
        key: Key,
        text: Option<String>,
    },
    /// The input method composes.
    Preedit(Preedit),
    /// The input method commits text.
    Commit(String),
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

/// A log of input.
#[derive(Clone, Debug, PartialEq)]
pub struct InputLog {
    pub header: Header,
    pub events: Vec<Stamped>,
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
        for (n, line) in lines {
            let fail = |why: &str| format!("line {n}: {why}: {line:?}");
            let (head, rest) = line.split_once(' ').unwrap_or((line, ""));
            if let Ok(ms) = head.parse::<f64>() {
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
        Ok(Self { header, events })
    }

    /// The log as text, which [`Self::parse`] reads back the same.
    #[must_use]
    pub fn write(&self) -> String {
        let mut out = header_text(&self.header);
        for e in &self.events {
            out.push_str(&event_line(e.at, &e.event));
            out.push('\n');
        }
        out
    }
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
    let mut s = format!("{:.3} ", at * 1000.0);
    match event {
        Event::Moved(x, y) => {
            s.push_str(&format!("move {x} {y}"));
        }
        Event::Left => s.push_str("leave"),
        Event::Primary(true) => s.push_str("down"),
        Event::Primary(false) => s.push_str("up"),
        Event::SecondaryDown => s.push_str("secondary"),
        Event::Wheel { delta, phase } => {
            let (kind, x, y) = match delta {
                WheelDelta::Lines(x, y) => ("lines", x, y),
                WheelDelta::Pixels(x, y) => ("pixels", x, y),
            };
            let phase = match phase {
                Phase::Started => "started",
                Phase::Moved => "moved",
                Phase::Ended => "ended",
                Phase::Cancelled => "cancelled",
            };
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
        Event::Key { key, text } => {
            match key {
                Key::Named(n) => match NAMED.iter().find(|(k, _)| k == n) {
                    Some((_, name)) => {
                        s.push_str(&format!("key named {name}"));
                    }
                    None => {
                        s.push_str(&format!("key other {}", quote(&format!("{n:?}"))));
                    }
                },
                Key::Character(c) => {
                    s.push_str(&format!("key char {}", quote(c)));
                }
                other => {
                    s.push_str(&format!("key other {}", quote(&format!("{other:?}"))));
                }
            }
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
    }
    s
}

fn parse_event(rest: &str) -> Result<Event, String> {
    let words = tokens(rest)?;
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let num = |s: &str| s.parse::<f32>().map_err(|_| format!("not a number: {s}"));
    Ok(match w.as_slice() {
        ["move", x, y] => Event::Moved(num(x)?, num(y)?),
        ["leave"] => Event::Left,
        ["down"] => Event::Primary(true),
        ["up"] => Event::Primary(false),
        ["secondary"] => Event::SecondaryDown,
        ["wheel", kind, x, y, phase] => {
            let (x, y) = (num(x)?, num(y)?);
            let delta = match *kind {
                "lines" => WheelDelta::Lines(x, y),
                "pixels" => WheelDelta::Pixels(x, y),
                k => return Err(format!("a wheel delta is lines or pixels, not {k}")),
            };
            let phase = match *phase {
                "started" => Phase::Started,
                "moved" => Phase::Moved,
                "ended" => Phase::Ended,
                "cancelled" => Phase::Cancelled,
                p => return Err(format!("unknown phase {p}")),
            };
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
        ["key", kind, what, text @ ..] => {
            let key = match *kind {
                "named" => NAMED
                    .iter()
                    .find(|(_, n)| n == what)
                    .map(|(k, _)| Key::Named(*k))
                    .ok_or_else(|| format!("unknown key {what}"))?,
                "char" => Key::Character((*what).into()),
                "other" => Key::Unidentified(NativeKey::Unidentified),
                k => return Err(format!("a key is named, char or other, not {k}")),
            };
            let text = match text {
                [] => None,
                [t] => Some((*t).to_owned()),
                _ => return Err("a key types one text".to_owned()),
            };
            Event::Key { key, text }
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
        _ => return Err("unknown event".to_owned()),
    })
}

/// The named keys a log names (the ones `gui` and its components read, and
/// the modifier keys); any other is logged as `other`.
const NAMED: [(NamedKey, &str); 38] = [
    (NamedKey::Enter, "Enter"),
    (NamedKey::Tab, "Tab"),
    (NamedKey::Space, "Space"),
    (NamedKey::Escape, "Escape"),
    (NamedKey::Backspace, "Backspace"),
    (NamedKey::Delete, "Delete"),
    (NamedKey::Insert, "Insert"),
    (NamedKey::ArrowUp, "ArrowUp"),
    (NamedKey::ArrowDown, "ArrowDown"),
    (NamedKey::ArrowLeft, "ArrowLeft"),
    (NamedKey::ArrowRight, "ArrowRight"),
    (NamedKey::Home, "Home"),
    (NamedKey::End, "End"),
    (NamedKey::PageUp, "PageUp"),
    (NamedKey::PageDown, "PageDown"),
    (NamedKey::Shift, "Shift"),
    (NamedKey::Control, "Control"),
    (NamedKey::Alt, "Alt"),
    (NamedKey::Super, "Super"),
    (NamedKey::Meta, "Meta"),
    (NamedKey::CapsLock, "CapsLock"),
    (NamedKey::ContextMenu, "ContextMenu"),
    (NamedKey::F1, "F1"),
    (NamedKey::F2, "F2"),
    (NamedKey::F3, "F3"),
    (NamedKey::F4, "F4"),
    (NamedKey::F5, "F5"),
    (NamedKey::F6, "F6"),
    (NamedKey::F7, "F7"),
    (NamedKey::F8, "F8"),
    (NamedKey::F9, "F9"),
    (NamedKey::F10, "F10"),
    (NamedKey::F11, "F11"),
    (NamedKey::F12, "F12"),
    (NamedKey::Copy, "Copy"),
    (NamedKey::Cut, "Cut"),
    (NamedKey::Paste, "Paste"),
    (NamedKey::Undo, "Undo"),
];

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
/// what is not input (or input `gui` takes no notice of: other buttons, a
/// secondary release, key releases).
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
        WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Right,
            ..
        } => Event::SecondaryDown,
        WindowEvent::MouseWheel { delta, phase, .. } => Event::Wheel {
            delta: match delta {
                MouseScrollDelta::LineDelta(x, y) => WheelDelta::Lines(x, y),
                MouseScrollDelta::PixelDelta(p) => WheelDelta::Pixels(f64_to_f32(p.x) / scale, f64_to_f32(p.y) / scale),
            },
            phase: phase.into(),
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
        },
        WindowEvent::Ime(Ime::Preedit(text, cursor)) => Event::Preedit(Preedit { text, cursor }),
        WindowEvent::Ime(Ime::Commit(text)) => Event::Commit(text),
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
    /// resampled, a notched wheel's applied; `ended`: the fingers lifted.
    Wheel {
        point: (f32, f32),
        delta: (f32, f32),
        precise: bool,
        ended: bool,
    },
}

/// The platform thread's state for routing: where the pointer is and which
/// modifiers are held.
#[derive(Clone, Debug, Default)]
pub(crate) struct Router {
    cursor: (f32, f32),
    modifiers: Modifiers,
}

impl Router {
    /// What `event` becomes.
    pub(crate) fn route(&mut self, event: Event) -> Routed {
        let (x, y) = self.cursor;
        Routed::Ui(match event {
            Event::Moved(x, y) => {
                self.cursor = (x, y);
                Input::PointerMoved { x, y }
            }
            Event::Left => Input::PointerLeft,
            Event::Primary(true) => Input::MouseDown { x, y },
            Event::Primary(false) => Input::MouseUp { x, y },
            Event::SecondaryDown => Input::SecondaryDown { x, y },
            Event::Wheel { delta, phase } => {
                let (delta, precise) = match delta {
                    WheelDelta::Lines(x, y) => ((-x * LINE_SCROLL, -y * LINE_SCROLL), false),
                    WheelDelta::Pixels(x, y) => ((-x, -y), true),
                };
                return Routed::Wheel {
                    point: self.cursor,
                    delta,
                    precise,
                    ended: matches!(phase, Phase::Ended | Phase::Cancelled),
                };
            }
            Event::Modifiers(m) => {
                self.modifiers = m;
                Input::Modifiers(m)
            }
            Event::Key { key, text } => Input::Key(KeyEvent {
                key,
                text,
                modifiers: self.modifiers,
            }),
            Event::Preedit(p) => Input::Ime(ImeEvent::Preedit(p)),
            Event::Commit(t) => Input::Ime(ImeEvent::Commit(t)),
        })
    }
}

/// Writes the platform thread's events to a log as they arrive. The
/// platform thread only sends each one down a channel; a thread of its own
/// formats and writes it, line by line, so a crash loses at most the line
/// being written.
pub struct Recorder {
    tx: Sender<(Instant, Event)>,
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
        let (tx, rx) = mpsc::channel::<(Instant, Event)>();
        let start = Instant::now();
        std::thread::Builder::new()
            .name("gui input log".to_owned())
            .spawn(move || {
                for (at, event) in rx {
                    let t = at.saturating_duration_since(start).as_secs_f64();
                    let line = event_line(t, &event);
                    if writeln!(file, "{line}").and_then(|()| file.flush()).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self { tx })
    }

    /// `event` arrived at `at`.
    pub fn record(&self, at: Instant, event: &Event) {
        self.tx.send((at, event.clone())).ok();
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
                        phase: Phase::Started,
                    },
                },
                Stamped {
                    at: 0.02,
                    event: Event::Wheel {
                        delta: WheelDelta::Lines(-1.0, 0.0),
                        phase: Phase::Moved,
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
                    },
                },
                Stamped {
                    at: 0.032,
                    event: Event::Key {
                        key: Key::Character("\"é\"".into()),
                        text: Some("\"é\"\\".to_owned()),
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
                    event: Event::SecondaryDown,
                },
                Stamped {
                    at: 0.038,
                    event: Event::Left,
                },
            ],
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
                phase: Phase::Moved,
            }),
            Routed::Wheel {
                point: (5.0, 6.0),
                delta: (0.0, LINE_SCROLL),
                precise: false,
                ended: false,
            }
        );
        assert_eq!(
            r.route(Event::Wheel {
                delta: WheelDelta::Pixels(2.0, -8.0),
                phase: Phase::Ended,
            }),
            Routed::Wheel {
                point: (5.0, 6.0),
                delta: (-2.0, 8.0),
                precise: true,
                ended: true,
            }
        );
        assert_eq!(
            r.route(Event::Primary(true)),
            Routed::Ui(Input::MouseDown { x: 5.0, y: 6.0 })
        );
    }
}
