//! Devtools on the command line: one vocabulary of commands that reads
//! everything a frame holds (the layout tree, the components, what a screen
//! reader hears, the labeled signals and their values), measures it (frame
//! times, memory, CPU) and acts on it (click, type, press, scroll, write a
//! signal), so a person or an agent drives an app as text.
//!
//! The same commands run two ways:
//! - **headless**: a [`Session`] over a [`Scenario`], fed lines from stdin or
//!   arguments ([`Session::run_script`]); time passes only when told to;
//! - **attached**: a live window started with [`crate::DebugTools::attach`] (or
//!   `GUI_DEVTOOLS=1`) listens on a local socket ([`socket_path`]), and
//!   [`request`] sends it lines; each runs on the UI thread between frames.
//!
//! `help` lists every command. Output is plain text, one fact per line, with
//! nodes named by their path (`@0/1/k3`) so the next command can reuse it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use accesskit::{Action, ActionData, ActionRequest, NodeId, TreeId};

use crate::access;
use crate::app::{Pipeline, Scenario, ScenarioError};
use crate::element::Element;
use crate::events::{Key, KeyEvent, Modifiers, NamedKey};
use crate::focus;
use crate::heap;
use crate::inspect::{self, Footprint, FrameReport, LayoutDiff, Spread, bytes, ms};
use crate::interact::{self, Outcome};
use crate::layout::{Content, Frame};
use crate::process;
use crate::reactive;
use crate::window::Input;

/// How many frame reports the log keeps.
pub const LOG_FRAMES: usize = 120;

/// The commands, as `help` prints them.
pub const HELP: &str = "\
Selectors (SEL): @PATH (a node's path, e.g. @0/2/k7), #NAME or #NAME:N (the Nth node
  named NAME, from 1), X,Y (the node under a point), or TEXT (a node showing exactly
  TEXT, else the first whose text, value, placeholder or label contains it, any case).
  Quote text with spaces: click \"Save changes\".

Inspect
  tree [SEL] [--depth=N]     the layout tree under SEL: path, what it is, box, behaviour
  components [SEL]           only the named nodes (shadcn's components), nested
  node SEL                   everything about one node: box, look, text, behaviour, role, ancestry
  find TEXT                  every node whose text, value, placeholder or label contains TEXT
  at X Y                     the node under a point and the nodes it sits in
  a11y [SEL]                 what a screen reader hears: roles, names, values, states, actions
  focus                      the focused node
  signals [FILTER]           the labeled signals: id, label, type, value, readers
  signal ID|LABEL            one labeled signal's whole value
  set ID|LABEL VALUE         write a labeled signal (numbers, bool, char, String)
  graph                      the reactive graph: signals, computed values, effects, subscriptions

Measure
  report [N]                 the last N frame reports (cause, signals, times, layout change, holds)
  perf                       frame times over the logged frames: total/build/layout/paint p50/p95/max
  memory                     the process (resident, CPU), the heap, the caches, the graph
  diff                       every node the last changing frame added, removed, moved or repainted
  bench [N]                  (headless) draw N frames of the page as it is and time them (default 60)
  monkey [N] [--seed=S] [--budget=MS] [--verbose]
                             (headless) N random clicks, keys, typing, scrolls and waits (default 200),
                             seeded so a run repeats; reports panics, failed layouts, slow frames and
                             what the heap kept, with the steps that led there

Act (each draws the next frame and prints its report)
  click SEL                  press and release at the node's centre
  rclick SEL                 the secondary button
  hover SEL | leave          move the pointer onto a node, or out of the window
  reveal SEL                 scroll the node's scroll containers until it is in view
                             (click, rclick, hover and drag do this first, as a browser
                             automation tool does)
  drag SEL SEL               press on one, move to the other, release
  wheel DY [SEL]             scroll by DY pixels (positive scrolls down), over SEL if given
  type TEXT                  type TEXT into the focused node, key by key
  press KEYS...              keys such as enter, esc, tab, shift+tab, down, cmd+k, ctrl+a, space
  activate SEL               a screen reader's press on the node (no pointer)
  focus SEL                  a screen reader's focus on the node
  setvalue SEL TEXT          a screen reader's value for a text input

Frames and time
  frame                      draw a frame and print its report
  screenshot PATH [--overlay]  write the next frame as PNG (with the debug overlay)
  wait MS                    (headless) let MS milliseconds pass, timers and motion included
  rest                       (headless) run due timers and motion until the page is still
  size W H                   (headless) resize the window to W x H logical pixels

  help                       this text
";

// ── Frame log ────────────────────────────────────────────────────────────────

/// The last frames' reports, each with when it was drawn.
#[derive(Default)]
pub struct FrameLog {
    frames: VecDeque<(Instant, FrameReport)>,
}

impl FrameLog {
    pub fn push(&mut self, report: FrameReport) {
        if self.frames.len() >= LOG_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back((Instant::now(), report));
    }

    pub fn last(&self) -> Option<&FrameReport> {
        self.frames.back().map(|(_, r)| r)
    }

    /// The logged reports, oldest first.
    pub fn reports(&self) -> impl DoubleEndedIterator<Item = &FrameReport> + ExactSizeIterator {
        self.frames.iter().map(|(_, r)| r)
    }

    /// Frames drawn in the last second (a live window's rate).
    fn fps(&self) -> usize {
        let second = Duration::from_secs(1);
        self.frames.iter().filter(|(at, _)| at.elapsed() < second).count()
    }
}

// ── What a command needs of the app ──────────────────────────────────────────

/// What the commands read and drive: a pipeline, live or headless.
pub(crate) trait Target {
    fn frame(&self) -> &Frame;
    fn send(&mut self, input: Input) -> Outcome;
    fn footprint(&self) -> Footprint;
    fn last_diff(&self) -> &LayoutDiff;
}

impl<C: FnMut() -> Element> Target for Pipeline<C> {
    fn frame(&self) -> &Frame {
        Pipeline::frame(self)
    }

    fn send(&mut self, input: Input) -> Outcome {
        self.input(input)
    }

    fn footprint(&self) -> Footprint {
        Pipeline::footprint(self)
    }

    fn last_diff(&self) -> &LayoutDiff {
        Pipeline::last_diff(self)
    }
}

/// What a command leaves to whoever runs it.
#[derive(Debug, PartialEq)]
pub(crate) enum Done {
    /// The whole answer.
    Text(String),
    /// Input went in: draw the next frame and add its report after this.
    Acted(String),
    /// Draw a frame and answer with its report.
    Frame,
    /// Write the next frame to `path`.
    Screenshot { path: PathBuf, overlay: bool },
    /// A command only a [`Session`] runs (time, size, bench).
    Headless(Vec<String>),
}

/// Split a command line into words: spaces separate them, quotes ("…" or '…')
/// keep spaces in one, a backslash escapes the next character.
pub fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                let next = chars.next().ok_or("a line ends in a backslash")?;
                word.get_or_insert_with(String::new).push(next);
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => word.get_or_insert_with(String::new).push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                word.get_or_insert_with(String::new);
            }
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            (None, c) => word.get_or_insert_with(String::new).push(c),
        }
    }
    if quote.is_some() {
        return Err("a quote is not closed".to_owned());
    }
    words.extend(word);
    Ok(words)
}

