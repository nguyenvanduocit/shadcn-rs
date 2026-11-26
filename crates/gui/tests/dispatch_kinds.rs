//! Fault injection over every kind of user code the engine reaches, by
//! every path input takes (ADR-0015, the dispatch rule; review r14 F):
//! each call of user code a scripted sequence makes panics in its turn,
//! and the run is compared with the clean one taken by the same path:
//! - the panic is not swallowed: exactly one action of the script unwinds;
//! - every piece of user code is told what it is told without the panic,
//!   the one that panicked included (the dispatch goes on past it);
//! - the state at the end is the same: every node's flags, focus and its
//!   ring, the cursor, the pointer, and where the scroll containers are.
//!
//! The kinds: handlers of every [`gui::Div`] registration (hover, pointer
//! move, press, click, release, secondary click, drag, key down, the
//! window's keys, key repeat, wheel, touch, long press, focus on press,
//! press focuses, IME preedit and commit, measured, stage), a span's
//! click, a layer's dismiss answer (a press outside, Escape, a scroll,
//! focus outside), a scroll container's reveal and layout hooks, a virtual
//! list's layout hook, a members list's watcher, a component callback, a
//! timer, a mailbox's handler (two values, delivered in a frame's turn),
//! and effects and their cleanups by every path the runtime runs them
//! ([`RUNTIME_PATHS`]: an effect's first run in a turn's work and after a
//! setup, a rerun by a user's write and in an engine commit's flush; a
//! cleanup before a rerun, as an instance unmounts and as an effect is
//! disposed). Accessibility actions (click, focus,
//! scroll into view) reach them as well. The paths: a scenario's input,
//! the platform's (queued through the render thread with the compositor's
//! pull, handled after), the devtools' (a session's commands), and input
//! sent to this window from a handler of another. The end compared holds
//! where an input method types and the layers open too.
//!
//! The kinds are the engine's ([`Site`]), which its gate
//! ([`gui::engine::call_out`]) requires: [`case`] matches every one without
//! a wildcard, so a kind the engine gains fails to compile here, and
//! [`sites_are_all_in_the_harness`] fails until the script reaches it.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
#![expect(clippy::panic, reason = "the injected fault is a panic of user code")]

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};
use std::time::Duration;

use gui::app::Scenario;
use gui::engine::Site;
use gui::events::{DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, TouchEvent, TouchPhase};
use gui::prelude::*;
use gui::window::Input;
use gui::{
    Callback, Dismiss, DismissPolicy, Effect, FocusHandle, Interaction, Layer, ListState, Members, PointerKind,
    Positioning, Preedit, ScrollHandle, Signal, deferred, div, key_of, virtual_list,
};

/// The payload of an injected panic.
struct Fault;

/// Held by one run ([`run`]) while it runs: the counters and the log below
/// are the process's, and tests run on threads of their own, so two tests'
/// runs never write them at once.
static ONE_RUN: Mutex<()> = Mutex::new(());
/// Calls of user code since the run was armed.
static CALLS: AtomicU32 = AtomicU32::new(0);
/// The call that panics; 0 for none.
static AT: AtomicU32 = AtomicU32::new(0);
/// Whether calls are counted and logged (not while the fixture is set up).
static ARMED: AtomicBool = AtomicBool::new(false);
/// Each call, in order.
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The log, whatever a panic left it as.
fn log() -> std::sync::MutexGuard<'static, Vec<String>> {
    LOG.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// User code of kind `kind` ran, as `what`: logged, and the armed call
/// panics after. The log line is `who kind what`.
fn call(who: &str, kind: &str, what: impl std::fmt::Display) {
    if !ARMED.load(SeqCst) {
        return;
    }
    let n = CALLS.fetch_add(1, SeqCst).saturating_add(1);
    log().push(format!("{who} {kind} {what}"));
    if AT.load(SeqCst) == n {
        std::panic::panic_any(Fault);
    }
}

/// Logs `what` only when it differs from the last it logged: for hooks
/// told on every layout pass.
fn changed(last: &Rc<RefCell<String>>, who: &str, kind: &str, what: String) {
    if *last.borrow() != what {
        last.replace(what.clone());
        call(who, kind, what);
    }
}

/// What the harness reaches the user code through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    /// A scenario's input.
    Scenario,
    /// The platform's: queued through the render thread, handled after.
    Platform,
    /// A devtools session's commands, for what one can send.
    Devtools,
    /// Sent to this window by a handler of another.
    Nested,
}

const PATHS: [Path; 4] = [Path::Scenario, Path::Platform, Path::Devtools, Path::Nested];

