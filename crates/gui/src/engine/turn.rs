//! The turn (ADR-0015, the turn rule): the one way work from outside (the
//! platform's input with the compositor's scroll handed on with it, mail,
//! timers, a frame, the devtools, a window made, an accessibility action)
//! enters the UI thread, and the one place the engine catches, orders and
//! raises the panics of the user code it reaches.
//!
//! - [`turn`] enters its window (the one guard, [`Entering`]) and opens a
//!   dispatch; nothing else does either. A turn inside another (input sent
//!   to a second window from a handler) shares the outer dispatch.
//! - The dispatch has one arbiter, total over unwinds: every panic that
//!   reaches it (from the gate, [`call_out`]; from a loop that runs every
//!   piece though one panics, [`Gather`]; from a render, [`attempt`]; or
//!   the engine's own) is ordered by time, the first goes on once the turn
//!   is done, and the others are dropped outside the arbiter's borrow,
//!   each under a catch of its own ([`drop_quietly`]).
//! - Work scheduled in a window (a timer, an effect, a mail handler) keeps
//!   that window ([`Here`]) and runs in it, whatever window runs the turn.
//!
//! No `catch_unwind` or `resume_unwind` of the UI thread lives outside the
//! engine module (`tests/dispatch_kinds.rs`, `catching_lives_in_the_engine`).

use std::any::Any;
use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::rc::Rc;

use crate::focus::FocusRoot;

/// What a panic carries.
pub(crate) type Payload = Box<dyn Any + Send>;