/// Run one command (already split) against `target`.
pub(crate) fn execute(target: &mut dyn Target, log: &FrameLog, words: &[String]) -> Result<Done, String> {
    let Some((verb, args)) = words.split_first() else {
        return Ok(Done::Text(String::new()));
    };
    let arg = |n: usize, what: &str| args.get(n).map(String::as_str).ok_or(format!("{verb}: needs {what}"));
    let text = |s: String| Ok(Done::Text(s));
    match verb.as_str() {
        "help" | "?" => text(HELP.to_owned()),
        "tree" => {
            let (flags, rest) = flags(args);
            let depth = flag_num(&flags, "depth")?;
            let root = rest.first().map(|s| node_of(target.frame(), s)).transpose()?;
            text(tree(target.frame(), root, depth))
        }
        "components" => {
            let root = args.first().map(|s| node_of(target.frame(), s)).transpose()?;
            text(components(target.frame(), root))
        }
        "node" => {
            let i = node_of(target.frame(), arg(0, "a node (SEL)")?)?;
            text(node_detail(target.frame(), i))
        }
        "find" => text(find(target.frame(), &args.join(" "))),
        "at" => {
            let x = num(arg(0, "X Y")?)?;
            let y = num(arg(1, "X Y")?)?;
            text(at(target.frame(), x, y))
        }
        "a11y" | "access" => {
            let root = args.first().map(|s| node_of(target.frame(), s)).transpose()?;
            text(a11y(target.frame(), root))
        }
        "focus" if args.is_empty() => text(focused(target.frame())),
        "signals" => text(signals(args.first().map(String::as_str))),
        "signal" => text(signal(arg(0, "a signal id or label")?)?),
        "set" => {
            let which = arg(0, "a signal id or label")?;
            let value = args.get(1..).map(|v| v.join(" ")).filter(|v| !v.is_empty());
            let value = value.ok_or("set: needs a value")?;
            let id = probe_id(which)?;
            reactive::write_probe(id, &value)?;
            Ok(Done::Acted(format!("set signal #{id} to {value}")))
        }
        "graph" => {
            let g = reactive::graph();
            text(format!(
                "signals {}  computed {}  effects {}  subscriptions {}",
                g.signals, g.computeds, g.effects, g.subscriptions
            ))
        }
        "report" => {
            let n = args.first().map(|s| count(s)).transpose()?.unwrap_or(1);
            text(reports(log, n))
        }
        "perf" => text(perf(log)),
        "memory" | "mem" => text(memory(&target.footprint(), target.frame())),
        "diff" => text(diff(target.last_diff())),
        "click" | "rclick" => {
            let sel = arg(0, "a node (SEL)")?;
            let scrolled = reveal(target, sel)?;
            let (x, y, about) = point_of(target.frame(), sel)?;
            let about = format!("{scrolled}{about}");
            target.send(Input::PointerMoved { x, y });
            let out = if verb == "click" {
                target.send(Input::MouseDown { x, y });
                target.send(Input::MouseUp { x, y })
            } else {
                target.send(Input::SecondaryDown { x, y })
            };
            Ok(Done::Acted(format!("{about}\n{}", handled(&out))))
        }
        "hover" => {
            let sel = arg(0, "a node (SEL)")?;
            let scrolled = reveal(target, sel)?;
            let (x, y, about) = point_of(target.frame(), sel)?;
            let about = format!("{scrolled}{about}");
            let out = target.send(Input::PointerMoved { x, y });
            Ok(Done::Acted(format!("{about}\n{}", handled(&out))))
        }
        "leave" => {
            let out = target.send(Input::PointerLeft);
            Ok(Done::Acted(handled(&out)))
        }
        "reveal" => {
            let sel = arg(0, "a node (SEL)")?;
            let scrolled = reveal(target, sel)?;
            let i = node_of(target.frame(), sel)?;
            Ok(Done::Acted(format!(
                "{scrolled}{} is in view",
                label(target.frame(), i)
            )))
        }
        "drag" => {
            let from_sel = arg(0, "two nodes (SEL SEL)")?;
            reveal(target, from_sel)?;
            let (x0, y0, from) = point_of(target.frame(), from_sel)?;
            let (x1, y1, to) = point_of(target.frame(), arg(1, "two nodes (SEL SEL)")?)?;
            target.send(Input::PointerMoved { x: x0, y: y0 });
            target.send(Input::MouseDown { x: x0, y: y0 });
            target.send(Input::PointerMoved { x: x1, y: y1 });
            let out = target.send(Input::MouseUp { x: x1, y: y1 });
            Ok(Done::Acted(format!("from {from}\nto {to}\n{}", handled(&out))))
        }
        "wheel" | "scroll" => {
            let dy = num(arg(0, "DY")?)?;
            let mut about = String::new();
            if let Some(sel) = args.get(1) {
                let (x, y, over) = point_of(target.frame(), sel)?;
                target.send(Input::PointerMoved { x, y });
                about = format!("{over}\n");
            }
            let out = target.send(Input::Wheel { dx: 0.0, dy });
            Ok(Done::Acted(format!("{about}{}", handled(&out))))
        }
        "type" => {
            let typed = args.join(" ");
            let mut last = Outcome::default();
            for c in typed.chars() {
                let s = c.to_string();
                let out = target.send(Input::Key(KeyEvent {
                    key: Key::Character(s.as_str().into()),
                    text: Some(s),
                    modifiers: Modifiers::default(),
                }));
                if out.handled_by.is_some() {
                    last = out;
                }
            }
            Ok(Done::Acted(format!("typed {typed:?}\n{}", handled(&last))))
        }
        "press" | "key" => {
            if args.is_empty() {
                return Err("press: needs keys (enter, esc, tab, shift+tab, cmd+k…)".to_owned());
            }
            let mut lines = Vec::new();
            for chord in args {
                let ev = key_event(chord)?;
                lines.push(format!("{chord}: {}", handled(&target.send(Input::Key(ev)))));
            }
            Ok(Done::Acted(lines.join("\n")))
        }
        "activate" | "focus" | "setvalue" => {
            let i = actor(target.frame(), arg(0, "a node (SEL)")?, verb)?;
            let (action, data) = match verb.as_str() {
                "activate" => (Action::Click, None),
                "focus" => (Action::Focus, None),
                _ => {
                    let value = args.get(1..).map(|v| v.join(" ")).unwrap_or_default();
                    (Action::SetValue, Some(ActionData::Value(value.into())))
                }
            };
            let node = target
                .frame()
                .nodes
                .get(i)
                .map(|n| access::id_of(&n.path))
                .unwrap_or(NodeId(0));
            let out = target.send(Input::Act(ActionRequest {
                action,
                target_tree: TreeId::ROOT,
                target_node: node,
                data,
            }));
            if out.handled_by.is_none() {
                return Err(format!(
                    "{verb}: {} does not take it (see `a11y` for each node's actions)",
                    label(target.frame(), i)
                ));
            }
            Ok(Done::Acted(handled(&out)))
        }
        "frame" => Ok(Done::Frame),
        "screenshot" | "shot" => {
            let (flags, rest) = flags(args);
            let path = rest.first().ok_or("screenshot: needs a PATH")?;
            Ok(Done::Screenshot {
                path: PathBuf::from(path),
                overlay: flags.iter().any(|(k, _)| *k == "overlay"),
            })
        }
        "wait" | "rest" | "size" | "bench" | "monkey" => Ok(Done::Headless(words.to_vec())),
        other => Err(format!("unknown command {other:?}; `help` lists them")),
    }
}

// ── Arguments ────────────────────────────────────────────────────────────────

/// `--key=value` and `--flag` words apart from the rest.
fn flags(args: &[String]) -> (Vec<(&str, &str)>, Vec<&String>) {
    let mut flags = Vec::new();
    let mut rest = Vec::new();
    for a in args {
        match a.strip_prefix("--") {
            Some(f) => flags.push(f.split_once('=').unwrap_or((f, ""))),
            None => rest.push(a),
        }
    }
    (flags, rest)
}

fn flag_num(flags: &[(&str, &str)], key: &str) -> Result<Option<usize>, String> {
    flags.iter().find(|(k, _)| *k == key).map(|(_, v)| count(v)).transpose()
}

fn num(s: &str) -> Result<f32, String> {
    s.parse::<f32>().map_err(|_| format!("{s:?} is not a number"))
}

fn count(s: &str) -> Result<usize, String> {
    s.parse::<usize>().map_err(|_| format!("{s:?} is not a count"))
}

/// The key event a chord such as `enter`, `shift+tab` or `cmd+k` names.
fn key_event(chord: &str) -> Result<KeyEvent, String> {
    let mut modifiers = Modifiers::default();
    let parts: Vec<&str> = chord.split('+').collect();
    let (key, mods) = parts.split_last().ok_or("an empty key")?;
    for m in mods {
        match m.to_ascii_lowercase().as_str() {
            "shift" => modifiers.shift = true,
            "ctrl" | "control" => modifiers.control = true,
            "alt" | "option" | "opt" => modifiers.alt = true,
            "cmd" | "command" | "meta" | "super" => modifiers.command = true,
            // The platform's shortcut modifier, as shortcuts are written once for both.
            "mod" | "primary" => {
                if cfg!(target_os = "macos") {
                    modifiers.command = true;
                } else {
                    modifiers.control = true;
                }
            }
            other => return Err(format!("unknown modifier {other:?} in {chord:?}")),
        }
    }
    let named = match key.to_ascii_lowercase().as_str() {
        "enter" | "return" => Some(NamedKey::Enter),
        "esc" | "escape" => Some(NamedKey::Escape),
        "tab" => Some(NamedKey::Tab),
        "space" => Some(NamedKey::Space),
        "backspace" => Some(NamedKey::Backspace),
        "delete" | "del" => Some(NamedKey::Delete),
        "up" | "arrowup" => Some(NamedKey::ArrowUp),
        "down" | "arrowdown" => Some(NamedKey::ArrowDown),
        "left" | "arrowleft" => Some(NamedKey::ArrowLeft),
        "right" | "arrowright" => Some(NamedKey::ArrowRight),
        "home" => Some(NamedKey::Home),
        "end" => Some(NamedKey::End),
        "pageup" => Some(NamedKey::PageUp),
        "pagedown" => Some(NamedKey::PageDown),
        "f1" => Some(NamedKey::F1),
        "f2" => Some(NamedKey::F2),
        "f3" => Some(NamedKey::F3),
        "f4" => Some(NamedKey::F4),
        "f5" => Some(NamedKey::F5),
        "f6" => Some(NamedKey::F6),
        "f7" => Some(NamedKey::F7),
        "f8" => Some(NamedKey::F8),
        "f9" => Some(NamedKey::F9),
        "f11" => Some(NamedKey::F11),
        _ => None,
    };
    if let Some(named) = named {
        let text = (named == NamedKey::Space).then(|| " ".to_owned());
        return Ok(KeyEvent {
            key: Key::Named(named),
            text,
            modifiers,
        });
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(KeyEvent {
            key: Key::Character(c.to_string().as_str().into()),
            text: Some(c.to_string()),
            modifiers,
        }),
        _ => Err(format!(
            "unknown key {key:?} (a character, or enter, esc, tab, space, up…)"
        )),
    }
}

// ── Selectors ────────────────────────────────────────────────────────────────

/// What a selector names: a node, or a point (a link span, `X,Y`).
enum Pick {
    Node(usize),
    Point(f32, f32),
}