/// One step of the script.
#[derive(Clone, Copy, Debug)]
enum Step {
    Move(f32, f32),
    Down(f32, f32),
    Up(f32, f32),
    Secondary(f32, f32),
    Key(NamedKey),
    Repeat(NamedKey),
    Leave,
    Wheel(f32, f32, f32),
    Tap(f32, f32),
    LongPress(f32, f32),
    Preedit,
    Commit,
    Act(accesskit::Action, &'static str),
    /// The span's link clicked, where it lies.
    Link,
    Shift(bool),
    /// Two values sent to the mailbox, delivered by the frame after.
    Mail,
    /// An effect made in a turn's own work (its first run there), with a
    /// cleanup, kept in [`EFFECT`].
    Effect,
    /// The effect [`Step::Effect`] made, disposed: its cleanup runs.
    Dispose,
}

/// What the steps act on beyond the window: the state a step writes, the
/// fixture's mailbox ([`Step::Mail`]), the effect [`Step::Effect`] made
/// for [`Step::Dispose`].
#[derive(Clone)]
struct Ctx {
    shift: Signal<bool>,
    mail: Rc<RefCell<Option<gui::Sender<u32>>>>,
    effect: Rc<RefCell<Option<Effect>>>,
}

/// Every kind and the places the script reaches it.
const SCRIPT: &[Step] = &[
    Step::Move(10., 10.),
    Step::Move(60., 60.),
    Step::Move(110., 10.),
    Step::Down(110., 10.),
    Step::Up(110., 10.),
    Step::Preedit,
    Step::Commit,
    Step::Repeat(NamedKey::ArrowDown),
    Step::Key(NamedKey::Escape),
    Step::Down(60., 60.),
    Step::Up(60., 60.),
    Step::Down(10., 10.),
    Step::Move(30., 30.),
    Step::Up(30., 30.),
    Step::Secondary(110., 10.),
    Step::Up(110., 10.),
    // Pressed on B, released over A: A's release.
    Step::Down(110., 10.),
    Step::Move(60., 60.),
    Step::Up(60., 60.),
    Step::Key(NamedKey::Tab),
    Step::Key(NamedKey::Tab),
    Step::Wheel(60., 60., 20.),
    Step::Wheel(240., 50., 40.),
    Step::Wheel(90., 130., 40.),
    Step::Tap(20., 20.),
    Step::LongPress(20., 20.),
    Step::Act(accesskit::Action::Click, "A"),
    Step::Act(accesskit::Action::Focus, "B"),
    Step::Act(accesskit::Action::ScrollIntoView, "far"),
    Step::Link,
    Step::Shift(true),
    Step::Shift(false),
    Step::Down(190., 150.),
    Step::Up(190., 150.),
    Step::Mail,
    Step::Effect,
    Step::Move(60., 60.),
    Step::Dispose,
    Step::Leave,
    Step::Move(110., 10.),
];

/// What the fixture's code reads and the test compares.
#[derive(Clone, Copy)]
struct Parts {
    a: Interaction,
    a1: Interaction,
    b: Interaction,
    fa: FocusHandle,
    fb: FocusHandle,
    scroll: ScrollHandle,
    list: ListState,
    shift: Signal<bool>,
}

/// R (300×200) holding, placed absolutely: A (0,0 80×80, focusable, its
/// press refusing focus) with A1 (40×40, drag, touch, long press) inside;
/// B (100,0 80×80, focusable, an IME target, measured, a member, its click
/// calling a component callback); S (200,0 80×100, a scroll container
/// with a keyed row far below and a layer's owner); V (0,100 180×60, a
/// virtual list); C (100,100, while `shift`, reporting its stages); a
/// paragraph with a link at (100,170); and a layer at (250,150) taking
/// every dismiss.
fn fixture(p: Parts, members: Members<u32>) -> Scenario {
    let measured = Rc::new(RefCell::new(String::new()));
    let callback: Callback<u32, ()> = Callback::new(|n| call("B", "callback", n));
    Scenario::measuring(
        move || {
            let measured = Rc::clone(&measured);
            let callback = callback.clone();
            let a1 = div()
                .interaction(p.a1)
                .size(40.)
                .on_drag(move |e| {
                    let phase = match e.phase {
                        DragPhase::Start => "start",
                        DragPhase::Move => "move",
                        DragPhase::End => "end",
                        _ => "cancel",
                    };
                    call("A1", "drag", phase);
                })
                .on_touch(move |e| call("A1", "touch", format!("{:?}", e.phase)))
                .on_long_press(move |_| call("A1", "long_press", ""));
            let a = div()
                .aria_label("A")
                .interaction(p.a)
                .track_focus(p.fa)
                .cursor(Cursor::Text)
                .position(Position::Absolute)
                .left(0.)
                .top(0.)
                .size(80.)
                .on_hover(move |on| call("A", "hover", on))
                .on_pointer_move(move |e| call("A", "pointer_move", format!("{} {}", e.x, e.y)))
                .on_mouse_down(move |_| call("A", "mouse_down", ""))
                .on_click(move |_| {
                    // Set before the call that may panic: what is compared
                    // is the dispatch, not code after a panic.
                    gui::clock::after(Duration::from_millis(5), || call("T", "timer", ""));
                    call("A", "click", "");
                })
                .on_release(move |_| call("A", "release", ""))
                .on_wheel(move |e| {
                    call("A", "wheel", e.dy);
                    false
                })
                .focus_on_press(move |_| {
                    call("A", "focus_on_press", "");
                    false
                })
                .child(a1);
            let b = div()
                .aria_label("B")
                .interaction(p.b)
                .track_focus(p.fb)
                .cursor(Cursor::Grab)
                .position(Position::Absolute)
                // Moved and renumbered while `shift`: measured anew, and
                // the members' watcher sees another list.
                .left(if p.shift.get() { 104. } else { 100. })
                .top(0.)
                .size(80.)
                .member(&members, if p.shift.get() { 2 } else { 1 })
                .on_click(move |_| callback.call(1))
                .on_secondary_click(move |_| call("B", "secondary_click", ""))
                .on_key_down(move |k| {
                    call("B", "key_down", format!("{:?}", k.key));
                    false
                })
                .on_ime(move |e| {
                    let what = match e {
                        ImeEvent::Preedit(p) => format!("preedit {}", p.text),
                        ImeEvent::Commit(t) => format!("commit {t}"),
                    };
                    call("B", "ime", what);
                })
                .on_measured(move |b| changed(&measured, "B", "measured", format!("{b:?}")));
            let layer = Layer::new()
                .dismiss(DismissPolicy::DEFAULT.scroll(true))
                .on_dismiss(|why| {
                    call("L", "dismiss", format!("{why:?}"));
                    Dismiss::Veto
                });
            let s = div()
                .track_scroll(&p.scroll)
                .position(Position::Absolute)
                .left(200.)
                .top(0.)
                .width(80.)
                .height(100.)
                .overflow_y(Overflow::Scroll)
                .child(div().size(10.).child(deferred(
                    Positioning::At { x: 250., y: 150. },
                    div().size(40.).layer(layer),
                )))
                .child(div().height(300.))
                // Its own child, keyed: what its reveal hook is asked for.
                .child(div().key(key_of("far")).aria_label("far").focusable().height(40.));
            let v = virtual_list(
                &p.list,
                100,
                |i| key_of(&format!("r{i}")),
                |i| div().height(20.).child(gui::text(format!("row {i}"))).into_element(),
            )
            .position(Position::Absolute)
            .left(0.)
            .top(100.)
            .width(180.)
            .height(60.);
            let link = gui::element::text("").key(key_of("para")).span(
                gui::element::span("link")
                    .key(1)
                    .on_click(|_| call("P", "span_click", "")),
            );
            let mut r = div()
                .width(300.)
                .height(200.)
                .on_window_key(move |k| {
                    call("R", "window_key", format!("{:?}", k.key));
                    false
                })
                .press_focuses(move || {
                    call("R", "press_focuses", "");
                    None
                })
                .child(a)
                .child(b)
                .child(s)
                .child(v)
                .child(div().position(Position::Absolute).left(100.).top(170.).child(link));
            // D: a render calling a derivation (its kind, `Derive`), told
            // as `shift` renders it again; what it shows is the same
            // either way, so a panic held at the render shows the same.
            let shift = p.shift;
            r = r.child(gui::component((), move |_| {
                let derive = gui::component::Derive::new(|on: bool| {
                    if on {
                        call("D", "derive", "as the render runs again");
                    }
                    on
                });
                move || {
                    let _shown = derive.call(shift.get());
                    div().size(1.).into_element()
                }
            }));
            if p.shift.get() {
                r = r.child(
                    div()
                        .position(Position::Absolute)
                        .left(100.)
                        .top(100.)
                        .size(20.)
                        .on_stage(|s| call("C", "stage", format!("{s:?}"))),
                );
                // An instance shown while `shift`: its setup's effect runs
                // first once the render returns, and its cleanup as the
                // instance is unmounted.
                r = r.child(gui::component((), |_| {
                    // Two: the one that runs second is told though the
                    // first panics.
                    gui::on_unmounted(|| call("C", "cleanup", "unmount, second"));
                    gui::on_unmounted(|| call("C", "cleanup", "unmount, first"));
                    let _owned_by_the_instance = Effect::new(|| call("C", "effect", "first in a setup"));
                    || div().size(1.).into_element()
                }));
            }
            r.into_element()
        },
        300,
        200,
        1.,
    )
    .expect("scenario")
}

/// The window the steps go to, by the path taken.
enum Window {
    /// Alone, for [`Path::Scenario`] and [`Path::Platform`].
    Alone(Scenario),
    /// In a devtools session.
    Session(Box<gui::devtools::Session>),
    /// Reached from another window's handler: the outer window, the step
    /// its handler sends, and this window.
    Nested {
        outer: Scenario,
        next: Rc<Cell<Option<Step>>>,
        inner: Rc<RefCell<Scenario>>,
    },
}

/// The steps an action of the script unwound, by index, and in what part.
type Unwound = Vec<(usize, &'static str)>;

/// The inputs of the step running now that unwound: a step of several
/// inputs (a move then a wheel, a press then a release) sends each as the
/// platform does, one after another, so a panic in one leaves the next to
/// come.
static INPUTS_UNWOUND: AtomicU32 = AtomicU32::new(0);

/// Run `f`, one input of a step (or a frame or the clock within it), under
/// a catch of its own: what unwound is counted for the step.
fn alone(f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        INPUTS_UNWOUND.fetch_add(1, SeqCst);
    }
}

impl Window {
    fn new(path: Path, s: Scenario, cx: &Ctx) -> Self {
        match path {
            Path::Scenario | Path::Platform => Self::Alone(s),
            Path::Devtools => Self::Session(Box::new(gui::devtools::Session::new(s).expect("session"))),
            Path::Nested => {
                let next = Rc::new(Cell::new(None));
                let inner = Rc::new(RefCell::new(s));
                let (n, i, cx) = (Rc::clone(&next), Rc::clone(&inner), cx.clone());
                let mut outer = Scenario::measuring(
                    move || {
                        let (n, i, cx) = (Rc::clone(&n), Rc::clone(&i), cx.clone());
                        div()
                            .size(100.)
                            .on_mouse_down(move |_| {
                                if let Some(step) = n.take() {
                                    apply(&mut i.borrow_mut(), Path::Scenario, step, &cx);
                                }
                            })
                            .into_element()
                    },
                    100,
                    100,
                    1.,
                )
                .expect("outer scenario");
                outer.settle().expect("outer frame");
                Self::Nested { outer, next, inner }
            }
        }
    }