crate::engine::engine_local! {
    /// The dispatch under way on this thread: `Some` while one runs,
    /// holding the first panic it was handed.
    [Fresh] static TELLING: RefCell<Option<Option<Payload>>> = RefCell::new(None);
    /// The dispatch under way, as an identity no other dispatch shares
    /// ([`ThisTurn`]): an id taken as it opens (no allocation: a turn of
    /// input allocates nothing), cleared as it closes.
    [Fresh] static THIS_TURN: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
    /// How many engine commit steps ([`commit`]) run now, one inside
    /// another.
    [Fresh] static COMMITTING: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Leave to enter a window: made only here, by [`turn`] and [`Here::run`],
/// so no other code makes a window current (`focus::enter_window`).
pub(crate) struct Entering(());

/// Leave to act as part of a turn (ADR-0015, the turn rule): made only in
/// this module, by a turn as it opens ([`turn`], [`in_turn`]), and handed
/// to its work by reference, so it never outlives it. Every engine step
/// that wakes user code takes one: an engine commit ([`commit`]), and so
/// every write of an engine-owned signal (a window's size, visibility and
/// activity, the pointer, focus, a node's flags, the modality), and the
/// platform's own entries (`Pipeline::told`). Code with no turn to show
/// does not compile; what has none opens one ([`in_turn`]).
pub struct Turn(());

/// Run `work`, from outside the UI thread's own code, as one turn of
/// `window`: the window is current while it runs (and the one current
/// before is put back after, when this turn runs inside another's code),
/// and it is one dispatch: user code that panics stops nothing else of it,
/// and the first panic in time goes on once it is done, its effects run.
/// What the work answered; `None` when a panic ended it, which a turn
/// inside another's sees (the outer one's arbiter has the panic): the
/// outermost one unwinds with the first panic instead.
///
/// A closed window runs no turn (ADR-0015, the window state machine): its
/// work is dropped at once, where the caller runs, never entered, and the
/// answer is `None`. Its owner's public entries say so to their caller
/// (`Pipeline::turn_with`).
pub(crate) fn turn<R>(window: &Rc<FocusRoot>, work: impl FnOnce(&Turn) -> R) -> Option<R> {
    if window.is_closed() {
        drop(work);
        return None;
    }
    let _window = crate::focus::enter_window(window, &Entering(()));
    dispatch(|| work(&Turn(())))
}

/// [`turn`], its own first panic handed back rather than raised: a
/// window's close (`Pipeline::close`), which reports what its close turn
/// raised as its result, at the top and nested alike (ADR-0015, rule 15).
/// Nested in another turn, the close turn keeps an arbiter of its own
/// while it runs, so the panics of its own code are told apart from the
/// outer turn's; its first is handed back here, for the caller to give to
/// the outer arbiter in its place in time ([`raise`]). At the top, its
/// effects are run first, as a turn's are. `Err` too, its work dropped
/// unrun, for a closed window.
pub(crate) fn turn_own<R>(window: &Rc<FocusRoot>, work: impl FnOnce(&Turn) -> R) -> Result<R, Option<Payload>> {
    if window.is_closed() {
        drop(work);
        return Err(None);
    }
    let _window = crate::focus::enter_window(window, &Entering(()));
    let (out, first) = dispatch_own(|| work(&Turn(())));
    match (out, first) {
        (_, Some(p)) => Err(Some(p)),
        (Some(out), None) => Ok(out),
        (None, None) => Err(None),
    }
}

/// Run `work` as part of the turn under way, or, with none, as a turn of
/// the thread's own window (code reached from outside with no window
/// named: a test's top level, a write a caller makes outside any handler,
/// ADR-0015, the thread-window rule): never of the window whose turn ran
/// last, and the turn rule has no bare mode. Inside a fork (a check, which
/// is a transaction of its own and runs no user code's effects) it runs as
/// part of the fork. What `work` answered; `None` when it panicked inside
/// another window's turn (whose arbiter has the panic).
pub(crate) fn in_turn<R>(work: impl FnOnce(&Turn) -> R) -> Option<R> {
    if dispatching() || super::forking() {
        return Some(work(&Turn(())));
    }
    thread_turn(|| work(&Turn(())))
}

/// A turn of the thread's own window ([`in_turn`]): it is pushed on the
/// window stack as any window is, so a turn of another window inside it
/// pops back to it, and it makes no window's signals.
fn thread_turn<R>(work: impl FnOnce() -> R) -> Option<R> {
    let _thread = crate::focus::enter_thread(&Entering(()));
    dispatch(work)
}

/// The turn under way when it was taken ([`Self::now`]), held without
/// keeping it: [`Self::is_now`] says whether that turn still runs. What a
/// turn decides for the rest of itself alone (a root a panic tore down
/// stays down until the turn is done, [`crate::component::Root`]) lasts
/// exactly that long, never into the next turn, and outside any turn it
/// never holds.
/// Each turn's id is its own ([`crate::engine::unique_id`]: never handed
/// out twice live, and a fork's from a block of its own).
#[derive(Clone, Copy, Default)]
pub(crate) struct ThisTurn(Option<u64>);

/// The ids of turns.
static TURNS: crate::engine::IdKind = crate::engine::IdKind::new("turn");

impl ThisTurn {
    /// The turn under way (none outside one).
    pub(crate) fn now() -> Self {
        Self(THIS_TURN.try_with(std::cell::Cell::get).ok().flatten())
    }

    /// Whether the turn taken still runs: not a later one, nor a fork's.
    pub(crate) fn is_now(&self) -> bool {
        self.0.is_some() && Self::now().0 == self.0
    }
}

/// Whether a turn runs on this thread.
pub(crate) fn dispatching() -> bool {
    TELLING.try_with(|t| t.borrow().is_some()).unwrap_or(false)
}

/// One dispatch: see [`turn`], which alone opens one.
fn dispatch<R>(work: impl FnOnce() -> R) -> Option<R> {
    if dispatching() {
        // Part of the outer one: whatever unwinds is one more panic of it.
        return match attempt(work) {
            Ok(out) => Some(out),
            Err(failed) => {
                failed.report();
                None
            }
        };
    }
    let (out, first) = dispatch_own(work);
    if let Some(p) = first {
        resume_unwind(p);
    }
    out
}

/// [`dispatch`], its first panic handed back: at the top, the turn's;
/// nested, that of the work alone, kept by an arbiter of its own while it
/// runs (the outer one's put back after, untouched), so the caller can
/// hand it on ([`turn_own`]).
fn dispatch_own<R>(work: impl FnOnce() -> R) -> (Option<R>, Option<Payload>) {
    if dispatching() {
        let outer = TELLING.with(|t| t.borrow_mut().replace(None));
        let out = match catch_unwind(AssertUnwindSafe(work)) {
            Ok(out) => Some(out),
            Err(p) => {
                raise(p);
                None
            }
        };
        let own = TELLING
            .with(|t| std::mem::replace(&mut *t.borrow_mut(), outer))
            .flatten();
        return (out, own);
    }
    TELLING.with(|t| *t.borrow_mut() = Some(None));
    THIS_TURN.with(|t| t.set(Some(crate::engine::unique_id(&TURNS))));
    let out = match catch_unwind(AssertUnwindSafe(work)) {
        Ok(out) => Some(out),
        Err(p) => {
            raise(p);
            None
        }
    };
    // What a panic left queued runs inside the dispatch, its panics kept
    // by the same arbiter: the web's microtask checkpoint.
    if let Err(p) = catch_unwind(crate::reactive::flush_unbatched) {
        raise(p);
    }
    let first = TELLING.with(|t| t.borrow_mut().take()).flatten();
    THIS_TURN.with(|t| t.set(None));
    (out, first)
}

/// What a panic said, for a caller's report (its payload kept).
pub(crate) fn said_of(p: &Payload) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic".to_owned())
}

