//! Fault injection over dispatch (ADR-0015, the hover transaction; review
//! r13 addendum): every user callback a scripted input sequence reaches —
//! hover, pointer move, press, click, release, drag, secondary click,
//! keys, and effects reading the `Interaction` flags (focus in and out
//! among them) — panics at its Nth call, for every N the sequence makes.
//! Each run is compared with the run where nothing panics:
//! - the panic is not swallowed: the step it is in unwinds;
//! - every callback is told what it is told without the panic, the one
//!   that panicked included (Chrome reports a listener's exception and
//!   still dispatches to the others);
//! - once the same inputs follow, the state is the same: every node's
//!   hovered, pressed, focused, focus-visible and focus-within flags as
//!   the next frame builds them (hover and press paths), and the cursor.

#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]
#![expect(clippy::panic, reason = "the injected fault is a panic of user code")]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

use gui::app::Scenario;
use gui::events::{DragPhase, Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Effect, Interaction, Signal, div};

/// The payload of an injected panic.
struct Fault;

/// Calls of user code since the run was armed (the one test of this
/// binary runs its cases one after another, on one thread).
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

/// User code `what` ran: logged, and the armed call panics after.
fn call(what: String) {
    if !ARMED.load(SeqCst) {
        return;
    }
    let n = CALLS.fetch_add(1, SeqCst).saturating_add(1);
    log().push(what);
    if AT.load(SeqCst) == n {
        std::panic::panic_any(Fault);
    }
}

/// The nodes' interactions: the root, A, A's child A1, and B.
#[derive(Clone, Copy)]
struct Nodes {
    root: Interaction,
    a: Interaction,
    a1: Interaction,
    b: Interaction,
}