    /// The window's scenario, for reading it.
    fn read<R>(&mut self, f: impl FnOnce(&mut Scenario) -> R) -> R {
        match self {
            Self::Alone(s) => f(s),
            Self::Session(session) => f(session.scenario()),
            Self::Nested { inner, .. } => f(&mut inner.borrow_mut()),
        }
    }

    /// Run `step` by this window's path, then let a frame come, each
    /// action under a catch of its own: what unwound goes to `unwound`.
    fn run(&mut self, path: Path, k: usize, step: Step, cx: &Ctx, unwound: &mut Unwound) {
        let sent = catch_unwind(AssertUnwindSafe(|| match self {
            Self::Alone(s) => apply(s, path, step, cx),
            Self::Session(session) => devtools(session, step, cx),
            Self::Nested { outer, next, .. } => {
                next.set(Some(step));
                // The outer window's own press and release: its handler
                // sends `step` on, inside the outer dispatch.
                drop(outer.send(Input::MouseDown { x: 5., y: 5. }));
            }
        }));
        if sent.is_err() {
            unwound.push((k, "step"));
        }
        for _ in 0..INPUTS_UNWOUND.swap(0, SeqCst) {
            unwound.push((k, "input"));
        }
        if let Self::Nested { outer, .. } = self
            && catch_unwind(AssertUnwindSafe(|| drop(outer.send(Input::MouseUp { x: 5., y: 5. })))).is_err()
        {
            unwound.push((k, "outer release"));
        }
        if catch_unwind(AssertUnwindSafe(|| self.read(|s| s.settle()))).is_err() {
            unwound.push((k, "frame"));
        }
    }
}

/// Send `input` by `path`: a scenario's, or the platform's (queued, a
/// frame drawn meanwhile, then handled).
fn send(s: &mut Scenario, path: Path, input: Input) {
    alone(|| match path {
        Path::Platform => {
            let queued = s.queue(input);
            drop(s.handle(queued));
        }
        _ => drop(s.send(input)),
    });
}

/// A key press of `key`.
fn key(key: NamedKey) -> KeyEvent {
    KeyEvent {
        key: Key::Named(key),
        text: None,
        modifiers: Modifiers::default(),
    }
}

/// A touch's `phase` at `(x, y)`.
fn touch(phase: TouchPhase, x: f32, y: f32) -> Input {
    Input::Touch(TouchEvent {
        id: 1,
        kind: PointerKind::Touch,
        phase,
        x,
        y,
    })
}

/// The node a screen reader names `label`.
fn access_node(s: &Scenario, label: &str) -> accesskit::NodeId {
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "t", &mut ts);
    tree.nodes
        .iter()
        .find(|(_, n)| n.label() == Some(label))
        .map(|(id, _)| *id)
        .expect("a node by that label")
}