fn pick(frame: &Frame, sel: &str) -> Result<Pick, String> {
    if let Some(path) = sel.strip_prefix('@') {
        return frame
            .nodes
            .iter()
            .position(|n| n.path.to_string() == path)
            .map(Pick::Node)
            .ok_or_else(|| format!("no node at @{path} in this frame"));
    }
    if let Some(name) = sel.strip_prefix('#') {
        let (name, nth) = match name.rsplit_once(':') {
            Some((n, k)) => (n, count(k)?.max(1)),
            None => (name, 1),
        };
        let mut named = frame.nodes.iter().enumerate().filter(|(_, n)| n.name == Some(name));
        if let Some((i, _)) = named.nth(nth.saturating_sub(1)) {
            return Ok(Pick::Node(i));
        }
        // No node has that name: the text itself may start with `#` (a button "#17").
        return pick_text(frame, sel)
            .map_err(|_| format!("no node #{name}:{nth} in this frame, nor one showing {sel:?} (see `components`)"));
    }
    if let Some((x, y)) = sel.split_once(',')
        && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
    {
        return Ok(Pick::Point(x, y));
    }
    pick_text(frame, sel)
}

/// The nodes showing `sel` exactly, in paint order, then those containing it
/// (any case): what a text selector may mean.
fn text_matches(frame: &Frame, sel: &str) -> Vec<usize> {
    let shown = |i: &usize| frame.nodes.get(*i).is_some_and(|n| !n.look.invisible);
    let lower = sel.to_lowercase();
    let exact = (0..frame.nodes.len())
        .filter(shown)
        .filter(|&i| texts(frame, i).contains(&sel));
    let within = (0..frame.nodes.len())
        .filter(shown)
        .filter(|&i| !texts(frame, i).contains(&sel))
        .filter(|&i| texts(frame, i).iter().any(|t| t.to_lowercase().contains(&lower)));
    exact.chain(within).collect()
}

/// A text selector: a node showing it exactly, a link span, then a node containing it.
fn pick_text(frame: &Frame, sel: &str) -> Result<Pick, String> {
    let shown = |i: &usize| frame.nodes.get(*i).is_some_and(|n| !n.look.invisible);
    if let Some(i) = (0..frame.nodes.len())
        .filter(shown)
        .find(|&i| texts(frame, i).contains(&sel))
    {
        return Ok(Pick::Node(i));
    }
    if let Some(b) = frame.span_bounds(sel) {
        return Ok(Pick::Point(b.x + b.width / 2.0, b.y + b.height / 2.0));
    }
    let lower = sel.to_lowercase();
    (0..frame.nodes.len())
        .filter(shown)
        .find(|&i| texts(frame, i).iter().any(|t| t.to_lowercase().contains(&lower)))
        .map(Pick::Node)
        .ok_or_else(|| format!("nothing on screen shows {sel:?} (try `find`, or `tree`)"))
}

/// The node a selector names (a point names the node under it).
fn node_of(frame: &Frame, sel: &str) -> Result<usize, String> {
    match pick(frame, sel)? {
        Pick::Node(i) => Ok(i),
        Pick::Point(x, y) => interact::hit(frame, x, y).ok_or_else(|| format!("no node under ({x},{y})")),
    }
}

/// The node a screen reader's action on `sel` reaches: of the nodes `sel`
/// may mean, the first that takes the action itself or through the nearest
/// ancestor that does (a label's text names an input; a button's text sits in it).
fn actor(frame: &Frame, sel: &str, verb: &str) -> Result<usize, String> {
    let takes = |i: usize| {
        frame.behaviour(i).is_some_and(|b| {
            !b.disabled
                && match verb {
                    "activate" => b.handlers.click.is_some(),
                    "focus" => b.focus.is_some(),
                    _ => b.field.is_some(),
                }
        })
    };
    let candidates = match sel.chars().next() {
        Some('@' | '#') => vec![node_of(frame, sel)?],
        _ if sel
            .split_once(',')
            .is_some_and(|(x, y)| x.trim().parse::<f32>().is_ok() && y.trim().parse::<f32>().is_ok()) =>
        {
            vec![node_of(frame, sel)?]
        }
        _ => text_matches(frame, sel),
    };
    candidates
        .iter()
        .find_map(|&c| frame.ancestors(c).find(|&a| takes(a)))
        .or_else(|| candidates.first().copied())
        .ok_or_else(|| format!("nothing on screen shows {sel:?} (try `find`, or `a11y`)"))
}

/// Scroll the containers around the node `sel` names until it is in view, as
/// a wheel over each would; a line per container scrolled (empty when none
/// had to). A point selector names no node to reveal.
fn reveal(target: &mut dyn Target, sel: &str) -> Result<String, String> {
    /// Room left between the node and the container's edge.
    const MARGIN: f32 = 8.0;
    let mut lines = String::new();
    for _ in 0..6 {
        let frame = target.frame();
        let Pick::Node(i) = pick(frame, sel)? else {
            break;
        };
        let Some(n) = frame.nodes.get(i) else { break };
        let b = n.bounds;
        let visible = |c: &crate::geometry::Bounds| {
            let v = frame.viewport;
            let (x0, y0) = (c.x.max(v.x), c.y.max(v.y));
            let (x1, y1) = (c.right().min(v.right()), c.bottom().min(v.bottom()));
            (x0, y0, x1, y1)
        };
        // The nearest scroll container that does not show all of the node.
        let hidden_in = frame.ancestors(i).skip(1).find(|&a| {
            frame.nodes.get(a).is_some_and(|c| {
                let (x0, y0, x1, y1) = visible(&c.bounds);
                c.scroll.is_some() && (b.y < y0 || b.bottom() > y1 || b.x < x0 || b.right() > x1)
            })
        });
        let Some(c) = hidden_in.and_then(|a| frame.nodes.get(a).map(|c| (a, c))) else {
            break;
        };
        let (a, container) = c;
        let (x0, y0, x1, y1) = visible(&container.bounds);
        let delta = |lo: f32, hi: f32, start: f32, end: f32| {
            if end - start > hi - lo || start < lo {
                start - lo - MARGIN
            } else if end > hi {
                end - hi + MARGIN
            } else {
                0.0
            }
        };
        let dy = delta(y0, y1, b.y, b.bottom());
        let dx = delta(x0, x1, b.x, b.right());
        if dx == 0.0 && dy == 0.0 {
            break;
        }
        let what = label(frame, a);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        target.send(Input::PointerMoved { x: cx, y: cy });
        target.send(Input::Wheel { dx, dy });
        // The next input lays the frame out again: the node is where the scroll put it.
        target.send(Input::PointerMoved { x: cx, y: cy });
        lines.push_str(&format!("scrolled {what} by ({dx:.0},{dy:.0})\n"));
    }
    Ok(lines)
}

/// Where to put the pointer for a selector, and a line saying what is there.
fn point_of(frame: &Frame, sel: &str) -> Result<(f32, f32, String), String> {
    let (x, y, named) = match pick(frame, sel)? {
        Pick::Point(x, y) => (x, y, None),
        Pick::Node(i) => {
            let n = frame.nodes.get(i).ok_or("the node is gone")?;
            // The part of the box its clip shows: a node scrolled out of view
            // cannot be pointed at.
            let b = n.bounds;
            let (mut x0, mut y0, mut x1, mut y1) = (b.x, b.y, b.right(), b.bottom());
            if let Some(c) = n.clip {
                (x0, y0, x1, y1) = (x0.max(c.x), y0.max(c.y), x1.min(c.right()), y1.min(c.bottom()));
            }
            let v = frame.viewport;
            (x0, y0, x1, y1) = (x0.max(v.x), y0.max(v.y), x1.min(v.right()), y1.min(v.bottom()));
            if x1 <= x0 || y1 <= y0 {
                return Err(format!(
                    "{} is out of view (scroll it in with `wheel DY SEL`, or use `activate`)",
                    label(frame, i)
                ));
            }
            ((x0 + x1) / 2.0, (y0 + y1) / 2.0, Some(i))
        }
    };
    let hit = interact::hit(frame, x, y);
    let mut about = format!("at ({x:.0},{y:.0})");
    match (hit, named) {
        (Some(h), Some(i)) if !frame.ancestors(h).any(|a| a == i) && !frame.ancestors(i).any(|a| a == h) => {
            about.push_str(&format!(" {}, covered by {}", label(frame, i), label(frame, h)));
        }
        (Some(h), _) => {
            about.push_str(&format!(" reaches {}", label(frame, h)));
        }
        (None, _) => about.push_str(" reaches nothing that takes the pointer"),
    }
    Ok((x, y, about))
}

/// The words a node shows or is named by: its text, an input's value and
/// placeholder, its accessible label.
fn texts(frame: &Frame, i: usize) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(n) = frame.nodes.get(i) {
        match &n.content {
            Content::Text { content, .. } => out.push(content.as_str()),
            Content::Input { edit, placeholder, .. } => {
                out.push(edit.text.as_str());
                if !placeholder.is_empty() {
                    out.push(placeholder.as_str());
                }
            }
            _ => {}
        }
    }
    if let Some(label) = frame.behaviour(i).and_then(|b| b.semantics.label.as_deref()) {
        out.push(label);
    }
    out
}

fn label(frame: &Frame, i: usize) -> String {
    frame
        .nodes
        .get(i)
        .map(|n| format!("@{} {}", n.path, inspect::describe(n)))
        .unwrap_or_default()
}

fn handled(out: &Outcome) -> String {
    match &out.handled_by {
        Some(by) => format!("handled: {by}"),
        None if out.redraw => "handled: no handler (the frame changes: hover, focus or scroll)".to_owned(),
        None => "handled: nothing took it".to_owned(),
    }
}

// ── Inspect ──────────────────────────────────────────────────────────────────