impl Nodes {
    fn all(self) -> [(&'static str, Interaction); 4] {
        [("R", self.root), ("A", self.a), ("A1", self.a1), ("B", self.b)]
    }

    /// Every flag of every node.
    fn flags(self) -> Vec<(&'static str, [bool; 5])> {
        self.all()
            .into_iter()
            .map(|(n, i)| {
                (
                    n,
                    [
                        i.hovered(),
                        i.pressed(),
                        i.focused(),
                        i.focus_visible(),
                        i.focus_within(),
                    ],
                )
            })
            .collect()
    }
}

/// A node that tells each of its callbacks to [`call`].
fn node(name: &'static str, i: Interaction, cursor: Cursor) -> gui::element::Div {
    div()
        .interaction(i)
        .cursor(cursor)
        .on_hover(move |on| call(format!("{name} hover {on}")))
        .on_pointer_move(move |e| call(format!("{name} move {} {}", e.x, e.y)))
        .on_mouse_down(move |_| call(format!("{name} down")))
        .on_click(move |_| call(format!("{name} click")))
        .on_release(move |_| call(format!("{name} release")))
        .on_secondary_click(move |_| call(format!("{name} secondary")))
        .on_key_down(move |k| {
            call(format!("{name} key {:?}", k.key));
            // Declined: it goes on up, and Tab moves focus.
            false
        })
}

/// R (200×100) holding A (80×80, holding A1, 40×40, which drags) and B
/// (80×80, at x 100, or over A while `shift`): two siblings and a nested
/// chain, every one focusable, each with its cursor.
fn fixture(n: Nodes, shift: Signal<bool>) -> Scenario {
    Scenario::measuring(
        move || {
            let a1 = node("A1", n.a1, Cursor::Pointer)
                .focusable()
                .size(40.)
                .on_drag(move |e| {
                    let phase = match e.phase {
                        DragPhase::Start => "start",
                        DragPhase::Move => "move",
                        DragPhase::End => "end",
                        _ => "cancel",
                    };
                    call(format!("A1 drag {phase}"));
                });
            let a = node("A", n.a, Cursor::Text).focusable().size(80.).child(a1);
            let b = node("B", n.b, Cursor::Grab)
                .focusable()
                .position(Position::Absolute)
                .left(if shift.get() { 0. } else { 100. })
                .top(0.)
                .size(80.);
            node("R", n.root, Cursor::Default)
                .width(200.)
                .height(100.)
                .child(a)
                .child(b)
                .into_element()
        },
        200,
        100,
        1.,
    )
    .expect("scenario")
}

/// One input, or a frame that moves a node (B over A, or back).
#[derive(Clone, Copy)]
enum Step {
    Move(f32, f32),
    Down(f32, f32),
    Up(f32, f32),
    Secondary(f32, f32),
    Key(NamedKey),
    Leave,
    Shift(bool),
}

/// Moves between siblings and along a nested chain, a click, a drag and
/// its release, a secondary press, focus moved by a press and by Tab, a
/// key, frames that move a node under a resting pointer (converge and
/// rehover), leaving and coming back. Each input is sent alone, so a
/// panic in one leaves the next to come.
const SCRIPT: &[Step] = &[
    Step::Move(10., 10.),
    Step::Move(60., 10.),
    Step::Move(110., 10.),
    Step::Move(10., 10.),
    Step::Down(10., 10.),
    Step::Up(10., 10.),
    Step::Down(10., 10.),
    Step::Move(60., 60.),
    Step::Up(60., 60.),
    Step::Move(110., 10.),
    Step::Secondary(110., 10.),
    Step::Up(110., 10.),
    Step::Key(NamedKey::Tab),
    Step::Key(NamedKey::Tab),
    Step::Key(NamedKey::Enter),
    Step::Shift(true),
    Step::Move(30., 30.),
    Step::Shift(false),
    Step::Leave,
    Step::Move(10., 10.),
    Step::Move(110., 10.),
    Step::Down(110., 10.),
    Step::Up(110., 10.),
];

/// Send `step` as the window does.
fn send(s: &mut Scenario, shift: Signal<bool>, step: Step) {
    use gui::window::Input;
    match step {
        Step::Move(x, y) => s.hover(x, y),
        Step::Down(x, y) => drop(s.send(Input::MouseDown { x, y })),
        Step::Up(x, y) => drop(s.send(Input::MouseUp { x, y })),
        Step::Secondary(x, y) => drop(s.send(Input::ButtonDown {
            x,
            y,
            button: gui::layer::PointerButton::Secondary,
        })),
        Step::Key(k) => s.press(k, Modifiers::default()),
        Step::Leave => s.leave(),
        Step::Shift(on) => s.act("shift", move || shift.set(on)),
    }
}

/// What a run ends with: the log, the steps a panic unwound (by index,
/// with whether the frame after it did), the flags and the cursor.
struct Run {
    log: Vec<String>,
    unwound: Vec<(usize, bool)>,
    flags: Vec<(&'static str, [bool; 5])>,
    cursor: Cursor,
}

/// The script, with call `at` panicking.
fn run(at: Option<u32>) -> Run {
    ARMED.store(false, SeqCst);
    let n = Nodes {
        root: Interaction::new(),
        a: Interaction::new(),
        a1: Interaction::new(),
        b: Interaction::new(),
    };
    let shift = Signal::new(false);
    let mut s = fixture(n, shift);
    s.settle().expect("frame");
    // Effects reading the flags: focus in and out, hover and press seen
    // by user code as a signal write runs them.
    let effects: Vec<Effect> = n
        .all()
        .into_iter()
        .map(|(name, i)| {
            Effect::new(move || {
                let f = [i.hovered(), i.pressed(), i.focused(), i.focus_within()];
                call(format!("{name} effect {f:?}"));
            })
        })
        .collect();
    CALLS.store(0, SeqCst);
    AT.store(at.unwrap_or(0), SeqCst);
    log().clear();
    ARMED.store(true, SeqCst);
    let mut unwound = Vec::new();
    for (k, &step) in SCRIPT.iter().enumerate() {
        if catch_unwind(AssertUnwindSafe(|| send(&mut s, shift, step))).is_err() {
            unwound.push((k, false));
        }
        if catch_unwind(AssertUnwindSafe(|| s.settle())).is_err() {
            unwound.push((k, true));
        }
    }
    ARMED.store(false, SeqCst);
    let out = Run {
        log: std::mem::take(&mut *log()),
        unwound,
        flags: n.flags(),
        cursor: s.cursor(),
    };
    drop(effects);
    out
}

#[test]
fn a_panic_in_any_callback_leaves_dispatch_as_it_was_without_it() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if !info.payload().is::<Fault>() {
            prev(info);
        }
    }));
    let clean = run(None);
    assert!(clean.unwound.is_empty(), "the clean run unwound: {:?}", clean.unwound);
    let total = u32::try_from(clean.log.len()).expect("calls");
    assert!(total > 50, "the script reaches the callbacks: {:?}", clean.log);
    let mut found = Vec::new();
    for (site, at) in clean.log.iter().zip(1..=total) {
        let r = run(Some(at));
        if r.unwound.len() != 1 {
            found.push(format!("#{at} `{site}`: unwound {:?}, not once", r.unwound));
        }
        if r.log != clean.log {
            let k = r.log.iter().zip(&clean.log).take_while(|(a, b)| a == b).count();
            let next = |l: &[String]| l.iter().skip(k).take(3).cloned().collect::<Vec<_>>();
            found.push(format!(
                "#{at} `{site}`: told otherwise after call {k}: {:?} instead of {:?}",
                next(&r.log),
                next(&clean.log),
            ));
        }
        if r.flags != clean.flags {
            found.push(format!("#{at} `{site}`: flags {:?}, clean {:?}", r.flags, clean.flags));
        }
        if r.cursor != clean.cursor {
            found.push(format!(
                "#{at} `{site}`: cursor {:?}, clean {:?}",
                r.cursor, clean.cursor
            ));
        }
    }
    println!("{total} cases (one per call of user code the script makes)");
    assert!(
        found.is_empty(),
        "{} of {total} cases differ:\n{}",
        found.len(),
        found.join("\n")
    );
}