/// Defines [`Site`] and its list, [`Site::ALL`], from one table: a kind
/// added is in both.
macro_rules! sites {
    ($($(#[$meta:meta])* $site:ident,)*) => {
        /// Every kind of user code the engine stores and calls back,
        /// through the gate ([`call_out`]), which takes one: a closure the
        /// engine keeps is reached by naming its kind. The fault harness
        /// (`tests/dispatch_kinds.rs`) matches it without a wildcard, so a
        /// kind added fails to compile there until the harness reaches it.
        /// Effects and cleanups, which the reactive runtime runs of its
        /// own, are reached through [`reach`], which takes a kind too.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Site {
            $($(#[$meta])* $site,)*
        }

        impl Site {
            /// Every kind, in the order declared.
            pub const ALL: &'static [Self] = &[$(Self::$site,)*];
        }
    };
}

sites! {
    /// A node's click handler ([`crate::Div::on_click`]).
    Click,
    /// A press on a node.
    MouseDown,
    /// A node's box measured after layout ([`crate::Div::on_measured`]).
    Measured,
    /// A drag.
    Drag,
    /// A key down while focus is inside a node.
    KeyDown,
    /// A key down in the window no node took.
    WindowKey,
    /// A secondary click.
    SecondaryClick,
    /// The pointer came over a node or left it.
    Hover,
    /// The pointer moved over a node.
    PointerMove,
    /// A wheel over a node.
    Wheel,
    /// Whether a press focuses a node.
    FocusOnPress,
    /// What a press on a node focuses.
    PressFocuses,
    /// A long press.
    LongPress,
    /// A press released.
    Release,
    /// A touch.
    Touch,
    /// An input method's preedit or commit.
    Ime,
    /// A span's click (a link).
    SpanClick,
    /// A layer's answer to a dismiss.
    Dismiss,
    /// A scroll container's reveal.
    Reveal,
    /// A scroll container's layout hook.
    ScrollLayout,
    /// A virtual list's layout hook.
    ListLayout,
    /// A members list's watcher.
    LaidOut,
    /// A component's callback.
    Callback,
    /// A timer's work.
    Timer,
    /// A mailbox's handler.
    Mail,
    /// A hook run after layout (a stage reported, `on_mounted`, `on_updated`).
    AfterLayout,
    /// An effect's run: its first (made in a turn, or after the render or
    /// setup that made it) or a rerun (a write woke it, a user's or an
    /// engine commit's).
    Effect,
    /// A cleanup: before its effect reruns, or as its owner is disposed
    /// (an instance unmounted, `on_unmounted` among them; an effect or a
    /// scope disposed).
    Cleanup,
    /// A derivation a render calls inline ([`crate::component::Derive`]):
    /// its panic is the render's, contained at the instance (the render
    /// keeps its last output), never run apart from it.
    Derive,
}

/// The one gate a caller's closure is reached through (ADR-0028, rule 5):
/// `f` runs now, outside a fork; inside one it does not run (`None`). A
/// closure a caller hands the engine (a callback, a hook, a timer's work,
/// a handler) writes what it captured, which no rollback puts back, so a
/// fork never calls one: it is the outside world. Where the engine needs
/// an answer, it takes the one `None` stands for, said at the call site.
///
/// Inside a turn, a closure that panics answers `None` too (no answer: a
/// dismiss not taken, a handler that did not consume), its panic goes to
/// the turn's arbiter, and the effects its own batch left queued as it
/// unwound run now. A site that must tell the two `None`s apart asks
/// [`super::forking`].
///
/// `_site` names the kind of user code `f` is ([`Site`]).
pub fn call_out<R>(site: Site, f: impl FnOnce() -> R) -> Option<R> {
    if super::forking() {
        return None;
    }
    reach(site, f)
}

/// The gate for user code the reactive runtime runs of its own (an
/// effect, [`Site::Effect`]; a cleanup, [`Site::Cleanup`]): inside a turn,
/// as [`call_out`], its panic to the arbiter and `None`, so the turn goes
/// on past it, whatever path reached it (a flush's loop, a first run in
/// the turn's own work, a settle's). Whether a fork runs it is the
/// runtime's to say (a cold frame runs none), so it asks no fork here.
///
/// With no turn under way it opens one ([`in_turn`]): user code never runs
/// bare, whatever path reached it.
pub(crate) fn reach<R>(_site: Site, f: impl FnOnce() -> R) -> Option<R> {
    // User code runs: no check opens inside it (ADR-0035, D4).
    let f = move || super::lend(f);
    if !dispatching() {
        if super::forking() {
            return Some(f());
        }
        return in_turn(|_| reached(f)).flatten();
    }
    reached(f)
}

/// Run `f`, user code inside a turn: its panic to the arbiter and `None`.
fn reached<R>(f: impl FnOnce() -> R) -> Option<R> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(p) => {
            raise(p);
            contained(crate::reactive::flush_unbatched);
            None
        }
    }
}

/// The arbiter: a panic of user code (or the engine's) caught on its way.
/// Panics are ordered by when their unwind began, not by when they reach
/// it: the first is kept to go on at the turn's end, and a later one is
/// dropped, outside the arbiter's borrow; outside a turn, raised now.
///
/// A panic caught while the thread already unwinds
/// (`std::thread::panicking`) began after that unwind: a destructor's,
/// run by it. It always loses, whether or not the earlier one has reached
/// the arbiter yet, and is never resumed (Rust aborts on a panic that
/// leaves a destructor during an unwind; the first is the one that goes
/// on, ADR-0015).
pub(crate) fn raise(p: Payload) {
    enum Then {
        Kept,
        Lost(Payload),
        Again(Payload),
    }
    if std::thread::panicking() {
        drop_quietly(p);
        return;
    }
    let then = TELLING
        .try_with(|t| match &mut *t.borrow_mut() {
            Some(first @ None) => {
                *first = Some(p);
                Then::Kept
            }
            Some(Some(_)) => Then::Lost(p),
            None => Then::Again(p),
        })
        .unwrap_or_else(|_| Then::Kept);
    match then {
        Then::Kept => {}
        Then::Lost(p) => drop_quietly(p),
        Then::Again(p) => resume_unwind(p),
    }
}

/// A panic `p` of one of several pieces of user code that each run, though
/// one panics: inside a turn, to its arbiter now, as it is caught; outside
/// one, kept in `first` (the first of them) for the caller to raise once
/// all have run, a later one dropped quietly.
pub(crate) fn caught(p: Payload, first: &mut Option<Payload>) {
    if dispatching() || std::thread::panicking() {
        raise(p);
    } else if first.is_none() {
        *first = Some(p);
    } else {
        drop_quietly(p);
    }
}

/// How many times a payload whose drop panics is dropped again (that
/// panic's own payload) before the rest is leaked.
const DROP_ROUNDS: usize = 8;

/// Drop a panic's payload that lost (a later one), under a catch of its
/// own: user data whose drop may reach the engine (the gate, a signal)
/// or panic in turn, never while a borrow of the engine's is held.
pub(crate) fn drop_quietly(p: Payload) {
    let mut next = Some(p);
    for _ in 0..DROP_ROUNDS {
        let Some(p) = next.take() else { return };
        if let Err(again) = catch_unwind(AssertUnwindSafe(move || drop(p))) {
            next = Some(again);
        }
    }
    if let Some(p) = next {
        // A payload whose every drop panics again: leaked, never dropped
        // where its panic would abort the process.
        #[expect(
            clippy::mem_forget,
            reason = "a payload whose drop keeps panicking is leaked, not dropped"
        )]
        std::mem::forget(p);
    }
}