/// What a node does, in a few words: `click focus focused input disabled …`.
fn behaviour(frame: &Frame, i: usize) -> String {
    let Some(b) = frame.behaviour(i) else {
        return String::new();
    };
    let h = &b.handlers;
    let mut w: Vec<String> = Vec::new();
    for (on, name) in [
        (h.click.is_some(), "click"),
        (h.mouse_down.is_some(), "press"),
        (h.secondary_click.is_some(), "rclick"),
        (h.drag.is_some(), "drag"),
        (h.hover.is_some(), "hover"),
        (h.key_down.is_some(), "keys"),
        (h.click_outside.is_some(), "click-outside"),
        (b.field.is_some(), "input"),
        (b.disabled, "disabled"),
        (b.pass_through, "pass-through"),
    ] {
        if on {
            w.push(name.to_owned());
        }
    }
    if let Some(f) = b.focus {
        w.push(
            if focus::focused() == Some(f.id()) {
                "focused"
            } else {
                "focusable"
            }
            .to_owned(),
        );
    }
    let s = &b.semantics;
    if let Some(role) = s.role {
        w.push(format!("role={role:?}"));
    }
    if let Some(label) = &s.label {
        w.push(format!("label={label:?}"));
    }
    if let Some(c) = s.checked {
        w.push(format!("checked={c:?}"));
    }
    if let Some(v) = s.selected {
        w.push(format!("selected={v}"));
    }
    if let Some(v) = s.expanded {
        w.push(format!("expanded={v}"));
    }
    if s.hidden {
        w.push("a11y-hidden".to_owned());
    }
    if let Some(n) = frame.nodes.get(i) {
        if let Some(sc) = n.scroll {
            w.push(format!(
                "scroll={:.0},{:.0}/{:.0},{:.0}",
                sc.offset.0, sc.offset.1, sc.max.0, sc.max.1
            ));
        }
        if n.look.invisible {
            w.push("invisible".to_owned());
        }
        let why = group_reasons(n);
        if !why.is_empty() {
            w.push(format!("group({})", why.join(",")));
        }
        if n.layer > 0 {
            w.push(format!("layer={}", n.layer));
        }
    }
    w.join(" ")
}

/// The nodes under `root` (all, when `None`), as their indices in paint order.
fn under(frame: &Frame, root: Option<usize>) -> Vec<usize> {
    (0..frame.nodes.len())
        .filter(|&i| root.is_none_or(|r| frame.ancestors(i).any(|a| a == r)))
        .collect()
}

fn depth_below(frame: &Frame, i: usize, root: Option<usize>) -> usize {
    frame
        .ancestors(i)
        .take_while(|&a| Some(a) != root)
        .count()
        .saturating_sub(usize::from(root.is_none()))
}

/// Why a node paints aside as a group (a window-sized layer, composited):
/// empty when it paints straight onto the frame.
fn group_reasons(n: &crate::layout::FrameNode) -> Vec<String> {
    let l = &n.look;
    let mut why = Vec::new();
    if l.opacity < 1.0 {
        why.push(format!("opacity {:.2}", l.opacity));
    }
    if !l.transform.is_identity() {
        why.push(format!(
            "scale {:.3} rotate {:.1}",
            l.transform.scale, l.transform.rotate
        ));
    }
    if l.effects.mask.is_some() {
        why.push("mask".to_owned());
    }
    if !l.effects.filters.is_empty() {
        why.push(format!("{} filters", l.effects.filters.len()));
    }
    why
}

fn tree(frame: &Frame, root: Option<usize>, max_depth: Option<usize>) -> String {
    let mut out = String::new();
    let mut shown = 0_usize;
    for i in under(frame, root) {
        let Some(n) = frame.nodes.get(i) else { continue };
        let depth = depth_below(frame, i, root);
        if max_depth.is_some_and(|m| depth > m) {
            continue;
        }
        shown = shown.saturating_add(1);
        out.push_str(&format!(
            "{}@{}  {}  {}  {}\n",
            "  ".repeat(depth),
            n.path,
            inspect::describe(n),
            n.bounds,
            behaviour(frame, i)
        ));
    }
    out.push_str(&format!("{shown} nodes"));
    out
}

fn components(frame: &Frame, root: Option<usize>) -> String {
    let mut out = String::new();
    let mut shown = 0_usize;
    for i in under(frame, root) {
        let Some(n) = frame.nodes.get(i) else { continue };
        let Some(name) = n.name else { continue };
        let depth = frame
            .ancestors(i)
            .skip(1)
            .take_while(|&a| Some(a) != root)
            .filter(|&a| frame.nodes.get(a).is_some_and(|n| n.name.is_some()))
            .count();
        shown = shown.saturating_add(1);
        out.push_str(&format!(
            "{}#{name}  @{}  {}  {}\n",
            "  ".repeat(depth),
            n.path,
            n.bounds,
            behaviour(frame, i)
        ));
    }
    out.push_str(&format!("{shown} named nodes of {}", frame.nodes.len()));
    out
}

fn node_detail(frame: &Frame, i: usize) -> String {
    let Some(n) = frame.nodes.get(i) else {
        return String::new();
    };
    let mut out = String::new();
    out.push_str(&format!("@{}  {}\n", n.path, inspect::describe(n)));
    out.push_str(&format!("  box        {}  content {}\n", n.bounds, n.content_box));
    if let Some(c) = n.clip {
        out.push_str(&format!("  clip       {c}\n"));
    }
    let l = &n.look;
    let mut look = Vec::new();
    if let Some(bg) = l.background {
        look.push(format!("bg {bg}"));
    }
    if l.border.top > 0.0 || l.border.right > 0.0 || l.border.bottom > 0.0 || l.border.left > 0.0 {
        let color = l.border_color.map(|c| c.to_string()).unwrap_or_default();
        look.push(format!(
            "border {}/{}/{}/{} {color}",
            l.border.top, l.border.right, l.border.bottom, l.border.left
        ));
    }
    let r = &l.radius;
    if r.top_left > 0.0 || r.top_right > 0.0 || r.bottom_right > 0.0 || r.bottom_left > 0.0 {
        look.push(format!(
            "radius {}/{}/{}/{}",
            r.top_left, r.top_right, r.bottom_right, r.bottom_left
        ));
    }
    if !l.shadows.is_empty() {
        look.push(format!("shadows {}", l.shadows.len()));
    }
    if let Some(ring) = l.ring {
        look.push(format!("ring {ring:?}"));
    }
    if l.opacity < 1.0 {
        look.push(format!("opacity {}", l.opacity));
    }
    if l.invisible {
        look.push("invisible".to_owned());
    }
    if !look.is_empty() {
        out.push_str(&format!("  look       {}\n", look.join("  ")));
    }
    match &n.content {
        Content::Text { content, text, runs } => {
            out.push_str(&format!(
                "  text       {content:?}\n             {}px/{}px w{} {}{}\n",
                text.size,
                text.line_height,
                text.weight,
                text.color,
                if runs.is_empty() {
                    String::new()
                } else {
                    format!("  {} styled runs", runs.len())
                }
            ));
        }
        Content::Input {
            edit,
            placeholder,
            focused,
            multiline,
            ..
        } => {
            out.push_str(&format!(
                "  input      value {:?}  placeholder {placeholder:?}  focused {focused}  multiline {multiline}\n",
                edit.text
            ));
        }
        Content::Icon { icon, color, .. } => {
            out.push_str(&format!("  icon       {} {color}\n", icon.name()));
        }
        Content::Image { source, .. } => {
            let (w, h) = source.size();
            out.push_str(&format!("  image      {w}x{h} source pixels\n"));
        }
        Content::Shapes(shapes) => {
            out.push_str(&format!("  shapes     {}\n", shapes.len()));
        }
        Content::None => {}
    }
    let does = behaviour(frame, i);
    if !does.is_empty() {
        out.push_str(&format!("  does       {does}\n"));
    }
    let ancestry: Vec<String> = frame
        .ancestors(i)
        .skip(1)
        .filter_map(|a| frame.nodes.get(a))
        .filter_map(|a| a.name.map(|name| format!("#{name} @{}", a.path)))
        .collect();
    if !ancestry.is_empty() {
        out.push_str(&format!("  inside     {}\n", ancestry.join(" < ")));
    }
    let children: Vec<String> = (i.saturating_add(1)..frame.nodes.len())
        .filter(|&c| frame.nodes.get(c).and_then(|n| n.parent) == Some(i))
        .map(|c| label(frame, c))
        .collect();
    out.push_str(&format!("  children   {}", children.len()));
    for c in children.iter().take(12) {
        out.push_str(&format!("\n             {c}"));
    }
    if children.len() > 12 {
        out.push_str(&format!("\n             … {} more", children.len().saturating_sub(12)));
    }
    out
}

fn find(frame: &Frame, needle: &str) -> String {
    if needle.is_empty() {
        return "find: needs TEXT".to_owned();
    }
    let lower = needle.to_lowercase();
    let mut out = String::new();
    let mut found = 0_usize;
    for i in 0..frame.nodes.len() {
        if !texts(frame, i).iter().any(|t| t.to_lowercase().contains(&lower)) {
            continue;
        }
        let Some(n) = frame.nodes.get(i) else { continue };
        found = found.saturating_add(1);
        // What takes the pointer for it: itself or the nearest ancestor that acts.
        let acts = frame.ancestors(i).find(|&a| {
            frame
                .behaviour(a)
                .is_some_and(|b| b.handlers.click.is_some() || b.focus.is_some() || b.field.is_some())
        });
        let via = match acts {
            Some(a) if a != i => format!("  in {}", label(frame, a)),
            _ => String::new(),
        };
        out.push_str(&format!(
            "@{}  {}  {}  {}{via}\n",
            n.path,
            inspect::describe(n),
            n.bounds,
            behaviour(frame, i)
        ));
    }
    out.push_str(&format!("{found} found"));
    out
}