/// Run `step` on `s` by `path` (a scenario's or the platform's), over the
/// fixture's `cx`.
fn apply(s: &mut Scenario, path: Path, step: Step, cx: &Ctx) {
    match step {
        Step::Move(x, y) => send(s, path, Input::PointerMoved { x, y }),
        Step::Down(x, y) => send(s, path, Input::MouseDown { x, y }),
        Step::Up(x, y) => send(s, path, Input::MouseUp { x, y }),
        Step::Secondary(x, y) => send(
            s,
            path,
            Input::ButtonDown {
                x,
                y,
                button: gui::PointerButton::Secondary,
            },
        ),
        Step::Key(k) => send(s, path, Input::Key(key(k))),
        Step::Repeat(k) => send(s, path, Input::KeyRepeat(key(k))),
        Step::Leave => send(s, path, Input::PointerLeft),
        Step::Wheel(x, y, dy) => {
            send(s, path, Input::PointerMoved { x, y });
            send(s, path, Input::Wheel { dx: 0., dy });
        }
        Step::Tap(x, y) => {
            send(s, path, touch(TouchPhase::Started, x, y));
            send(s, path, touch(TouchPhase::Ended, x, y));
        }
        Step::LongPress(x, y) => {
            send(s, path, touch(TouchPhase::Started, x, y));
            // Held past the long press: its timer comes due, and the
            // click's timer queued before it.
            alone(|| s.at(gui::clock::current_seconds() + 0.8));
            alone(|| s.settle().expect("frame"));
            send(s, path, touch(TouchPhase::Ended, x, y));
        }
        Step::Preedit => send(
            s,
            path,
            Input::Ime(ImeEvent::Preedit(Preedit {
                text: "a".to_owned(),
                cursor: Some((1, 1)),
            })),
        ),
        Step::Commit => send(s, path, Input::Ime(ImeEvent::Commit("ab".to_owned()))),
        Step::Act(action, label) => {
            let target_node = access_node(s, label);
            send(
                s,
                path,
                Input::Act(accesskit::ActionRequest {
                    action,
                    target_tree: accesskit::TreeId::ROOT,
                    target_node,
                    data: None,
                }),
            );
        }
        Step::Link => {
            let (x, y) = s.locate("link").expect("the link");
            send(s, path, Input::PointerMoved { x, y });
            send(s, path, Input::MouseDown { x, y });
            send(s, path, Input::MouseUp { x, y });
        }
        // The write runs the effects that read it; what the act does after
        // is told though one of them panics.
        Step::Shift(on) => {
            let shift = cx.shift;
            s.act("shift", move || {
                shift.set(on);
                call("S", "wrote", on);
            });
        }
        Step::Mail => {
            {
                let m = cx.mail.borrow();
                let tx = m.as_ref().expect("the mailbox");
                assert!(tx.send(1) && tx.send(2));
            }
            // Delivered in the frame's turn, before it builds.
            alone(|| s.settle().expect("frame"));
        }
        // Kept only once made: should its first run unwind the turn's
        // work, it is never disposed, and its cleanup never told.
        Step::Effect => s.act("effect", || {
            let e = Effect::new(|| {
                gui::on_cleanup(|| call("E", "cleanup", "dispose"));
                call("E", "effect", "first in a turn");
            });
            *cx.effect.borrow_mut() = Some(e);
        }),
        // What the act does after the dispose is told though its cleanup
        // panics.
        Step::Dispose => s.act("dispose", || {
            let made = cx.effect.borrow_mut().take();
            if let Some(e) = made {
                e.dispose();
                call("E", "disposed", "");
            }
        }),
    }
}