/// Several pieces of user code that each run, though one panics (effects,
/// cleanups, hooks after a layout, mail handlers, timers): each panic goes
/// to the arbiter as it is caught ([`caught`]); outside a turn, the first
/// goes on at [`Self::finish`].
#[must_use = "finish it: outside a turn, the first panic goes on there"]
pub(crate) struct Gather(Option<Payload>);

impl Gather {
    pub(crate) const fn new() -> Self {
        Self(None)
    }

    /// Run one piece; `None` when it panicked.
    pub(crate) fn run<R>(&mut self, f: impl FnOnce() -> R) -> Option<R> {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => Some(r),
            Err(p) => {
                caught(p, &mut self.0);
                None
            }
        }
    }

    /// One more panic of them, caught elsewhere (a release's, F1).
    pub(crate) fn add(&mut self, p: Payload) {
        caught(p, &mut self.0);
    }

    /// All have run: outside a turn, the first panic goes on.
    pub(crate) fn finish(mut self) {
        if let Some(p) = self.0.take() {
            resume_unwind(p);
        }
    }
}

impl Drop for Gather {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            drop_quietly(p);
        }
    }
}

/// A panic [`attempt`] caught, for its caller to act on (a render's: the
/// root poisoned) before it goes to the arbiter.
pub(crate) struct Failed(Payload);