fn at(frame: &Frame, x: f32, y: f32) -> String {
    let Some(h) = interact::hit(frame, x, y) else {
        return format!("nothing at ({x},{y}) takes the pointer");
    };
    let mut out = String::new();
    for (k, a) in frame.ancestors(h).enumerate() {
        let does = behaviour(frame, a);
        out.push_str(&format!(
            "{}{}  {does}\n",
            if k == 0 { "" } else { "  in " },
            label(frame, a)
        ));
    }
    out.trim_end().to_owned()
}

fn focused(frame: &Frame) -> String {
    let Some(f) = focus::focused() else {
        return "nothing has focus".to_owned();
    };
    (0..frame.nodes.len())
        .find(|&i| frame.behaviour(i).and_then(|b| b.focus).is_some_and(|h| h.id() == f))
        .map_or_else(
            || "focus is on a node not in this frame".to_owned(),
            |i| label(frame, i),
        )
}

/// What a screen reader hears, as a tree: containers without a name fold
/// into their parent, so the meaningful nodes stand out.
fn a11y(frame: &Frame, root: Option<usize>) -> String {
    let update = access::tree(frame, focus::focused(), 1.0, "window");
    let nodes: std::collections::HashMap<NodeId, &accesskit::Node> =
        update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let start = match root {
        Some(i) => match frame.nodes.get(i).map(|n| access::id_of(&n.path)) {
            Some(id) if nodes.contains_key(&id) => id,
            _ => return format!("{} is not in what a screen reader hears", label(frame, i)),
        },
        None => NodeId(0),
    };
    let mut out = String::new();
    let mut count = 0_usize;
    let mut stack = vec![(start, 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = nodes.get(&id) else { continue };
        let role = node.role();
        let quiet = matches!(role, accesskit::Role::GenericContainer) && node.label().is_none();
        let next = if quiet {
            depth
        } else {
            count = count.saturating_add(1);
            let mut line = format!("{}{role:?}", "  ".repeat(depth));
            if let Some(l) = node.label() {
                line.push_str(&format!(" {l:?}"));
            }
            if let Some(v) = node.value()
                && node.label() != Some(v)
            {
                line.push_str(&format!(" value={v:?}"));
            }
            if let Some(t) = node.toggled() {
                line.push_str(&format!(" toggled={t:?}"));
            }
            if let Some(e) = node.is_expanded() {
                line.push_str(&format!(" expanded={e}"));
            }
            if let Some(s) = node.is_selected() {
                line.push_str(&format!(" selected={s}"));
            }
            if node.is_disabled() {
                line.push_str(" disabled");
            }
            if id == update.focus && id != NodeId(0) {
                line.push_str(" FOCUSED");
            }
            let actions: Vec<&str> = [
                (Action::Click, "click"),
                (Action::Focus, "focus"),
                (Action::SetValue, "setvalue"),
            ]
            .into_iter()
            .filter(|(a, _)| node.supports_action(*a))
            .map(|(_, n)| n)
            .collect();
            if !actions.is_empty() {
                line.push_str(&format!(" [{}]", actions.join(",")));
            }
            if let Some(i) = access::target(frame, id)
                && let Some(n) = frame.nodes.get(i)
            {
                line.push_str(&format!("  @{}", n.path));
            }
            out.push_str(&format!("{line}\n"));
            depth.saturating_add(1)
        };
        for child in node.children().iter().rev() {
            stack.push((*child, next));
        }
    }
    out.push_str(&format!("{count} nodes heard"));
    out
}

// ── Signals ──────────────────────────────────────────────────────────────────

const VALUE_WIDTH: usize = 120;

fn signals(filter: Option<&str>) -> String {
    let all = reactive::probes();
    let lower = filter.map(str::to_lowercase);
    let mut out = String::new();
    let mut shown = 0_usize;
    for v in &all {
        if lower
            .as_ref()
            .is_some_and(|f| !v.label.to_lowercase().contains(f) && !v.value.to_lowercase().contains(f))
        {
            continue;
        }
        shown = shown.saturating_add(1);
        let mut value: String = v.value.chars().take(VALUE_WIDTH).collect();
        if v.value.chars().count() > VALUE_WIDTH {
            value.push('…');
        }
        out.push_str(&format!(
            "#{} {}: {} = {value}  ({} readers)\n",
            v.id,
            v.label,
            short_type(v.type_name),
            v.subscribers
        ));
    }
    let g = reactive::graph();
    out.push_str(&format!(
        "{shown} labeled signals shown, {} labeled, {} signals alive",
        all.len(),
        g.signals
    ));
    out
}

/// A type name without its module paths: `Option<String>`, not `core::option::Option<alloc::string::String>`.
fn short_type(name: &str) -> String {
    let mut out = String::new();
    let mut word = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' || c == ':' {
            word.push(c);
        } else {
            out.push_str(word.rsplit("::").next().unwrap_or(&word));
            word.clear();
            out.push(c);
        }
    }
    out.push_str(word.rsplit("::").next().unwrap_or(&word));
    out
}

/// The id of the labeled signal `which` names: `#12`, `12`, or a label
/// only one signal has.
fn probe_id(which: &str) -> Result<u64, String> {
    let trimmed = which.trim_start_matches('#');
    let all = reactive::probes();
    if let Ok(id) = trimmed.parse::<u64>()
        && all.iter().any(|v| v.id == id)
    {
        return Ok(id);
    }
    let named: Vec<_> = all.iter().filter(|v| v.label == which).collect();
    match named.as_slice() {
        [one] => Ok(one.id),
        [] => Err(format!("no labeled signal {which:?} (see `signals`)")),
        many => Err(format!(
            "{} signals are labeled {which:?}: {}; name one by id",
            many.len(),
            many.iter().map(|v| format!("#{}", v.id)).collect::<Vec<_>>().join(" ")
        )),
    }
}

fn signal(which: &str) -> Result<String, String> {
    let id = probe_id(which)?;
    let v = reactive::probes()
        .into_iter()
        .find(|v| v.id == id)
        .ok_or("the signal was dropped")?;
    Ok(format!(
        "#{} {}: {} ({} readers)\n{}",
        v.id, v.label, v.type_name, v.subscribers, v.value
    ))
}

// ── Measure ──────────────────────────────────────────────────────────────────

fn reports(log: &FrameLog, n: usize) -> String {
    let reports: Vec<&FrameReport> = log.reports().rev().take(n.max(1)).collect();
    if reports.is_empty() {
        return "no frame drawn yet".to_owned();
    }
    reports
        .iter()
        .rev()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The frame budget `perf` counts frames over: one 60 Hz refresh.
const BUDGET: Duration = Duration::from_millis(16);

/// Frame times over `reports`, as `perf` and `bench` print them.
pub fn perf_of<'a>(reports: impl IntoIterator<Item = &'a FrameReport>) -> String {
    let reports: Vec<&FrameReport> = reports.into_iter().collect();
    let (Some(first), Some(last)) = (reports.first(), reports.last()) else {
        return "no frame drawn yet".to_owned();
    };
    let line = |name: &str, s: Spread| format!("{name:<7} p50 {}  p95 {}  max {}", ms(s.p50), ms(s.p95), ms(s.max));
    let total = Spread::of(reports.iter().map(|r| r.timings.total()));
    let over = reports.iter().filter(|r| r.timings.total() > BUDGET).count();
    let mut out = format!(
        "{} frames (#{}..#{}), {over} over {}\n",
        reports.len(),
        first.number,
        last.number,
        ms(BUDGET)
    );
    out.push_str(&format!("{}\n", line("total", total)));
    out.push_str(&format!(
        "{}\n",
        line("build", Spread::of(reports.iter().map(|r| r.timings.build)))
    ));
    out.push_str(&format!(
        "{}\n",
        line("layout", Spread::of(reports.iter().map(|r| r.timings.layout)))
    ));
    out.push_str(&line("paint", Spread::of(reports.iter().map(|r| r.timings.paint))));
    let heaps: Vec<heap::Heap> = reports.iter().filter_map(|r| r.footprint.heap).collect();
    if !heaps.is_empty() {
        let n = Spread::of(heaps.iter().map(|h| h.allocations));
        let b = Spread::of(heaps.iter().map(|h| h.allocated));
        out.push_str(&format!(
            "\nallocs  p50 {}  p95 {}  max {} per frame; bytes p50 {}  p95 {}  max {}",
            n.p50,
            n.p95,
            n.max,
            bytes(b.p50),
            bytes(b.p95),
            bytes(b.max)
        ));
    }
    let effects = Spread::of(reports.iter().map(|r| r.reactive.effect_runs));
    out.push_str(&format!(
        "\neffects p50 {}  p95 {}  max {} runs per frame",
        effects.p50, effects.p95, effects.max
    ));
    out
}

fn perf(log: &FrameLog) -> String {
    let mut out = perf_of(log.reports());
    let fps = log.fps();
    if fps > 0 {
        out.push_str(&format!("\n{fps} frames in the last second"));
    }
    out
}