/// Run `step` as a devtools session's command where one sends it (a
/// pointer's moves, presses and releases, a secondary press, keys, the
/// wheel, a screen reader's click and focus); the rest (touch, IME, key
/// repeat, scroll into view, a step of state) by the session's scenario.
fn devtools(session: &mut gui::devtools::Session, step: Step, cx: &Ctx) {
    let at = |x: f32, y: f32| format!("{x},{y}");
    let named = |k: NamedKey| match k {
        NamedKey::Tab => "tab",
        NamedKey::Escape => "esc",
        NamedKey::Enter => "enter",
        _ => "down",
    };
    let line = match step {
        Step::Move(x, y) => Some(format!("hover {}", at(x, y))),
        Step::Down(x, y) => Some(format!("down {}", at(x, y))),
        Step::Up(x, y) => Some(format!("up {}", at(x, y))),
        Step::Secondary(x, y) => Some(format!("rclick {}", at(x, y))),
        Step::Key(k) => Some(format!("press {}", named(k))),
        Step::Leave => Some("leave".to_owned()),
        Step::Wheel(x, y, dy) => Some(format!("wheel {dy} {}", at(x, y))),
        Step::Act(accesskit::Action::Click, "A") => Some("activate 60,60".to_owned()),
        Step::Act(accesskit::Action::Focus, "B") => Some("focus 110,10".to_owned()),
        _ => None,
    };
    match line {
        // What the command answers is the session's report: an injected
        // panic unwinds through it.
        Some(line) => drop(session.run(&line)),
        None => apply(session.scenario(), Path::Scenario, step, cx),
    }
}

/// What a run ends with.
#[derive(Debug, PartialEq)]
struct End {
    flags: Vec<[bool; 5]>,
    focused: [bool; 2],
    visible: bool,
    cursor: Cursor,
    pointer: Option<(f32, f32)>,
    scroll: (f32, f32),
    /// Where an input method types.
    ime: Option<String>,
    /// The layers open, bottom first.
    layers: Vec<String>,
}

/// What a run logged, what unwound, and its end.
struct Run {
    log: Vec<String>,
    unwound: Unwound,
    end: End,
}