impl Failed {
    /// To the arbiter ([`raise`]): inside a turn, kept or dropped by time;
    /// outside one, raised now.
    pub(crate) fn report(self) {
        raise(self.0);
    }

    /// Dropped quietly ([`drop_quietly`]): a teardown's panic while another
    /// unwinds, which came after it and so never goes on.
    pub(crate) fn dismiss(self) {
        drop_quietly(self.0);
    }

    /// What it said, for a reply (the devtools'), its payload dropped.
    pub(crate) fn message(self) -> String {
        let said = self
            .0
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| self.0.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic".to_owned());
        drop_quietly(self.0);
        said
    }
}

impl Failed {
    /// Let it go on now, unwinding, as it was (a check's own failure, its
    /// cleanup done: the caller's turn catches it as one more panic).
    pub(crate) fn go_on(self) -> ! {
        go_on(self.0)
    }
}

/// Let the panic `p` go on now, unwinding (see [`Failed::go_on`]).
pub(crate) fn go_on(p: Payload) -> ! {
    resume_unwind(p)
}

/// Run `f`, its panic given back to act on before it is reported.
pub(crate) fn attempt<R>(f: impl FnOnce() -> R) -> Result<R, Failed> {
    catch_unwind(AssertUnwindSafe(f)).map_err(Failed)
}

/// Run `f`, a teardown that goes on whatever: inside a turn, its panic
/// goes to the arbiter; outside one, it is dropped (one already goes on).
pub(crate) fn contained<R>(f: impl FnOnce() -> R) -> Option<R> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(p) => {
            if dispatching() {
                raise(p);
            } else {
                drop_quietly(p);
            }
            None
        }
    }
}

/// The window current where work is scheduled (a timer set, an effect, a
/// cleanup, a hook or a mailbox made: [`Stored`]), kept with it as its
/// reactive owner is, and current again while it runs ([`Self::run`]),
/// whatever window runs the turn then. Held weakly, as the work belongs to
/// the window and not the other way round: work never keeps a window that
/// closed alive, and never runs once it has ([`FocusRoot::close`]).
///
/// Always a window, by type: the one whose code runs, or, where none's
/// does (the reactive core used with no window, a test's top level before
/// any window's turn), the thread's own (`focus`'s `NOWHERE`, made once,
/// never closed). So work never runs "in whatever window is current" at
/// the time it fires: there is no "no window" to fall back on.
/// Private to this module: the one way to keep it is [`Stored`].
#[derive(Clone)]
/// `None` is the thread's own window (`focus` `NOWHERE`), which never
/// closes nor is replaced: work made with no window current, kept without
/// making that window (the reactive core used alone makes none).
struct Here(Option<std::rc::Weak<FocusRoot>>);

impl Here {
    /// The window current now, to run scheduled work in later.
    fn now() -> Self {
        Self(crate::focus::entered_window().as_ref().map(Rc::downgrade))
    }