fn memory(f: &Footprint, frame: &Frame) -> String {
    let p = process::usage();
    let mut out = String::new();
    out.push_str(&format!(
        "process  resident {}  cpu time {}\n",
        p.resident.map_or_else(|| "?".to_owned(), bytes),
        p.cpu
            .map_or_else(|| "?".to_owned(), |c| format!("{:.2}s", c.as_secs_f32()))
    ));
    out.push_str(&format!(
        "heap     {}\n",
        match heap::now() {
            Some(h) => format!(
                "{} live; {} allocations, {} allocated since start",
                bytes(h.live),
                h.allocations,
                bytes(h.allocated)
            ),
            None => "not counted (install gui::heap::COUNTING as the #[global_allocator])".to_owned(),
        }
    ));
    out.push_str(&format!(
        "caches   {} of images: glyphs {}  icons {}  shadows {}  images {}  layers {}\n",
        bytes(u64::try_from(f.raster_bytes()).unwrap_or(u64::MAX)),
        f.glyphs,
        f.icons,
        f.shadows,
        f.images,
        f.layers
    ));
    out.push_str(&format!("text     {} paragraphs shaped\n", f.shaped));
    let g = f.graph;
    out.push_str(&format!(
        "graph    signals {} ({} labeled)  computed {}  effects {}  subscriptions {}\n",
        g.signals,
        reactive::probes().len(),
        g.computeds,
        g.effects,
        g.subscriptions
    ));
    let named = frame.nodes.iter().filter(|n| n.name.is_some()).count();
    out.push_str(&format!(
        "tree     {} nodes, {named} named, {} layers",
        f.nodes,
        frame.layers.len().saturating_add(1)
    ));
    out
}

fn diff(d: &LayoutDiff) -> String {
    if d.is_empty() {
        return "the last frames changed nothing".to_owned();
    }
    let mut out = format!("{d}\n");
    for c in d.changes.iter().take(80) {
        let how = match &c.change {
            inspect::Change::Added { after } => format!("added {after}"),
            inspect::Change::Removed { before } => format!("removed {before}"),
            u @ inspect::Change::Updated { before, after, .. } => {
                let mut w = Vec::new();
                if u.moved() || u.resized() {
                    w.push(format!("{before} -> {after}"));
                }
                if u.repainted() {
                    w.push("repainted".to_owned());
                }
                w.join(", ")
            }
        };
        out.push_str(&format!("@{}  {}  {how}\n", c.path, c.what));
    }
    if d.changes.len() > 80 {
        out.push_str(&format!("… {} more", d.changes.len().saturating_sub(80)));
    }
    out.trim_end().to_owned()
}

// ── Headless ─────────────────────────────────────────────────────────────────

/// A headless devtools session over a [`Scenario`]: each command runs and
/// answers as text; input draws the next frame, as a window would.
pub struct Session<C> {
    scenario: Scenario<C>,
    log: FrameLog,
}

impl<C: FnMut() -> Element> Session<C> {
    /// A session on `scenario`, its first frame drawn.
    pub fn new(mut scenario: Scenario<C>) -> Result<Self, ScenarioError> {
        let mut log = FrameLog::default();
        scenario.settle()?;
        for r in scenario.take_reports() {
            log.push(r);
        }
        Ok(Self { scenario, log })
    }

    /// The frames this session drew, the last [`LOG_FRAMES`].
    #[must_use]
    pub fn log(&self) -> &FrameLog {
        &self.log
    }

    /// Draw `n` frames (at least one) of the page as it is, each built anew; their reports.
    pub fn bench(&mut self, n: usize) -> Result<Vec<FrameReport>, String> {
        let mut drawn = Vec::with_capacity(n.max(1));
        for _ in 0..n.max(1) {
            self.scenario.act("bench", || {});
            drawn.push(self.draw()?);
        }
        Ok(drawn)
    }

    /// Drop the logged frame reports, so the heap holds only what the app
    /// does: a leak check measures after this (a report holds its layout
    /// diff and every signal its frame wrote).
    pub fn forget_log(&mut self) {
        self.log = FrameLog::default();
    }

    /// Bring the page to rest (due timers run, motion ends) and draw it.
    pub fn rest(&mut self) -> Result<FrameReport, String> {
        self.scenario.rest(Duration::from_secs(30)).map_err(|e| e.to_string())?;
        self.draw()
    }

    /// The scenario under the session.
    pub fn scenario(&mut self) -> &mut Scenario<C> {
        &mut self.scenario
    }

    /// Draw a frame and log it; its report.
    fn draw(&mut self) -> Result<FrameReport, String> {
        self.scenario.settle().map_err(|e| e.to_string())?;
        let mut last = None;
        for r in self.scenario.take_reports() {
            last = Some(r.clone());
            self.log.push(r);
        }
        last.ok_or_else(|| "no frame was drawn".to_owned())
    }

    /// Run one command line; the answer, or why it failed.
    pub fn run(&mut self, line: &str) -> Result<String, String> {
        let words = split(line)?;
        let done = execute(self.scenario.pipeline_mut(), &self.log, &words)?;
        match done {
            Done::Text(t) => Ok(t),
            Done::Acted(what) => Ok(format!("{what}\n{}", self.draw()?)),
            Done::Frame => self.draw().map(|r| r.to_string()),
            Done::Screenshot { path, overlay } => {
                let report = self.draw()?;
                let mut pixmap = self.scenario.pixmap().clone();
                if overlay {
                    let scale = self.scenario.scale();
                    self.scenario.pipeline_mut().paint_overlay(&mut pixmap, scale);
                }
                save(&pixmap, &path)?;
                Ok(format!(
                    "wrote {} ({}x{})\n{report}",
                    path.display(),
                    pixmap.width(),
                    pixmap.height()
                ))
            }
            Done::Headless(words) => self.headless(&words),
        }
    }

    fn headless(&mut self, words: &[String]) -> Result<String, String> {
        let (verb, args) = words.split_first().ok_or("no command")?;
        let arg = |n: usize| {
            args.get(n)
                .map(String::as_str)
                .ok_or(format!("{verb}: needs an argument"))
        };
        match verb.as_str() {
            "wait" => {
                let ms_ = num(arg(0)?)?;
                self.scenario.wait(Duration::from_secs_f32(ms_.max(0.0) / 1000.0));
                let r = self.draw()?;
                Ok(format!(
                    "{} later\n{r}",
                    ms(Duration::from_secs_f32(ms_.max(0.0) / 1000.0))
                ))
            }
            "rest" => {
                self.scenario.rest(Duration::from_secs(30)).map_err(|e| e.to_string())?;
                self.draw().map(|r| format!("at rest\n{r}"))
            }
            "size" => {
                let w = u32::try_from(count(arg(0)?)?).map_err(|e| e.to_string())?;
                let h = u32::try_from(count(arg(1)?)?).map_err(|e| e.to_string())?;
                self.scenario.resize(w, h).map_err(|e| e.to_string())?;
                self.draw().map(|r| format!("{w}x{h}\n{r}"))
            }
            "bench" => {
                let n = args.first().map(|s| count(s)).transpose()?.unwrap_or(60);
                Ok(perf_of(&self.bench(n)?))
            }
            "monkey" => self.monkey(args),
            other => Err(format!("unknown command {other:?}")),
        }
    }