/// The script by `path`, with call `at` panicking.
fn run(path: Path, at: Option<u32>) -> Run {
    let _one = ONE_RUN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    ARMED.store(false, SeqCst);
    let p = Parts {
        a: Interaction::new(),
        a1: Interaction::new(),
        b: Interaction::new(),
        fa: FocusHandle::new(),
        fb: FocusHandle::new(),
        scroll: ScrollHandle::new(),
        list: ListState::new(20.),
        shift: Signal::new(false),
    };
    let members: Members<u32> = Members::new();
    let watched = Rc::new(RefCell::new(String::new()));
    members.on_laid_out(move |v| changed(&watched, "M", "laid_out", format!("{v:?}")));
    let (seen_scroll, seen_list) = (
        Rc::new(RefCell::new(String::new())),
        Rc::new(RefCell::new(String::new())),
    );
    p.scroll.on_layout(move |l| {
        changed(&seen_scroll, "S", "scroll_layout", format!("{:?}", l.offset));
        false
    });
    p.scroll.on_reveal(move |k| {
        call("S", "reveal", k);
        false
    });
    p.list.on_layout(move |l| {
        changed(&seen_list, "V", "list_layout", format!("{}", l.top));
        false
    });
    let mut s = fixture(p, members);
    s.settle().expect("frame");
    let cx = Ctx {
        shift: p.shift,
        // The window's own mailbox: made in a turn of it, which owns it
        // (ADR-0015, the thread-window rule: the top level owns none).
        mail: Rc::new(RefCell::new(Some(
            s.read(|| gui::mailbox(|n: u32| call("Q", "mail", n))),
        ))),
        effect: Rc::default(),
    };
    let mut w = Window::new(path, s, &cx);
    // Effects reading the flags (focus in and out, hover and press seen by
    // user code as an engine commit's write wakes them: rerun in the flush
    // that closes the commit), one with a cleanup run before each rerun;
    // and one reading a user's signal, rerun as the user writes it.
    let mut effects: Vec<Effect> = [("A", p.a), ("A1", p.a1), ("B", p.b)]
        .into_iter()
        .map(|(name, i)| {
            Effect::new(move || {
                let f = [i.hovered(), i.pressed(), i.focused(), i.focus_within()];
                // Registered before the call that may panic.
                if name == "A" {
                    gui::on_cleanup(move || call(name, "cleanup", "before a rerun"));
                }
                call(name, "effect", format!("rerun in a commit's flush {f:?}"));
            })
        })
        .collect();
    let shift = p.shift;
    effects.push(Effect::new(move || {
        let on = shift.get();
        call("S", "effect", format!("rerun by a user's write {on}"));
    }));
    CALLS.store(0, SeqCst);
    AT.store(at.unwrap_or(0), SeqCst);
    log().clear();
    ARMED.store(true, SeqCst);
    let mut unwound = Vec::new();
    for (k, &step) in SCRIPT.iter().enumerate() {
        w.run(path, k, step, &cx, &mut unwound);
    }
    ARMED.store(false, SeqCst);
    let end = w.read(|s| End {
        flags: [p.a, p.a1, p.b]
            .iter()
            .map(|i| {
                [
                    i.hovered(),
                    i.pressed(),
                    i.focused(),
                    i.focus_visible(),
                    i.focus_within(),
                ]
            })
            .collect(),
        focused: [p.fa.is_focused(), p.fb.is_focused()],
        visible: gui::focus::focus_visible(),
        cursor: s.cursor(),
        pointer: gui::interact::pointer_position(),
        scroll: p.scroll.offset(),
        ime: s
            .ime_target()
            .map(|t| format!("B's {} at {:?}", t.field == p.fb.id(), t.area)),
        layers: s.open_layers(),
    });
    cx.mail.borrow_mut().take();
    let made = cx.effect.borrow_mut().take();
    if let Some(e) = made {
        e.dispose();
    }
    let out = Run {
        log: std::mem::take(&mut *log()),
        unwound,
        end,
    };
    drop(effects);
    out
}

/// The kind of a logged call (its second word).
fn kind_of(line: &str) -> &str {
    line.split(' ').nth(1).unwrap_or("")
}

/// Every kind the script must reach, by any path: one per gate to user
/// code and per handler registration ([`sites_are_all_in_the_harness`]).
const KINDS: &[&str] = &[
    "hover",
    "pointer_move",
    "mouse_down",
    "click",
    "release",
    "secondary_click",
    "drag",
    "key_down",
    "window_key",
    "wheel",
    "touch",
    "long_press",
    "focus_on_press",
    "press_focuses",
    "ime",
    "measured",
    "stage",
    "span_click",
    "dismiss",
    "reveal",
    "scroll_layout",
    "list_layout",
    "laid_out",
    "callback",
    "timer",
    "mail",
    "effect",
    "cleanup",
    "derive",
];