    /// See [`Stored::run`].
    fn run<R>(&self, site: Site, f: impl FnOnce() -> R) -> Option<R> {
        let Some(weak) = &self.0 else {
            // The thread's own: its turn, or, inside another window's
            // code, the thread's own entered on the window stack.
            if !dispatching() && !super::forking() {
                return thread_turn(|| reach(site, f)).flatten();
            }
            let _thread = (crate::focus::entered_window().is_some()).then(|| crate::focus::enter_thread(&Entering(())));
            return reach(site, f);
        };
        // Its window while it is open or closing (its close runs what its
        // cleanups make in it); once it closed, its work was dropped by
        // that close, and what still names it never runs.
        let window = weak.upgrade().filter(|w| !w.is_closed())?;
        Self::run_in(&window, site, f)
    }

    /// Run `f` in `window`: the turn under way, `window` pushed on the
    /// window stack (popped after, whatever it ran from), or a turn of
    /// `window` of its own.
    fn run_in<R>(window: &Rc<FocusRoot>, site: Site, f: impl FnOnce() -> R) -> Option<R> {
        if !dispatching() && !super::forking() {
            return turn(window, |_| reach(site, f)).flatten();
        }
        let _window = (!crate::focus::is_current(window)).then(|| crate::focus::enter_window(window, &Entering(())));
        reach(site, f)
    }
}