    /// `monkey`: random input, each step checked; the report, or why it stopped.
    fn monkey(&mut self, args: &[String]) -> Result<String, String> {
        let (flags, rest) = flags(args);
        let steps = rest.first().map(|s| count(s)).transpose()?.unwrap_or(200);
        let seed = flag_num(&flags, "seed")?.map_or(0x5eed, |s| u64::try_from(s).unwrap_or(0x5eed));
        let budget =
            flag_num(&flags, "budget")?.map_or(BUDGET, |ms| Duration::from_millis(u64::try_from(ms).unwrap_or(16)));
        let verbose = flags.iter().any(|(k, _)| *k == "verbose");
        let mut rng = Rng(seed.max(1));
        let mut log: Vec<String> = Vec::new();
        let mut out = String::new();
        // What the log holds is the devtools', not the app's.
        self.forget_log();
        let heap_before = heap::now();
        let shown_before = self.scenario.last().nodes.len();
        let mut slow: Vec<(Duration, usize, String)> = Vec::new();
        let mut times = Vec::new();
        let mut most_nodes = 0_usize;
        let mut errors = 0_usize;
        for step in 1..=steps {
            let action = self.random_action(&mut rng);
            let words = split(&action)?;
            // A handler that panics leaves its state half written; the run stops there.
            let ran = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.step(&words)));
            let report = match ran {
                Ok(Ok(report)) => report,
                Ok(Err(e)) => {
                    // A selector that missed (a node gone) is not a fault of the app.
                    errors = errors.saturating_add(1);
                    if e.starts_with("layout error") {
                        return Err(stopped(seed, step, &action, &format!("the layout failed: {e}"), &log));
                    }
                    log.push(format!("{step:>4} {action}  ({e})"));
                    continue;
                }
                Err(panic) => {
                    let why = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "a panic".to_owned());
                    return Err(stopped(seed, step, &action, &format!("PANIC {why}"), &log));
                }
            };
            let took = report.timings.total();
            times.push(took);
            most_nodes = most_nodes.max(report.footprint.nodes);
            if took > budget {
                slow.push((took, step, action.clone()));
            }
            let line = format!(
                "{step:>4} {action}  {}  {} nodes{}",
                ms(took),
                report.footprint.nodes,
                if report.causes.is_empty() {
                    String::new()
                } else {
                    format!("  {}", report.causes.join("; "))
                }
            );
            if verbose {
                out.push_str(&line);
                out.push('\n');
            }
            log.push(line);
        }
        self.scenario.rest(Duration::from_secs(30)).map_err(|e| e.to_string())?;
        self.draw()?;
        self.forget_log();
        let heap_after = heap::now();
        let spread = Spread::of(times.iter().copied());
        out.push_str(&format!(
            "monkey seed={seed}: {steps} steps, {} drew a frame, {errors} missed (a target gone)\n",
            times.len()
        ));
        out.push_str(&format!(
            "frames   p50 {}  p95 {}  max {}  {} over {}\n",
            ms(spread.p50),
            ms(spread.p95),
            ms(spread.max),
            slow.len(),
            ms(budget)
        ));
        slow.sort_by_key(|(t, _, _)| std::cmp::Reverse(*t));
        for (t, step, action) in slow.iter().take(5) {
            out.push_str(&format!("  slow   {} at step {step}: {action}\n", ms(*t)));
        }
        out.push_str(&format!("tree     at most {most_nodes} nodes\n"));
        match (heap_before, heap_after) {
            (Some(b), Some(a)) => out.push_str(&format!(
                "heap     {} before, {} after at rest ({}{})",
                bytes(b.live),
                bytes(a.live),
                if a.live >= b.live { "+" } else { "-" },
                bytes(a.live.abs_diff(b.live))
            )),
            _ => out.push_str("heap     not counted"),
        }
        let shown_after = self.scenario.last().nodes.len();
        if shown_after != shown_before {
            out.push_str(&format!(
                "\n         the page ends showing {shown_after} nodes where it began with {shown_before} \
                 (a menu or dialog left open): the heap holds what that shows, not only what leaked"
            ));
        }
        Ok(out)
    }

    /// Run one monkey action and draw the frame it leads to.
    fn step(&mut self, words: &[String]) -> Result<FrameReport, String> {
        if words.first().is_some_and(|w| w == "wait") {
            let ms_ = num(words.get(1).map_or("0", String::as_str))?;
            self.scenario.wait(Duration::from_secs_f32(ms_ / 1000.0));
            return self.draw();
        }
        execute(self.scenario.pipeline_mut(), &self.log, words)?;
        self.draw()
    }

    /// A random action on what the last frame shows.
    fn random_action(&self, rng: &mut Rng) -> String {
        let frame = Target::frame(self.scenario.pipeline());
        let targets = frame.pointer_targets();
        let v = frame.viewport;
        let point = |rng: &mut Rng| -> String {
            match rng.pick(&targets) {
                Some(b) => {
                    // Somewhere inside the box, kept in the window.
                    let x = (b.x + b.width * rng.unit()).clamp(v.x, v.right() - 1.0);
                    let y = (b.y + b.height * rng.unit()).clamp(v.y, v.bottom() - 1.0);
                    format!("{x:.0},{y:.0}")
                }
                None => format!("{:.0},{:.0}", v.width * rng.unit(), v.height * rng.unit()),
            }
        };
        const KEYS: [&str; 14] = [
            "tab",
            "shift+tab",
            "enter",
            "esc",
            "space",
            "down",
            "up",
            "left",
            "right",
            "home",
            "end",
            "backspace",
            "pagedown",
            "mod+a",
        ];
        const WORDS: [&str; 8] = ["a", "hello", "42", "Tiếng Việt", "😀", "x y z", "-1", "@me"];
        match rng.below(100) {
            0..=39 => format!("click {}", point(rng)),
            40..=54 => format!("hover {}", point(rng)),
            55..=69 => format!("press {}", rng.pick(&KEYS).copied().unwrap_or("tab")),
            70..=77 => format!("type {:?}", rng.pick(&WORDS).copied().unwrap_or("a")),
            78..=86 => {
                let dy = if rng.below(2) == 0 { 240 } else { -240 };
                format!("wheel {dy} {}", point(rng))
            }
            87..=89 => format!("rclick {}", point(rng)),
            90..=94 => "leave".to_owned(),
            _ => format!("wait {}", rng.below(600)),
        }
    }

    /// Run each line of `script` (`#` starts a comment), writing each command
    /// and its answer to `out`; whether every command succeeded.
    pub fn run_script(
        &mut self,
        script: impl IntoIterator<Item = String>,
        out: &mut impl std::io::Write,
    ) -> std::io::Result<bool> {
        let mut ok = true;
        for line in script {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            writeln!(out, "> {line}")?;
            match self.run(line) {
                Ok(answer) => writeln!(out, "{answer}")?,
                Err(e) => {
                    ok = false;
                    writeln!(out, "error: {e}")?;
                }
            }
            writeln!(out)?;
            out.flush()?;
        }
        Ok(ok)
    }
}

/// A small, seeded generator (xorshift64): the same seed walks the same way.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number below `n` (0 when `n` is 0).
    fn below(&mut self, n: u64) -> u64 {
        self.next().checked_rem(n).unwrap_or(0)
    }

    /// A number in `0..1`.
    fn unit(&mut self) -> f32 {
        // 24 bits: what an f32 holds exactly.
        let bits = u32::try_from(self.next() >> 40).unwrap_or(0);
        crate::convert::u32_to_f32(bits) / 16_777_216.0
    }

    fn pick<'a, T>(&mut self, from: &'a [T]) -> Option<&'a T> {
        let n = u64::try_from(from.len()).unwrap_or(0);
        from.get(usize::try_from(self.below(n)).unwrap_or(0))
    }
}

/// Why a monkey run stopped, with the steps before it and how to repeat it.
fn stopped(seed: u64, step: usize, action: &str, why: &str, log: &[String]) -> String {
    let mut out = format!("monkey seed={seed} stopped at step {step}: {action}\n  {why}\nthe steps before it:\n");
    let from = log.len().saturating_sub(12);
    for line in log.get(from..).unwrap_or_default() {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!("repeat with: monkey {step} --seed={seed} --verbose"));
    out
}

fn save(pixmap: &tiny_skia::Pixmap, path: &Path) -> Result<(), String> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    pixmap.save_png(path).map_err(|e| e.to_string())
}

// ── Attached ─────────────────────────────────────────────────────────────────

/// The socket a live app listens on when `GUI_DEVTOOLS` is `1` (or set,
/// with no path): one per process, in the temporary directory.
#[must_use]
pub fn socket_path() -> PathBuf {
    match std::env::var_os("GUI_DEVTOOLS") {
        Some(p) if !p.is_empty() && p != "1" => PathBuf::from(p),
        _ => std::env::temp_dir().join(format!("gui-devtools-{}.sock", std::process::id())),
    }
}

/// How long a request waits for the window to answer (a frame it waits for
/// is not drawn while the window is hidden).
pub const ANSWER_WITHIN: Duration = Duration::from_secs(10);

/// The sockets of the apps listening now, newest first; a stale socket file
/// (its app gone) is removed.
#[cfg(unix)]
#[must_use]
pub fn listening() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(std::env::temp_dir()) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gui-devtools-") && n.ends_with(".sock"))
        })
        .filter(|p| {
            let alive = std::os::unix::net::UnixStream::connect(p).is_ok();
            if !alive {
                // Best effort: another process may have removed it first.
                std::fs::remove_file(p).unwrap_or(());
            }
            alive
        })
        .map(|p| {
            (
                std::fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
                p,
            )
        })
        .collect();
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found.into_iter().map(|(_, p)| p).collect()
}

/// Send `lines` to the app listening on `socket` and return what it answers,
/// each command echoed as `> line` before its answer.
#[cfg(unix)]
pub fn request(socket: &Path, lines: &[String]) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(
        ANSWER_WITHIN.saturating_mul(u32::try_from(lines.len()).unwrap_or(1).max(1)),
    ))?;
    for line in lines {
        writeln!(stream, "{line}")?;
    }
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut answer = String::new();
    stream.read_to_string(&mut answer)?;
    Ok(answer)
}

/// One line from a client, and where its answer goes.
pub(crate) struct Request {
    pub(crate) line: String,
    pub(crate) answer: std::sync::mpsc::Sender<String>,
}

/// Listen on `path` for clients; each line they send reaches the UI thread
/// through `to_ui` and its answer goes back. The listener runs on its own
/// thread for the life of the process.
#[cfg(unix)]
pub(crate) fn listen(path: &Path, to_ui: crate::mailbox::Sender<Request>) -> std::io::Result<()> {
    use std::io::{BufRead, BufReader, Write};
    // A socket left by an earlier run with this path (an explicit one).
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    let listener = std::os::unix::net::UnixListener::bind(path)?;
    std::thread::Builder::new()
        .name("gui-devtools".to_owned())
        .spawn(move || {
            for stream in listener.incoming().filter_map(Result::ok) {
                let to_ui = to_ui.clone();
                let Ok(reader) = stream.try_clone() else { continue };
                let mut writer = stream;
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    let line = line.trim().to_owned();
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    let (tx, rx) = std::sync::mpsc::channel();
                    let answer = if to_ui.send(Request {
                        line: line.clone(),
                        answer: tx,
                    }) {
                        rx.recv_timeout(ANSWER_WITHIN).unwrap_or_else(|_| {
                            "error: the window did not answer (hidden or minimised windows draw no frames)".to_owned()
                        })
                    } else {
                        "error: the app is gone".to_owned()
                    };
                    if writeln!(writer, "> {line}\n{answer}\n").is_err() {
                        break;
                    }
                }
            }
        })?;
    Ok(())
}

/// A request waiting for the next frame: what the command said, and where
/// to write the frame's pixels (with the overlay or not), for a screenshot.
type Waiting = (Request, String, Option<(PathBuf, bool)>);

/// A live window's devtools: the requests it holds and the frames it logged.
#[derive(Default)]
pub(crate) struct Attached {
    pub(crate) log: FrameLog,
    /// Requests that arrived and wait for the UI thread.
    pub(crate) inbox: std::rc::Rc<std::cell::RefCell<VecDeque<Request>>>,
    /// Answers waiting for the next frame (its report, or its pixels).
    waiting: Vec<Waiting>,
}