#[test]
fn a_panic_in_any_user_code_by_any_path_leaves_dispatch_as_without_it() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !info.payload().is::<Fault>() {
            prev(info);
        }
    }));
    let mut found = Vec::new();
    let mut report = Vec::new();
    for path in PATHS {
        let clean = run(path, None);
        assert!(
            clean.unwound.is_empty(),
            "{path:?}: the clean run unwound: {:?}",
            clean.unwound
        );
        if path == Path::Scenario {
            let missing: Vec<_> = KINDS
                .iter()
                .filter(|k| !clean.log.iter().any(|l| kind_of(l) == **k))
                .collect();
            assert!(
                missing.is_empty(),
                "the script reaches no {missing:?}: {:#?}",
                clean.log
            );
            let unreached: Vec<_> = RUNTIME_PATHS
                .iter()
                .filter(|p| !clean.log.iter().any(|l| l.starts_with(**p)))
                .collect();
            assert!(
                unreached.is_empty(),
                "the script reaches no effect or cleanup by {unreached:?}: {:#?}",
                clean.log
            );
        }
        let mut per_kind: BTreeMap<String, u32> = BTreeMap::new();
        let total = u32::try_from(clean.log.len()).expect("calls");
        for (site, at) in clean.log.iter().zip(1..=total) {
            *per_kind.entry(kind_of(site).to_owned()).or_default() += 1;
            let r = run(path, Some(at));
            let name = format!("{path:?} #{at} `{site}`");
            if r.unwound.len() != 1 {
                found.push(format!("{name}: unwound {:?}, not once", r.unwound));
            }
            if r.log != clean.log {
                let k = r.log.iter().zip(&clean.log).take_while(|(a, b)| a == b).count();
                let next = |l: &[String]| l.iter().skip(k).take(3).cloned().collect::<Vec<_>>();
                found.push(format!(
                    "{name}: told otherwise after call {k}: {:?} instead of {:?}",
                    next(&r.log),
                    next(&clean.log),
                ));
            }
            if r.end != clean.end {
                found.push(format!("{name}: ended {:?}, clean {:?}", r.end, clean.end));
            }
        }
        report.push(format!("{path:?}: {total} cases {per_kind:?}"));
    }
    println!("{}", report.join("\n"));
    assert!(found.is_empty(), "{} cases differ:\n{}", found.len(), found.join("\n"));
}

/// The kind the script logs a [`Site`] as: every kind of user code the
/// engine stores and calls back, matched without a wildcard, so a kind the
/// engine gains fails to compile here until the fixture reaches it.
const fn case(site: Site) -> &'static str {
    match site {
        Site::Click => "click",
        Site::MouseDown => "mouse_down",
        Site::Measured => "measured",
        Site::Drag => "drag",
        Site::KeyDown => "key_down",
        Site::WindowKey => "window_key",
        Site::SecondaryClick => "secondary_click",
        Site::Hover => "hover",
        Site::PointerMove => "pointer_move",
        Site::Wheel => "wheel",
        Site::FocusOnPress => "focus_on_press",
        Site::PressFocuses => "press_focuses",
        Site::LongPress => "long_press",
        Site::Release => "release",
        Site::Touch => "touch",
        Site::Ime => "ime",
        Site::SpanClick => "span_click",
        Site::Dismiss => "dismiss",
        Site::Reveal => "reveal",
        Site::ScrollLayout => "scroll_layout",
        Site::ListLayout => "list_layout",
        Site::LaidOut => "laid_out",
        Site::Callback => "callback",
        Site::Timer => "timer",
        Site::Mail => "mail",
        // A stage reported is one of the hooks run after layout.
        Site::AfterLayout => "stage",
        Site::Effect => "effect",
        Site::Cleanup => "cleanup",
        Site::Derive => "derive",
    }
}

/// Every path the runtime reaches an effect or a cleanup by
/// ([`Site::Effect`], [`Site::Cleanup`]), as the clean run logs it (a
/// line's start): each is a case of its own.
const RUNTIME_PATHS: &[&str] = &[
    "E effect first in a turn",
    "C effect first in a setup",
    "S effect rerun by a user's write",
    "A effect rerun in a commit's flush",
    "A cleanup before a rerun",
    "C cleanup unmount",
    "E cleanup dispose",
];

/// The source files under `dir`, recursively, by path from `root`.
fn sources(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("src") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            sources(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let name = path
                .strip_prefix(root)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            // Unit tests are no gate.
            if name.ends_with("tests.rs") || name.contains("/tests/") {
                continue;
            }
            out.push((name, std::fs::read_to_string(&path).expect("source")));
        }
    }
}