crate::engine::engine_local! {
    /// The kinds of user code stored on this thread so far ([`Stored::new`],
    /// a bit per [`Site`]): what the fault harness finds by type, so a
    /// kind stored with no case of its own fails it ([`stored_kinds`]).
    [Fork] static KINDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// User code of kind `site` is stored ([`Stored::new`], and a closure a
/// render calls inline: [`crate::component::Derive`]).
pub(crate) fn stored_kind(site: Site) {
    let bit = Site::ALL
        .iter()
        .position(|s| *s == site)
        .and_then(|k| u32::try_from(k).ok())
        .and_then(|k| 1_u64.checked_shl(k))
        .unwrap_or(0);
    KINDS.try_with(|k| k.set(k.get() | bit)).ok();
}

/// The kinds of user code stored on this thread so far ([`Stored`]).
#[must_use]
pub fn stored_kinds() -> Vec<Site> {
    let bits = KINDS.try_with(std::cell::Cell::get).unwrap_or(0);
    Site::ALL
        .iter()
        .enumerate()
        .filter(|(k, _)| {
            u32::try_from(*k)
                .ok()
                .and_then(|k| 1_u64.checked_shl(k))
                .is_some_and(|b| bits & b != 0)
        })
        .map(|(_, s)| *s)
        .collect()
}

/// User code the engine keeps to run later, `f`, of kind `site` (ADR-0015,
/// a window owns what is made in it): the one constructor of every piece
/// of work scheduled in a window (a timer's work, an effect's run, a
/// cleanup or unmount hook, a hook after layout, a mailbox's handler).
/// Made, it
///
/// - registers its kind ([`stored_kinds`], what the harness finds);
/// - keeps the window current now, weakly (it never keeps a closed window
///   alive), with what it schedules: it runs there, whichever window's
///   turn runs it, in a turn ([`Self::run`]);
/// - belongs to that window's life: once the window closed it never runs,
///   and the queues holding it let it go as it closes ([`Self::is_of`]).
pub(crate) struct Stored<F> {
    site: Site,
    here: Here,
    f: F,
}

crate::engine::parts_whole!(Stored<Box<dyn FnOnce()>>, Stored<()>);

impl<F> Stored<F> {
    /// `f`, of kind `site`, kept in the window current now.
    pub(crate) fn new(site: Site, f: F) -> Self {
        stored_kind(site);
        Self {
            site,
            here: Here::now(),
            f,
        }
    }

    /// Whether it was made in `window` (its close lets it go).
    pub(crate) fn is_of(&self, window: &FocusRoot) -> bool {
        self.here.0.as_ref().is_some_and(|w| std::ptr::eq(w.as_ptr(), window))
    }

    /// Run it (`go` given the closure) in its window, as part of a turn:
    /// the turn under way, its window entered and the one before put back
    /// after, or, with none, a turn of its own of its window ([`turn`]).
    /// Inside a turn its panic goes to the arbiter and it answers `None`,
    /// as through the gate ([`reach`]). `None` too, without running it,
    /// once its window has closed. Whether a fork runs it is the caller's
    /// to say: [`Self::call`] asks.
    pub(crate) fn run<R>(&self, go: impl FnOnce(&F) -> R) -> Option<R> {
        self.here.run(self.site, || go(&self.f))
    }

    /// [`Self::run`], never inside a fork (a caller's closure writes what
    /// it captured, which no rollback puts back: [`call_out`]).
    pub(crate) fn call<R>(&self, go: impl FnOnce(&F) -> R) -> Option<R> {
        if super::forking() {
            return None;
        }
        self.run(go)
    }

    /// [`Self::call`], for work that runs once (it is given the closure).
    pub(crate) fn call_once<R>(self, go: impl FnOnce(F) -> R) -> Option<R> {
        if super::forking() {
            return None;
        }
        let Self { site, here, f } = self;
        here.run(site, || go(f))
    }

    /// [`Self::run`], for work that runs once.
    pub(crate) fn run_once<R>(self, go: impl FnOnce(F) -> R) -> Option<R> {
        let Self { site, here, f } = self;
        here.run(site, || go(f))
    }
}

/// One step of the engine's own state that writes more than one signal
/// (focus and its ring, a press, hover), as one commit: its writes inside
/// one batch, so what reads them (an effect, a render) sees every one or
/// none (Flutter's `applyFocusChangesIfNeeded`: commit, then notify). The
/// engine-owned signals take a write only with the [`Commit`] it hands
/// `f`, so a step that writes them outside one does not compile; in a
/// debug build an effect that runs inside a step panics ([`in_commit`]).
/// A step is part of a turn (`_turn`): the effects its writes wake run in
/// that turn, never bare.
pub(crate) fn commit<R>(_turn: &Turn, f: impl FnOnce(&Commit) -> R) -> R {
    /// Counts the step while it runs, on return or unwind.
    struct Step;
    impl Drop for Step {
        fn drop(&mut self) {
            COMMITTING.try_with(|c| c.set(c.get().saturating_sub(1))).ok();
        }
    }
    crate::reactive::batch(|| {
        COMMITTING.with(|c| c.set(c.get().saturating_add(1)));
        let _step = Step;
        f(&Commit(()))
    })
}

/// [`commit`] as part of the turn under way, or of a turn of its own
/// ([`in_turn`]): an engine step reached from deep inside one (input's
/// routing, layout writing a node's flags), where the turn is the one its
/// pipeline opened. `R::default()` when the step panicked inside another
/// window's turn.
pub(crate) fn commit_here<R: Default>(f: impl FnOnce(&Commit) -> R) -> R {
    in_turn(|t| commit(t, f)).unwrap_or_default()
}

/// Leave to write an engine-owned signal ([`Owned`]): made only by
/// [`commit`], for the step it runs.
pub(crate) struct Commit(());

/// Whether an engine commit step runs now ([`commit`]): an effect run then
/// would read it half done.
pub(crate) fn in_commit() -> bool {
    COMMITTING.try_with(std::cell::Cell::get).is_ok_and(|n| n > 0)
}

/// A signal the engine owns (focus, its ring, a node's flags, the pointer,
/// a window's ambient state, the modality): anyone reads it; only a
/// commit step writes it ([`commit`]).
#[derive(Debug)]
pub(crate) struct Owned<T: 'static>(crate::reactive::Signal<T>);

impl<T: 'static> Clone for Owned<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Owned<T> {}

impl<T: 'static> PartialEq for Owned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Clone + PartialEq + std::fmt::Debug + 'static> Owned<T> {
    /// One labelled `label`, holding `value`, made detached.
    #[track_caller]
    pub(crate) fn labeled(label: &'static str, value: T) -> Self {
        Self(crate::reactive::detached(|| {
            crate::reactive::Signal::labeled(label, value)
        }))
    }

    /// Read it (tracked).
    pub(crate) fn get(self) -> T {
        self.0.get()
    }

    /// Read it without subscribing.
    pub(crate) fn get_untracked(self) -> T {
        self.0.get_untracked()
    }

    /// Write `value` inside the commit `_c`, only when it differs and it
    /// still lives (its node or window gone, nothing); whether it did.
    pub(crate) fn put(self, value: T, _c: &Commit) -> bool {
        let changed = self.0.is_alive() && self.0.get_untracked() != value;
        if changed {
            self.0.set(value);
        }
        changed
    }

    /// Whether it lives (its window or node not yet gone).
    pub(crate) fn is_alive(self) -> bool {
        self.0.is_alive()
    }

    /// Release it with its owner (a window gone).
    pub(crate) fn dispose_quietly(self) {
        self.0.dispose_quietly();
    }
}