impl Attached {
    /// Run the requests that arrived; whether a frame is due for one.
    pub(crate) fn run(&mut self, target: &mut dyn Target) -> bool {
        let mut due = false;
        loop {
            let next = self.inbox.borrow_mut().pop_front();
            let Some(req) = next else { break };
            let done = split(&req.line).and_then(|w| execute(target, &self.log, &w));
            match done {
                Ok(Done::Text(t)) => drop(req.answer.send(t)),
                Ok(Done::Acted(what)) => {
                    due = true;
                    self.waiting.push((req, what, None));
                }
                Ok(Done::Frame) => {
                    due = true;
                    self.waiting.push((req, String::new(), None));
                }
                Ok(Done::Screenshot { path, overlay }) => {
                    due = true;
                    self.waiting.push((req, String::new(), Some((path, overlay))));
                }
                Ok(Done::Headless(w)) => drop(req.answer.send(format!(
                    "error: {} runs only headless (a live window keeps its own time and size)",
                    w.first().map_or("", String::as_str)
                ))),
                Err(e) => drop(req.answer.send(format!("error: {e}"))),
            }
        }
        due
    }

    /// A frame was drawn: log it and answer what waited for it. `overlay`
    /// paints the debug overlay on a copy of the pixels.
    pub(crate) fn drawn(
        &mut self,
        report: &FrameReport,
        pixmap: &tiny_skia::Pixmap,
        overlay: &mut dyn FnMut(&mut tiny_skia::Pixmap),
    ) {
        self.log.push(report.clone());
        for (req, what, shot) in self.waiting.drain(..) {
            let mut answer = String::new();
            if !what.is_empty() {
                answer.push_str(&format!("{what}\n"));
            }
            if let Some((path, with_overlay)) = shot {
                let mut copy = pixmap.clone();
                if with_overlay {
                    overlay(&mut copy);
                }
                match save(&copy, &path) {
                    Ok(()) => {
                        answer.push_str(&format!(
                            "wrote {} ({}x{})\n",
                            path.display(),
                            copy.width(),
                            copy.height()
                        ));
                    }
                    Err(e) => {
                        answer.push_str(&format!("error: {e}\n"));
                    }
                }
            }
            answer.push_str(&format!("{report}"));
            drop(req.answer.send(answer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{IntoElement, div, text};
    use crate::reactive::Signal;
    use crate::styled::Styled;

    /// A counter: a labeled count, a button that adds one, its text.
    fn counter() -> Session<impl FnMut() -> Element> {
        let count = Signal::labeled("count", 0_i32);
        let scenario = Scenario::measuring(
            move || {
                let c = count.clone();
                div()
                    .size_full()
                    .p(8.0)
                    .child(
                        div()
                            .named("add")
                            .p(4.0)
                            .on_click(move |_| c.update(|n| n.saturating_add(1)))
                            .child(text("Add")),
                    )
                    .child(text(format!("count is {}", count.get())))
                    .into_element()
            },
            320,
            200,
            1.0,
        )
        .expect("a scenario");
        Session::new(scenario).expect("a session")
    }

    #[test]
    fn a_session_reads_acts_and_writes_signals_by_text() {
        let mut s = counter();
        let found = s.run("find count").expect("find");
        assert!(found.contains("Text \"count is 0\""), "{found}");
        let clicked = s.run("click Add").expect("click");
        assert!(clicked.contains("handled: click"), "{clicked}");
        assert!(clicked.contains("wrote [count]"), "{clicked}");
        assert!(s.run("find \"count is 1\"").expect("find").contains("1 found"));
        let listed = s.run("signals count").expect("signals");
        assert!(listed.contains("count: i32 = 1"), "{listed}");
        s.run("set count 41").expect("set");
        assert!(s.run("find \"count is 41\"").expect("find").contains("1 found"));
        assert!(s.run("set count many").is_err(), "not an i32");
        let parts = s.run("components").expect("components");
        assert!(parts.contains("#add"), "{parts}");
        let heard = s.run("a11y").expect("a11y");
        assert!(heard.contains("Button") && heard.contains("count is 41"), "{heard}");
        let node = s.run("node #add").expect("node");
        assert!(node.contains("does       click"), "{node}");
        assert!(s.run("perf").expect("perf").contains("frames"));
        assert!(s.run("bench 3").expect("bench").starts_with("3 frames"));
        assert!(s.run("click Nowhere").is_err());
        assert!(s.run("frobnicate").is_err());
    }

    /// Fifty buttons `#0`…`#49` in a list 100px tall, each counting into `hits`,
    /// and an input named only by its label.
    fn long_list() -> (Session<impl FnMut() -> Element>, Signal<Vec<usize>>) {
        let hits = Signal::labeled("hits", Vec::<usize>::new());
        let field = crate::edit::TextField::new("");
        let h = hits.clone();
        let scenario = Scenario::measuring(
            move || {
                let rows = (0..50).map(|i| {
                    let h = h.clone();
                    div()
                        .h_px(24.0)
                        .on_click(move |_| {
                            h.update(|mut v| {
                                v.push(i);
                                v
                            });
                        })
                        .child(text(format!("#{i}")))
                });
                div()
                    .size_full()
                    .flex_col()
                    .child(text("Your name"))
                    .child(crate::element::text_input(&field).aria_label("Your name").h_px(24.0))
                    .child(div().named("list").h_px(100.0).overflow_y_scroll().children(rows))
                    .into_element()
            },
            320,
            240,
            1.0,
        )
        .expect("a scenario");
        (Session::new(scenario).expect("a session"), hits)
    }

    #[test]
    fn a_click_scrolls_its_node_into_view_first() {
        let (mut s, hits) = long_list();
        let out = s.run("click \"#40\"").expect("click");
        assert!(out.contains("scrolled @"), "{out}");
        assert_eq!(hits.get(), [40], "{out}");
        // Back up to the top, and the first row takes its click too.
        s.run("click \"#0\"").expect("click");
        assert_eq!(hits.get(), [40, 0]);
        assert!(s.run("reveal \"#49\"").expect("reveal").contains("is in view"));
    }

    #[test]
    fn a_hash_selector_with_no_such_name_means_the_text() {
        let (mut s, _) = long_list();
        assert!(s.run("node #list").expect("named").contains("list: Div"));
        assert!(s.run("node \"#7\"").expect("text").contains("Text \"#7\""));
        assert!(s.run("node #nope").is_err());
    }

    #[test]
    fn a_screen_reader_action_reaches_the_node_that_takes_it() {
        let (mut s, _) = long_list();
        // "Your name" is first a label's text; the input is named by it too.
        let out = s.run("setvalue \"Your name\" Ana").expect("setvalue");
        assert!(out.contains("SetValue"), "{out}");
        assert!(s.run("find Ana").expect("find").contains("Input \"Ana\""));
    }

    #[test]
    fn a_script_echoes_each_command_and_says_whether_all_succeeded() {
        let mut s = counter();
        let mut out = Vec::new();
        let ok = s
            .run_script(
                ["# a comment", "click Add", "", "tree --depth=1"].map(str::to_owned),
                &mut out,
            )
            .expect("written");
        let out = String::from_utf8(out).expect("text");
        assert!(ok, "{out}");
        assert!(out.contains("> click Add") && out.contains("> tree --depth=1"), "{out}");
        assert!(!out.contains("a comment"));
        let failed = s.run_script(["nope".to_owned()], &mut Vec::new()).expect("written");
        assert!(!failed);
    }

    #[test]
    fn a_line_splits_on_spaces_and_keeps_quoted_words_whole() {
        assert_eq!(
            split(r#"click "Save changes" 'a b' c\ d"#).ok(),
            Some(vec![
                "click".to_owned(),
                "Save changes".to_owned(),
                "a b".to_owned(),
                "c d".to_owned()
            ])
        );
        assert!(split("click \"open").is_err());
        assert_eq!(split(r#"type """#).ok(), Some(vec!["type".to_owned(), String::new()]));
    }

    #[cfg(unix)]
    #[test]
    fn a_client_on_the_socket_is_answered_by_the_ui_thread() {
        let mut s = counter();
        let mut attached = Attached::default();
        let inbox = std::rc::Rc::clone(&attached.inbox);
        let to_ui = crate::mailbox::mailbox(move |req: Request| inbox.borrow_mut().push_back(req));
        let path = std::env::temp_dir().join(format!("gui-devtools-test-{}.sock", std::process::id()));
        listen(&path, to_ui).expect("listening");
        let client = {
            let path = path.clone();
            std::thread::spawn(move || request(&path, &["graph".to_owned(), "wait 5".to_owned()]))
        };
        // The UI thread: deliver the mail and run what arrived, as the window's idle does.
        let started = Instant::now();
        while !client.is_finished() && started.elapsed() < ANSWER_WITHIN {
            crate::mailbox::deliver();
            attached.run(s.scenario().pipeline_mut());
            std::thread::sleep(Duration::from_millis(5));
        }
        let answer = client.join().expect("the client ran").expect("answered");
        std::fs::remove_file(&path).expect("removed");
        assert!(answer.contains("> graph\nsignals "), "{answer}");
        assert!(answer.contains("error: wait runs only headless"), "{answer}");
    }

    #[test]
    fn chords_name_keys_and_modifiers() {
        let ev = key_event("shift+tab").ok();
        assert!(ev.as_ref().is_some_and(|e| e.is(NamedKey::Tab) && e.modifiers.shift));
        let ev = key_event("cmd+k").ok();
        assert!(ev.is_some_and(|e| e.modifiers.command && e.text.as_deref() == Some("k")));
        assert!(key_event("hyper+x").is_err());
    }

    #[test]
    fn type_names_lose_their_module_paths() {
        assert_eq!(
            short_type("core::option::Option<alloc::string::String>"),
            "Option<String>"
        );
    }
}