/// The code of `source`: comments and `#[cfg(test)]` modules left out.
fn code(source: &str) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = source.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        // A test module runs to the end of the file in this crate.
        if t.starts_with("#[cfg(test)]")
            && lines
                .get(i.saturating_add(1))
                .is_some_and(|n| n.trim_start().starts_with("mod "))
        {
            break;
        }
        if t.starts_with("//") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Every kind of user code the engine calls back ([`Site::ALL`], which the
/// gate requires one of) is a case of the script: its kind is among those
/// the clean run must reach ([`KINDS`]). The match in [`case`] has no
/// wildcard, so a kind added does not compile here until it has a case.
#[test]
fn sites_are_all_in_the_harness() {
    let missing: Vec<_> = Site::ALL.iter().filter(|s| !KINDS.contains(&case(**s))).collect();
    assert!(
        missing.is_empty(),
        "kinds of user code the script does not reach: {missing:?}"
    );
}

/// The kinds found by type (ADR-0015, decision 6): what the engine stored
/// as the script ran (every piece of work kept in a window goes through
/// one constructor, `engine::Stored::new`, which registers its kind; a
/// derivation registers as it is made) are each a case of the script, and
/// the script stores every kind there is.
#[test]
fn every_kind_the_engine_stores_is_a_case() {
    drop(run(Path::Scenario, None));
    let stored = gui::engine::stored_kinds();
    let uncovered: Vec<_> = stored.iter().filter(|s| !KINDS.contains(&case(**s))).collect();
    assert!(uncovered.is_empty(), "stored with no case: {uncovered:?}");
    // Every stored kind (those the engine keeps to run later, and Derive)
    // is stored by the script, so the registry is the whole list.
    let kept = [
        Site::Timer,
        Site::Mail,
        Site::AfterLayout,
        Site::Effect,
        Site::Cleanup,
        Site::Derive,
    ];
    let unstored: Vec<_> = kept.iter().filter(|s| !stored.contains(s)).collect();
    assert!(unstored.is_empty(), "kinds the script never stored: {unstored:?}");
}

/// The places outside the engine module that catch or raise an unwind of
/// their own, each with why it is not the turn's to arbitrate: none on the
/// UI thread's path from a turn to user code (ADR-0015, the turn rule).
const CATCHES: &[(&str, usize, &str)] = &[
    (
        "compositor/thread.rs",
        3,
        "the render thread: no UI turn runs there; a panic of its own becomes the thread's failure, told to the UI",
    ),
    (
        "list/view.rs",
        1,
        "a list's worker thread: a build job's panic goes back as its result",
    ),
    (
        "canvas/planner.rs",
        1,
        "the canvas planner's pool thread: a plan's panic goes back as its result",
    ),
    (
        "testing.rs",
        2,
        "the seeded test harness: names the seed of a failing run, then lets it fail",
    ),
    (
        "holder.rs",
        1,
        "F1 (ADR-0028): a holder's release while a panic unwinds, its own panic dropped",
    ),
    (
        "component.rs",
        1,
        "F1 (ADR-0034, S3): a check's cold build that panicked, raised as its string",
    ),
    (
        "app.rs",
        1,
        "F1 (ADR-0034, S3): the debug check's verdict on a cold build that panicked, raised as its string",
    ),
    (
        "compositor/replay.rs",
        2,
        "test fixtures (the replay's, ADR-0033): a replay that panics, reported as its message; \
         the fault matrix's worker threads, a worker's panic passed on as it is joined (no UI turn there)",
    ),
];

/// ADR-0015, the turn rule: one arbiter. No `catch_unwind` or
/// `resume_unwind` of the UI thread lives outside the engine module, but
/// for the places [`CATCHES`] names, each with its reason: a local catch
/// is a second arbiter, which orders panics on its own.
#[test]
fn catching_lives_in_the_engine() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &root, &mut files);
    let mut found: BTreeMap<String, usize> = BTreeMap::new();
    for (name, source) in &files {
        if name == "engine.rs" || name.starts_with("engine/") {
            continue;
        }
        let code = code(source);
        let n = code.matches("catch_unwind(").count() + code.matches("resume_unwind(").count();
        if n > 0 {
            found.insert(name.clone(), n);
        }
    }
    let allowed: BTreeMap<String, usize> = CATCHES.iter().map(|(f, n, _)| ((*f).to_owned(), *n)).collect();
    assert_eq!(
        found, allowed,
        "a catch or a raise outside the engine the list does not name"
    );
}

/// ADR-0015, the turn rule: the window current is written only by the
/// guard a turn, or work a window scheduled, enters it with (and by a
/// window closing): its cell lives in a module of `focus.rs` of its own,
/// and nothing else names it.
#[test]
fn the_window_current_is_written_by_the_guard_alone() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    sources(&root, &root, &mut files);
    for (name, source) in &files {
        let code = code(source);
        let uses = code.matches("CURRENT.").count();
        if name == "focus.rs" {
            let (inner, rest) = code.split_at(code.find("use cx::current;").expect("the guard's module ends"));
            let start = inner.find("mod cx {").expect("the guard's module");
            assert_eq!(
                inner[start..].matches("CURRENT.").count(),
                uses,
                "the window current named outside the guard's module"
            );
            assert!(
                !rest.contains("CURRENT"),
                "the window current named outside the guard's module"
            );
        } else {
            assert!(
                !code.contains("focus::CURRENT") && !(code.contains("CURRENT.") && code.contains("FocusRoot")),
                "{name} names the window current"
            );
        }
    }
}
