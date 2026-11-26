//! Keyboard focus: which focusable element receives keys, how the user
//! acts ([`Modality`]), and the focus scopes of open layers
//! ([`FocusScope`], kept by the engine's focus manager, ADR-0024).
//!
//! A component creates a [`FocusHandle`] once (it is part of the component's
//! state, like a signal) and attaches it to an element with `track_focus`.
//! Which handle holds focus is a signal, so reading it re-renders on change.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::edit::TextEdit;
use crate::engine::{Commit, Owned};
use crate::geometry::NodePath;
use crate::layer::{Entry, Layer, Stack, layer_start, within};
use crate::layout::Frame;

/// The focus handles made, numbered.
static NEXT: crate::engine::IdKind = crate::engine::IdKind::new("focus handle");
/// The windows made, for each one's own path-derived ids.
static WINDOWS: crate::engine::IdKind = crate::engine::IdKind::new("focus window");

/// The windows whose code runs now, one inside another (ADR-0015, the
/// window stack), written only here: pushed by the guard a turn or work
/// scheduled in a window enters it with ([`enter_window`],
/// [`enter_thread`], which take the engine's leave, `engine::Entering`),
/// popped as that guard drops, and a closing window struck out of every
/// place in it ([`cx::forget`]). Nothing else in this module or outside it
/// reaches it.
mod cx {
    use super::FocusRoot;
    use std::cell::{Cell, RefCell};
    use std::rc::{Rc, Weak};

    /// How deep turns of windows may nest, one inside another's code: a
    /// handler sending input to another window, work scheduled in one run
    /// from another's turn. Deeper is runaway recursion, an engine fault.
    const DEPTH: usize = 64;

    /// One place in the stack.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Entry {
        /// The thread's own window ([`NOWHERE`]): code from outside any
        /// turn that names no window (ADR-0015, the thread-window rule).
        Thread,
        /// A window, by its salt ([`WINDOW_OF`] finds it).
        Window(u64),
        /// A window that closed while its entry was below the top: never
        /// current again; what is above or below it is.
        Gone,
    }

    /// The stack: plain data in a `Copy` cell, so a check's pushes, pops
    /// and strikes are undone by the switch, and a turn allocates nothing.
    #[derive(Clone, Copy)]
    struct Stack {
        len: usize,
        at: [Entry; DEPTH],
    }

    impl Stack {
        /// The window current: the top entry not struck out; `None` for
        /// the thread's own (an empty stack, or `Thread` on top).
        fn top(&self) -> Option<u64> {
            self.at.get(..self.len)?.iter().rev().find_map(|e| match e {
                Entry::Thread => Some(None),
                Entry::Window(salt) => Some(Some(*salt)),
                Entry::Gone => None,
            })?
        }
    }

    crate::engine::engine_local! {
        /// The windows whose code runs now ([`Stack`]): its top (struck
        /// entries skipped) is what [`super::focused`],
        /// [`super::set_focus`], a handle's `focus()` and the window's
        /// ambient signals ([`super::WindowSignals`]) read and write. A
        /// turn pushes its window ([`enter_window`]), and its guard pops it
        /// as it ends, on return or unwind; with nothing on it, no turn
        /// runs, and the window current is the thread's own ([`NOWHERE`]),
        /// never the window of the last turn.
        [Fork] static STACK: Cell<Stack> = const { Cell::new(Stack { len: 0, at: [Entry::Gone; DEPTH] }) };
        /// Each window made on this thread, by its salt, held weakly: how
        /// the stack's entries (plain salts) find their window. Written by
        /// a window as it is made and as its last handle drops, never by
        /// its close.
        [Shared("a live index from a window's salt to its handle, held weakly; a stack entry names the salt, which forks")]
        static WINDOW_OF: RefCell<std::collections::HashMap<u64, Weak<FocusRoot>>> = RefCell::default();
        /// The thread's own window: what code with no window current runs
        /// in (the reactive core used with no window, a test's top level
        /// before any window's turn). Made once, never closed nor replaced,
        /// so what is scheduled in it and what reads it stay its own
        /// (Opus r16 I2: a stand-in replaced and disposed left readers on
        /// dead signals). Shared: a fork's code with no window reads it.
        /// Made the first time its state is read ([`current_window`]),
        /// never just to schedule or run work in it: work made with no
        /// window current runs with none current ([`entered_window`]), so
        /// the reactive core used alone makes no window's signals.
        [Shared("the thread's own window, made once; its signals are the reactive arena's, which forks")]
        static NOWHERE: Rc<FocusRoot> = FocusRoot::new_detached();
    }

    /// The current window's focus (a clone is a count, not an allocation).
    pub(super) fn current() -> Rc<FocusRoot> {
        current_window()
    }

    /// The window current now: always one, the thread's own when no
    /// window's code runs ([`NOWHERE`]).
    pub(crate) fn current_window() -> Rc<FocusRoot> {
        entered_window().unwrap_or_else(thread_window)
    }

    /// The window a turn made current, if any: `None` is the thread's own
    /// ([`NOWHERE`]), which never closes, so work made then needs no
    /// window to be kept in nor entered to run (and makes none). Never a
    /// closed window: a close strikes it out of the stack ([`forget`]).
    pub(crate) fn entered_window() -> Option<Rc<FocusRoot>> {
        let salt = STACK.try_with(|s| s.get().top()).ok().flatten()?;
        WINDOW_OF
            .try_with(|w| w.try_borrow().ok().and_then(|w| w.get(&salt).and_then(Weak::upgrade)))
            .ok()
            .flatten()
    }

    /// The thread's own window ([`NOWHERE`]).
    pub(crate) fn thread_window() -> Rc<FocusRoot> {
        NOWHERE.with(Rc::clone)
    }

    /// The salt of the window current now: 0, the thread's own, when no
    /// window's code runs.
    pub(super) fn current_salt() -> u64 {
        STACK.try_with(|s| s.get().top()).ok().flatten().unwrap_or(0)
    }

    /// Whether `root` is the window current now.
    pub(crate) fn is_current(root: &Rc<FocusRoot>) -> bool {
        STACK.try_with(|s| s.get().top()).ok().flatten() == Some(root.salt)
    }

    /// `root`, made: the stack's entries can name it.
    pub(super) fn made(root: &Rc<FocusRoot>) {
        let made = WINDOW_OF.try_with(|w| {
            w.try_borrow_mut()
                .map(|mut w| {
                    // Those whose last handle went are let go as more are
                    // made: the index holds about the live ones.
                    if w.len() == w.capacity() {
                        w.retain(|_, r| r.strong_count() > 0);
                    }
                    w.insert(root.salt, Rc::downgrade(root))
                })
                .ok()
        });
        drop(made);
    }

    /// The window salted `salt` is gone (its last handle dropped).
    pub(super) fn unmade(salt: u64) {
        let gone = WINDOW_OF.try_with(|w| w.try_borrow_mut().ok().and_then(|mut w| w.remove(&salt)));
        drop(gone);
    }

    /// Push `entry`: the guard that pops it.
    fn push(entry: Entry) -> Entered {
        let index = STACK.with(|s| {
            let mut stack = s.get();
            let index = stack.len;
            #[expect(
                clippy::panic,
                reason = "turns nested past the bound are runaway recursion, an engine fault"
            )]
            let Some(slot) = stack.at.get_mut(index) else {
                panic!("gui: turns of windows nested deeper than {DEPTH}, one inside another");
            };
            *slot = entry;
            stack.len = index.saturating_add(1);
            s.set(stack);
            index
        });
        Entered { index }
    }

    /// Make `root` the current window while its code runs, until the
    /// guard drops ([`Entered`]): pushed on the stack, popped then, on
    /// return or unwind, whatever it entered from. Only the engine's turn
    /// and the work a window scheduled enter one (`_leave`). A closed
    /// window is never entered: what would run in it does not
    /// (`engine::turn`), so this is its stack's guard, never its door.
    pub(crate) fn enter_window(root: &Rc<FocusRoot>, _leave: &crate::engine::Entering) -> Entered {
        debug_assert!(!root.is_closed(), "gui: a closed window entered");
        push(Entry::Window(root.salt))
    }

    /// Make the thread's own window current while code from outside any
    /// turn runs, until the guard drops: a turn that names no window
    /// (ADR-0015, the thread-window rule). Makes no window's signals.
    pub(crate) fn enter_thread(_leave: &crate::engine::Entering) -> Entered {
        push(Entry::Thread)
    }

    /// A window's code running ([`enter_window`], [`enter_thread`]): its
    /// place on the stack. Dropped, on return or unwind, the stack is cut
    /// back to below it: it restores no window it saved, so none it named
    /// can come back once closed.
    #[must_use = "the window is current while this lives"]
    pub struct Entered {
        index: usize,
    }

    impl Drop for Entered {
        fn drop(&mut self) {
            let index = self.index;
            STACK
                .try_with(|s| {
                    let mut stack = s.get();
                    stack.len = stack.len.min(index);
                    s.set(stack);
                })
                .ok();
        }
    }

    /// `root` closes: struck out of every place on the stack, so no guard
    /// popping down to it makes it current again (Opus r17 I1).
    pub(super) fn forget(root: &FocusRoot) {
        let salt = root.salt;
        STACK
            .try_with(|s| {
                let mut stack = s.get();
                let len = stack.len;
                for e in stack.at.iter_mut().take(len) {
                    if *e == Entry::Window(salt) {
                        *e = Entry::Gone;
                    }
                }
                s.set(stack);
            })
            .ok();
    }
}

pub use cx::Entered;
use cx::current;
pub(crate) use cx::{enter_thread, enter_window, entered_window, is_current};

crate::engine::engine_local! {
    /// Each window's focus to hand on and focusable nodes, by its salt.
    /// Logged, not behind the window's `Rc`: the registry puts a pointer
    /// back, not what it points to, so state a fork writes lives where the
    /// rollback undoes it (ADR-0028, rule 4).
    [Fork(log)] static WINDOW_FOCUS: RefCell<crate::engine::ForkMap<u64, WindowFocus>> = RefCell::default();
    /// Windows gone while [`WINDOW_FOCUS`] was borrowed: their entries,
    /// taken out at its next use ([`sweep_gone`]), so none is left behind.
    /// Bounded: one entry per window dropped during a single borrow (a key
    /// per window, so a window counts once), all of them emptied by the
    /// next use, which every focus read or write is.
    [Fork(log)] static GONE: RefCell<crate::engine::ForkMap<u64, ()>> = RefCell::default();
}

/// One window's focus: which of its nodes holds it and whether its ring
/// shows (signals, which the reactive arena forks), and its salt, which
/// names its focus to hand on and its focusable nodes ([`WINDOW_FOCUS`]).
/// Each window has its own, so laying one out or moving focus in it leaves
/// another's as it was.
pub struct FocusRoot {
    /// Mixed into its path-derived ids ([`FocusHandle::for_path`]), so the
    /// same place in two windows is two nodes.
    salt: u64,
    focused: Owned<Option<FocusId>>,
    /// Whether the input modality was [`Modality::Keyboard`] when focus last
    /// moved, which is when `focus-visible` shows: written only by
    /// [`set_focus`], from [`MODALITY`], so the two never disagree.
    visible: Owned<bool>,
    /// What the platform says of this window: each window's own, so a
    /// second window's input or resize leaves this one's as it was.
    window: WindowSignals,
    /// Its end as its last `Rc` goes ([`WindowEnd`]).
    _ending: crate::engine::Ends<WindowEnd>,
}

/// Where a window is in its life (ADR-0015, the window state machine):
///
/// ```text
/// Open ──Pipeline::close──▶ Closing ──its close turn settled──▶ Closed
///  (runs turns)           (one turn: the fixed point)          (no record)
/// ```
///
/// Closed is the record's absence ([`WINDOW_WORK`]): nothing names a
/// closed window's state, and a check's close removes the record in the
/// check's world alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Life {
    /// It runs turns, and owns what is made in it.
    #[default]
    Open,
    /// Its close turn runs: what is made in it still goes to it, and the
    /// close takes it too ([`FocusRoot::settle_close`]).
    Closing,
}

/// How many rounds a close takes before work that keeps making work in a
/// closing window is a fault of the close, not a hang.
pub(crate) const CLOSE_ROUNDS: usize = 64;

/// What a window owns of the work made in it, by its salt ([`WINDOW_WORK`]):
/// present while it is open or closing, gone once it closed (its work was
/// ended by its close, and what names it never runs: `engine::Here`). Not
/// behind the window's `Rc`: logged, so what a fork writes there its
/// rollback undoes (ADR-0028, rule 4).
#[derive(Clone, Default)]
struct WindowWork {
    /// Where it is in its life ([`Life`]).
    life: Life,
    /// The effects made in it that no scope owns (made at a turn's top,
    /// a setup's own): disposed as it closes, in its own turn
    /// ([`FocusRoot::dispose_effects`]). Those disposed before are let go
    /// as more are made.
    effects: Vec<crate::reactive::Effect>,
    /// The engine's live obligations made in it ([`WindowOwned`]): each
    /// ended, once, as it closes, whatever still holds them (ADR-0034,
    /// S4). Held weakly: one already gone is let go as more are made.
    owned: Vec<std::rc::Weak<dyn WindowOwned>>,
}

// A logged value (ADR-0034 S7, audited): `&WindowWork` changes nothing.
crate::engine::fork_value_audited!(WindowWork);
impl crate::engine::parts_seal::Sealed for WindowWork {}
impl crate::engine::Parts for WindowWork {
    /// Each effect handle and each weak obligation on its own box (S9).
    /// Every field named (the drain lint).
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        let Self { life, effects, owned } = self;
        for e in effects {
            crate::engine::whole(e, out);
        }
        for o in owned {
            crate::engine::whole(o, out);
        }
        crate::engine::whole(life, out);
    }
}
impl crate::engine::sealed::Sealed for WindowWork {}
impl crate::engine::ForkClone for WindowWork {
    /// A copy of two short lists, on a fork's first write to the entry
    /// (a fork makes no effect it keeps, and owns no obligation).
    fn fork_clone(&self) -> Self {
        self.clone()
    }
}

crate::engine::engine_local! {
    /// Each open window's work ([`WindowWork`]), by its salt.
    [Fork(log)] static WINDOW_WORK: RefCell<crate::engine::ForkMap<u64, WindowWork>> = RefCell::default();
}

/// The windows' work and focus as the live registry holds them, by
/// window in the order they were made (salts rise), with no id: what V2
/// compares between two engines (review r18, decision 6).
#[cfg(test)]
pub(crate) fn registry_summary() -> String {
    let mut work: Vec<(u64, String)> = WINDOW_WORK.with_borrow(|w| {
        w.iter()
            .map(|(salt, w)| {
                let owned = w.owned.iter().filter(|o| o.strong_count() > 0).count();
                (*salt, format!("{:?} effects {} owned {owned}", w.life, w.effects.len()))
            })
            .collect()
    });
    work.sort_unstable_by_key(|(salt, _)| *salt);
    let mut focus: Vec<(u64, bool)> =
        WINDOW_FOCUS.with_borrow(|f| f.iter().map(|(salt, f)| (*salt, f.hand_on.is_some())).collect());
    focus.sort_unstable_by_key(|(salt, _)| *salt);
    format!(
        "work {:?} hand_on {:?}",
        work.into_iter().map(|(_, w)| w).collect::<Vec<_>>(),
        focus.into_iter().map(|(_, h)| h).collect::<Vec<_>>()
    )
}

/// Write window `salt`'s work, if it is open and the table not borrowed.
fn window_work<R>(salt: u64, f: impl FnOnce(&mut WindowWork) -> R) -> Option<R> {
    WINDOW_WORK
        .try_with(|w| w.try_with_mut(|w| w.contains_key(&salt).then(|| f(w.entry_mut(salt)))))
        .ok()
        .flatten()
        .flatten()
}

/// A live obligation of the engine's that its window ends as it closes
/// (a list's views and their worker): its end is idempotent, as its own
/// owner (a scope, its last live handle) may have ended it first.
pub(crate) trait WindowOwned {
    /// End it (again: nothing).
    fn end_with_window(&self);
}

/// The current window owns `owned` ([`WindowOwned`]): ended as it closes.
/// Whether a window took it: not with no window's code running (the
/// thread's own never closes). In a check, the check's copy of the
/// window's work takes it, gone with the check (review r17, decision 1).
/// Only [`crate::engine::own`] calls it.
pub(crate) fn owned_by_window(owned: std::rc::Weak<dyn WindowOwned>) -> bool {
    let Some(window) = entered_window() else {
        return false;
    };
    window_work(window.salt, |w| {
        if w.owned.len() == w.owned.capacity() {
            w.owned.retain(|o| o.strong_count() > 0);
        }
        w.owned.push(owned);
    })
    .is_some()
}

/// The ambient state of one window that its code reads (tracked) without
/// naming it: where the pointer is over it
/// ([`crate::interact::pointer_position`]), whether it has the keyboard
/// focus ([`crate::interact::window_active`]), its size
/// ([`crate::environment::window_size`]) and whether it can be seen
/// ([`crate::environment::window_visible`]). The current window's
/// ([`window_signals`]). How the user acts ([`Modality`]) and reduced
/// motion are the user's, one for the app.
#[derive(Clone, Copy)]
pub(crate) struct WindowSignals {
    pub(crate) pointer: Owned<Option<(f32, f32)>>,
    pub(crate) active: Owned<bool>,
    pub(crate) size: Owned<Option<(f32, f32)>>,
    pub(crate) visible: Owned<bool>,
}

impl WindowSignals {
    fn new() -> Self {
        Self {
            pointer: Owned::labeled("pointer", None),
            active: Owned::labeled("window active", true),
            size: Owned::labeled("window size", None),
            visible: Owned::labeled("window visible", true),
        }
    }

    fn dispose(self) {
        self.pointer.dispose_quietly();
        self.active.dispose_quietly();
        self.size.dispose_quietly();
        self.visible.dispose_quietly();
    }
}

/// The current window's ambient signals ([`WindowSignals`]).
pub(crate) fn window_signals() -> WindowSignals {
    current().window
}

/// A window's focus to hand on once laid out ([`hand_on`]) and its
/// focusable nodes as its last layout left them ([`publish`]).
#[derive(Clone, Default)]
struct WindowFocus {
    /// Written often (each `hand_on`), and small: a check's copy of it is
    /// a bit copy.
    hand_on: Option<HandOn>,
    /// Published whole by each layout ([`publish`]), and large: shared, so
    /// a check's copy of the entry shares the live set (ADR-0034, S7). A
    /// check's layout publishes a set of its own beside it, never copying
    /// the live one; live, the one owner refills its buffers in place.
    focusables: Rc<Focusables>,
}

// A logged value (ADR-0034 S7, audited): `&WindowFocus` changes nothing.
crate::engine::fork_value_audited!(WindowFocus);
impl crate::engine::parts_seal::Sealed for WindowFocus {}
impl crate::engine::Parts for WindowFocus {
    /// The published set, let go of at once if live shares it (S9).
    /// Every field named (the drain lint).
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        let Self { hand_on, focusables } = self;
        crate::engine::whole(hand_on, out);
        crate::engine::Parts::parts(focusables, out);
    }
}
impl crate::engine::sealed::Sealed for WindowFocus {}
impl crate::engine::ForkClone for WindowFocus {
    /// O(1): the hand-on is copied, the focusable set shared.
    fn fork_clone(&self) -> Self {
        Self {
            hand_on: self.hand_on,
            focusables: Rc::clone(&self.focusables),
        }
    }
}

impl FocusRoot {
    /// A window's. Making one switches no window: the caller's stays
    /// current, and this one is current only in its own turns
    /// (`engine::turn`).
    #[must_use]
    pub fn new() -> Rc<Self> {
        // Hashed into the handles of its paths ([`FocusHandle::for_path`]),
        // which a caller holds: unique as they are.
        let salt = crate::engine::unique_id(&WINDOWS);
        WINDOW_WORK.with_borrow_mut(|w| w.insert(salt, WindowWork::default()));
        let root = Rc::new(Self::with_salt(salt));
        cx::made(&root);
        root
    }
}

impl FocusRoot {
    /// Open → Closing ([`Life`]): whether it was open. Its live owner
    /// begins its close (`Pipeline::close`), then runs one turn of it in
    /// which the close settles ([`Self::settle_close`]) and ends
    /// ([`Self::finish_close`]). Closing again, or from inside its own
    /// close, does nothing.
    pub(crate) fn begin_close(&self) -> bool {
        window_work(self.salt, |w| {
            let open = w.life == Life::Open;
            w.life = Life::Closing;
            open
        })
        .unwrap_or(false)
    }

    /// The close's fixed point, inside its close turn (ADR-0015, the close
    /// order): round after round, every piece of work it owns is taken out
    /// and ended there, in it: its effects disposed (their cleanups run),
    /// its timers, hooks after layout and mailboxes cancelled and dropped,
    /// its obligations ended. What that work makes in it in turn is the
    /// next round's, until a round finds none. `Err` with the rounds run
    /// when work keeps making work past [`CLOSE_ROUNDS`]: a fault of the
    /// close, never a hang.
    pub(crate) fn settle_close(self: &Rc<Self>) -> Result<(), usize> {
        for _ in 0..CLOSE_ROUNDS {
            if !self.close_round() {
                return Ok(());
            }
        }
        Err(CLOSE_ROUNDS)
    }

    /// One round of [`Self::settle_close`]: whether it found any work.
    fn close_round(self: &Rc<Self>) -> bool {
        let (effects, owned) = window_work(self.salt, |w| {
            (std::mem::take(&mut w.effects), std::mem::take(&mut w.owned))
        })
        .unwrap_or_default();
        let mut found = !effects.is_empty() || !owned.is_empty();
        // Each piece ends though one panics: inside the close turn, each
        // panic goes to its arbiter as it is caught.
        let mut all = crate::engine::Gather::new();
        for e in effects {
            all.run(|| e.dispose());
        }
        found |= all.run(|| crate::clock::cancel_window(self)).unwrap_or(0) > 0;
        found |= all.run(|| crate::component::cancel_hooks(self)).unwrap_or(0) > 0;
        found |= all.run(|| crate::mailbox::close_window(self)).unwrap_or(0) > 0;
        for o in owned {
            if let Some(o) = o.upgrade() {
                all.run(|| o.end_with_window());
            }
        }
        all.finish();
        found
    }

    /// Closing → Closed, the last step of its close turn: its record goes
    /// (a debug build panics should a settled close leave work in it), no
    /// place on the window stack names it any more, and its signals end in
    /// the running world. `settled`: whether [`Self::settle_close`] found
    /// none left; one that did not was a fault, already raised, and what it
    /// left belongs to no open window, so never runs.
    pub(crate) fn finish_close(&self, settled: bool) {
        let left = WINDOW_WORK
            .try_with(|w| w.try_with_mut(|w| w.remove(&self.salt)).flatten())
            .ok()
            .flatten();
        if let Some(left) = &left {
            debug_assert!(
                !settled
                    || (left.effects.iter().all(|e| !e.is_alive()) && left.owned.iter().all(|o| o.strong_count() == 0)),
                "gui: a window's record removed while it still holds work"
            );
        }
        drop(left);
        cx::forget(self);
        self.end();
    }

    /// Whether it closed ([`Self::finish_close`]): its record is gone.
    pub(crate) fn is_closed(&self) -> bool {
        // The thread's own (salt 0) never closes, and keeps no work table:
        // what it owns lives as long as the thread.
        self.salt != 0
            && WINDOW_WORK
                .try_with(|w| w.try_with(|w| !w.contains_key(&self.salt)).unwrap_or(false))
                .unwrap_or(true)
    }

    /// It owns `effect`, made in it with no scope to own it: disposed as
    /// it closes ([`Self::settle_close`]; made while it closes, by the
    /// close's next round). A fork's effect goes with it. A closed window
    /// owns nothing: an effect made for one is disposed at once. No code
    /// runs in a closed window (it is struck off the stack), so this is a
    /// guard, not a path: such an effect's cleanups would run where the
    /// caller runs.
    pub(crate) fn own_effect(&self, effect: crate::reactive::Effect) {
        if crate::engine::forking() {
            return;
        }
        let owned = window_work(self.salt, |w| {
            // Those disposed since are let go before the list grows: it
            // holds at most twice the live ones.
            if w.effects.len() == w.effects.capacity() {
                w.effects.retain(crate::reactive::Effect::is_alive);
            }
            w.effects.push(effect);
        });
        if owned.is_none() && self.salt != 0 {
            effect.dispose();
        }
    }

    /// Its signals and entries ended in the running world: an explicit
    /// close a check makes ends the check's copies (review r17, decision 3).
    fn end(&self) {
        let (focused, visible, window, salt) = (self.focused, self.visible, self.window, self.salt);
        end_window(focused, visible, window, salt);
    }
}

/// A window's signals and entries ended, in the running world.
fn end_window(focused: Owned<Option<FocusId>>, visible: Owned<bool>, window: WindowSignals, salt: u64) {
    focused.dispose_quietly();
    visible.dispose_quietly();
    window.dispose();
    // Its entry goes now, or, the map borrowed (a window dropped while its
    // focus is read), at the map's next use: never left behind. Its work
    // table goes too (a window dropped unclosed): closed, its work never
    // runs.
    let work = WINDOW_WORK
        .try_with(|w| w.try_with_mut(|w| w.remove(&salt)).flatten())
        .ok()
        .flatten();
    drop(work);
    let removed = WINDOW_FOCUS
        .try_with(|w| w.try_with_mut(|w| drop(w.remove(&salt))).is_some())
        .unwrap_or(true);
    if !removed {
        GONE.try_with(|g| g.with_mut(|g| g.insert(salt, ()))).ok();
    }
}

/// A window's end, as its [`FocusRoot`]'s last `Rc` goes: its index entry
/// ([`cx::unmade`]), its signals and its entries, in the world that made it
/// ([`crate::engine::Ends`], ADR-0035). A live window a check drops goes
/// once the check has switched back: released in the check, the rollback
/// would put its entry and its signals back with no window left to release
/// them (ADR-0028; review r18, decision 4).
struct WindowEnd {
    focused: Owned<Option<FocusId>>,
    visible: Owned<bool>,
    window: WindowSignals,
    salt: u64,
}

impl crate::engine::Ending for WindowEnd {
    fn end(self) {
        cx::unmade(self.salt);
        end_window(self.focused, self.visible, self.window, self.salt);
    }
}

/// Take out the entries of windows gone while the map was borrowed.
fn sweep_gone() {
    let gone: Vec<u64> = GONE.with_borrow_mut(|g| {
        if g.is_empty() {
            return Vec::new();
        }
        let keys: Vec<u64> = g.iter().map(|(k, ())| *k).collect();
        for k in &keys {
            g.remove(k);
        }
        keys
    });
    if gone.is_empty() {
        return;
    }
    let removed: Vec<WindowFocus> =
        WINDOW_FOCUS.with_borrow_mut(|w| gone.iter().filter_map(|salt| w.remove(salt)).collect());
    drop(removed);
}

/// What the current window's focus to hand on and focusable nodes are
/// ([`WindowFocus`]), read.
fn window_focus<R>(f: impl FnOnce(Option<&WindowFocus>) -> R) -> R {
    sweep_gone();
    let salt = current().salt;
    WINDOW_FOCUS.with_borrow(|w| f(w.get(&salt)))
}

/// The current window's [`WindowFocus`], written (logged in a fork).
fn window_focus_mut<R>(f: impl FnOnce(&mut WindowFocus) -> R) -> R {
    sweep_gone();
    let salt = current().salt;
    WINDOW_FOCUS.with_borrow_mut(|w| f(w.entry_mut(salt)))
}

impl fmt::Debug for FocusRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FocusRoot")
            .field("salt", &self.salt)
            .finish_non_exhaustive()
    }
}

impl FocusRoot {
    /// The thread's own, for code that runs outside any window.
    fn new_detached() -> Rc<Self> {
        Rc::new(Self::with_salt(0))
    }

    /// Its state, `salt` its id, ending with it ([`WindowEnd`]).
    fn with_salt(salt: u64) -> Self {
        let (focused, visible, window) = (
            Owned::labeled("focus", None),
            Owned::labeled("focus-visible", false),
            WindowSignals::new(),
        );
        Self {
            salt,
            focused,
            visible,
            window,
            _ending: crate::engine::Ends::new(WindowEnd {
                focused,
                visible,
                window,
                salt,
            }),
        }
    }
}

#[derive(Clone, Copy)]
struct HandOn {
    from: FocusId,
    into: crate::access::NodeRef,
    or: Option<FocusId>,
}

/// Focus held by `from`, which the next frame takes out of reach (its node
/// goes inert or away: a carousel's slide leaving the view), goes on to the
/// first Tab stop inside the node `into` names, else to `or`, so it is not
/// lost (reka's carousel `rescueFocus`). Applied as that frame settles,
/// only if `from` still holds focus and no node holds it any more.
pub fn hand_on(from: FocusId, into: crate::access::NodeRef, or: Option<FocusHandle>) {
    let h = HandOn {
        from,
        into,
        or: or.map(FocusHandle::id),
    };
    window_focus_mut(|w| w.hand_on = Some(h));
}

/// Hand focus on as [`hand_on`] asked; whether it moved.
fn take_hand_on(frame: &Frame, stack: &Stack) -> bool {
    // Written only when one is asked for: a frame with none logs nothing.
    if !window_focus(|w| w.is_some_and(|w| w.hand_on.is_some())) {
        return false;
    }
    let Some(h) = window_focus_mut(|w| w.hand_on.take()) else {
        return false;
    };
    if focused_now() != Some(h.from) || node_of(frame, h.from).is_some() {
        return false;
    }
    let inside = (0..frame.nodes.len())
        .find(|&i| frame.behaviour(i).is_some_and(|b| b.semantics.node == Some(h.into)))
        .and_then(|o| (o..=subtree_end(frame, o)).find(|&i| tabbable(frame, stack, i)))
        .and_then(|i| focus_at(frame, i));
    let Some(to) = inside.or(h.or.filter(|&id| node_of(frame, id).is_some())) else {
        return false;
    };
    engine_focus(frame, Some(to));
    true
}

/// The identity of one focusable element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusId(u64);

impl FocusId {
    /// It as an element key: no two ids issued share one, so what it
    /// focuses can be keyed by it among its siblings (a virtual list's
    /// rows).
    #[must_use]
    pub const fn key(self) -> crate::ElementKey {
        self.0
    }
}

/// A focus target that stays the same across frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusHandle {
    id: FocusId,
}

impl Default for FocusHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            id: FocusId(crate::engine::unique_id(&NEXT)),
        }
    }

    /// The handle of a node focusable by its place in the tree of the
    /// current window: the same place in another window is another node.
    #[must_use]
    pub fn for_path(path: &crate::geometry::NodePath) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        current().salt.hash(&mut h);
        path.hash(&mut h);
        // The top bit keeps these apart from counted handles.
        Self {
            id: FocusId(h.finish() | (1 << 63)),
        }
    }

    #[must_use]
    pub fn id(self) -> FocusId {
        self.id
    }

    /// Whether this handle holds focus (reactive).
    #[must_use]
    pub fn is_focused(self) -> bool {
        focused() == Some(self.id)
    }

    /// Give this handle focus; its ring shows when the user is at the
    /// keyboard ([`modality`]), as a script's `focus()` matches
    /// `:focus-visible` after a key.
    pub fn focus(self) {
        set_focus(Some(self.id), false);
    }

    /// Drop focus if this handle holds it.
    pub fn blur(self) {
        if self.is_focused() {
            set_focus(None, false);
        }
    }
}

/// A window, by identity: the key of state an app keeps per window (one
/// tooltip open in each), never a handle to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(u64);

impl WindowId {
    /// As a number, to key a map (unique on this thread; the thread's own
    /// window's is 0).
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// The window whose code runs now (ADR-0015, the window stack): the
/// thread's own when none's does. Not reactive: which window runs code is
/// not state its code reads to change.
#[must_use]
pub fn window_id() -> WindowId {
    WindowId(cx::current_salt())
}

/// The focused element, if any (reactive).
#[must_use]
pub fn focused() -> Option<FocusId> {
    current().focused.get()
}

/// Whether the current focus shows its ring: the user was at the keyboard
/// when it moved there (reactive).
#[must_use]
pub fn focus_visible() -> bool {
    current().visible.get()
}

/// Move focus to `id`. `by_keyboard`: the keyboard moved it, so the input
/// modality is [`Modality::Keyboard`] from now on; otherwise the modality
/// is left as it is. The ring shows when the modality is `Keyboard`: one
/// fact decides it, whoever moves focus.
pub fn set_focus(id: Option<FocusId>, by_keyboard: bool) {
    // Focus asked for a node laid out but not shown does nothing, as
    // `focus()` on an element that is not rendered does nothing in a
    // browser: what holds focus keeps it.
    if let Some(id) = id
        && window_focus(|w| w.is_some_and(|w| w.focusables.hidden.contains(&id)))
    {
        return;
    }
    // One commit: what reads focus, its ring and the modality sees them
    // change together, never the pair half written (ADR-0015). Part of the
    // turn under way (a handler's), or one of its own.
    crate::engine::in_turn(|t| {
        crate::engine::commit(t, |c| {
            if by_keyboard {
                set_modality(Modality::Keyboard, c);
            }
            let visible = id.is_some() && MODALITY.with(|m| m.get_untracked()) == Modality::Keyboard;
            let root = current();
            root.focused.put(id, c);
            root.visible.put(visible, c);
        });
    });
}

/// How the user acts now: one fact of the engine (reka's
/// `useIsUsingKeyboard`). A key press makes it `Keyboard`; a pointer
/// pressing or moving makes it `Pointer`, before any handler of that
/// event runs. Focus the engine moves itself (a layer opening or closing,
/// a trap pulling focus back) shows its ring when it is `Keyboard`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Modality {
    Keyboard,
    #[default]
    Pointer,
}

/// A window's focusable nodes as its last layout left them: those it laid
/// out but does not show ([`Frame::shown`]), whose focus asked for is
/// refused, and where each shown one lies (its path), for what asks where
/// focus is before the next layout.
#[derive(Clone, Default)]
struct Focusables {
    hidden: Vec<FocusId>,
    paths: Vec<(FocusId, NodePath)>,
    /// The frame they were taken from: its epoch and change count.
    of: Option<(u64, usize)>,
}

/// Take the current window's focusable nodes from `frame`, into the
/// buffers kept from the last. Layout calls it as it ends, so what runs
/// after it (a list's watcher, a hook after layout, the focus rules) asks
/// against the frame as it is now. A frame already taken (focus alone
/// moved since) is not read again.
pub(crate) fn publish(frame: &Frame) {
    let key = (frame.epoch(), frame.changes.end());
    if window_focus(|w| w.is_some_and(|w| w.focusables.of == Some(key))) {
        return;
    }
    window_focus_mut(|w| {
        // The set's one owner (live, always) refills its buffers; a set
        // shared with live (a check's copy of the entry) is replaced by
        // one of the check's own, never copied (S7).
        if Rc::get_mut(&mut w.focusables).is_none() {
            w.focusables = Rc::new(Focusables::default());
        }
        let Some(f) = Rc::get_mut(&mut w.focusables) else {
            return;
        };
        f.of = Some(key);
        f.hidden.clear();
        f.paths.clear();
        for i in 0..frame.nodes.len() {
            let (Some(id), Some(n)) = (focus_at(frame, i), frame.nodes.get(i)) else {
                continue;
            };
            if frame.shown(i) {
                // Paths are shared: a clone is a count, not an allocation.
                f.paths.push((id, n.path.clone()));
            } else {
                f.hidden.push(id);
            }
        }
    });
}

/// Where the node focused by `id` lies in the current window's last frame,
/// shown: for what keeps it built (a virtual list's row) before the next
/// layout, as focus moves.
pub(crate) fn path_of(id: FocusId) -> Option<NodePath> {
    window_focus(|w| {
        w?.focusables
            .paths
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, p)| p.clone())
    })
}

crate::engine::engine_local! {
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static MODALITY: Owned<Modality> = Owned::labeled("modality", Modality::Pointer);
}

/// How the user acts now (reactive).
#[must_use]
pub fn modality() -> Modality {
    MODALITY.with(|m| m.get())
}

/// Record how the user acts, inside the step `c` of the input that shows
/// it; written only when it changes.
pub(crate) fn set_modality(m: Modality, c: &Commit) {
    MODALITY.with(|s| s.put(m, c));
}

/// Where a focus scope puts focus as its layer opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoFocus {
    /// Its first node in Tab order; with none, the region itself.
    First,
    /// The region itself (a menu's frame, before an item is highlighted).
    Region,
    /// This node; when it is not there, as [`Self::First`].
    Node(FocusHandle),
    /// Nowhere: focus stays where it is (reka's prevented
    /// `open-auto-focus`), and the owner moves it if it wants.
    Keep,
}

/// What a layer does with focus while it is open (reka's `FocusScope`).
/// Scopes stack in opening order (the layers' order); only the top one acts, so a scope opened
/// above another, trapped or not, pauses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusScope {
    /// Focus cannot leave: a press, Tab or code moving it out is undone
    /// (to the node inside that had it last).
    pub trapped: bool,
    /// Tab at its last node goes to its first, Shift+Tab the other way;
    /// otherwise a trapped scope stops there and one that is not lets
    /// focus go on past its owner.
    pub cycle: bool,
    pub auto_focus: AutoFocus,
    /// As it closes, focus goes back to what held it as it opened, unless
    /// something else has taken focus since.
    pub restore: bool,
    /// Where restoring focus goes first, when it is there: a popover's or a
    /// drawer's trigger (reka's `PopoverContent` and `DrawerContent` focus
    /// their trigger, not what held focus as they opened).
    pub return_to: Option<FocusHandle>,
}

impl FocusScope {
    /// Not trapped, cycling, focusing its first node, restoring focus.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            trapped: false,
            cycle: true,
            auto_focus: AutoFocus::First,
            restore: true,
            return_to: None,
        }
    }

    /// Trapped and cycling, as a modal dialog keeps focus.
    #[must_use]
    pub const fn trapped() -> Self {
        Self {
            trapped: true,
            ..Self::new()
        }
    }

    #[must_use]
    pub const fn trap(self, trapped: bool) -> Self {
        Self { trapped, ..self }
    }

    #[must_use]
    pub const fn cycle(self, cycle: bool) -> Self {
        Self { cycle, ..self }
    }

    #[must_use]
    pub const fn auto_focus(self, auto_focus: AutoFocus) -> Self {
        Self { auto_focus, ..self }
    }

    #[must_use]
    pub const fn restore(self, restore: bool) -> Self {
        Self { restore, ..self }
    }

    /// Give focus back to `to` as it closes, when `to` is there; else as
    /// [`Self::restore`] says.
    #[must_use]
    pub const fn return_to(self, to: FocusHandle) -> Self {
        Self {
            return_to: Some(to),
            ..self
        }
    }
}

impl Default for FocusScope {
    fn default() -> Self {
        Self::new()
    }
}

/// The frame node holding focus `id`: one not shown ([`Frame::shown`])
/// holds none, as one not laid out holds none.
pub(crate) fn node_of(frame: &Frame, id: FocusId) -> Option<usize> {
    (0..frame.nodes.len()).find(|&i| frame.shown(i) && focus_at(frame, i) == Some(id))
}

/// The focus id of node `i`, if it is focusable.
fn focus_at(frame: &Frame, i: usize) -> Option<FocusId> {
    frame.behaviour(i).and_then(|b| b.focus).map(FocusHandle::id)
}

/// Whether node `i` is a Tab stop: focusable, shown, not below a modal
/// layer, and not a focus scope's region (which takes focus as a last
/// resort, as reka's `tabindex="-1"` content does, but is no stop). A node
/// out of Tab order ([`crate::Div::tab_stop`]) is one while it holds focus,
/// so Tab goes on from it.
fn tabbable(frame: &Frame, stack: &Stack, i: usize) -> bool {
    let Some(b) = frame.behaviour(i) else {
        return false;
    };
    b.focus
        .is_some_and(|f| Some(f.id()) == focused_now() || b.tab_stop.get())
        && frame.shown(i)
        && !stack.blocked(i)
        && !b.handlers.layer.as_ref().is_some_and(|l| l.scope.is_some())
        && (b.focus.is_some_and(|f| Some(f.id()) == focused_now()) || !stack.untabbable(frame, i))
}

/// Every Tab stop of `frame` in Tab order: the tree's order, each deferred
/// subtree right after its owner's subtree (where it is declared, not at
/// the end of the page as a portal puts it).
pub(crate) fn tab_order(frame: &Frame, stack: &Stack) -> Vec<usize> {
    let layers = frame.layers.len().saturating_add(1);
    // Where each subtree goes: after which node (the end of its owner's subtree).
    let mut after: Vec<(usize, usize)> = Vec::new();
    let mut orphans: Vec<usize> = Vec::new();
    for l in 1..layers {
        match stack.tab_owners.get(l).copied().flatten() {
            Some(o) => after.push((subtree_end(frame, o), l)),
            None => orphans.push(l),
        }
    }
    let mut out = Vec::new();
    // (layer, next node of it) frames, innermost last.
    let mut walk: Vec<(usize, usize)> = vec![(0, 0)];
    let mut orphans = orphans.into_iter();
    while walk.len() <= layers.saturating_add(1) {
        let Some(top) = walk.last_mut() else {
            match orphans.next() {
                Some(l) => {
                    walk.push((l, layer_start(frame, l)));
                    continue;
                }
                None => break,
            }
        };
        let (l, i) = *top;
        if i >= layer_start(frame, l.saturating_add(1)) {
            walk.pop();
            continue;
        }
        top.1 = i.saturating_add(1);
        if tabbable(frame, stack, i) {
            out.push(i);
        }
        // The first declared after `i` is walked first: pushed last.
        for &(at, k) in after.iter().rev() {
            if at == i {
                walk.push((k, layer_start(frame, k)));
            }
        }
    }
    for r in 0..frame.nodes.len() {
        if frame.behaviour(r).is_some_and(|b| b.tab_reversed) {
            reverse_children(frame, stack, r, &mut out);
        }
    }
    out
}

/// The child of node `r` whose subtree holds stop `i`, a layer declared in
/// that subtree counting as in it.
fn child_holding(frame: &Frame, stack: &Stack, r: usize, mut i: usize) -> Option<usize> {
    let layer = frame.nodes.get(r)?.layer;
    while frame.nodes.get(i)?.layer != layer {
        let l = frame.nodes.get(i)?.layer;
        i = stack.owners.get(l).copied().flatten()?;
    }
    loop {
        let parent = frame.nodes.get(i)?.parent?;
        if parent == r {
            return Some(i);
        }
        i = parent;
    }
}

/// Reorder the stops in `order` that lie under node `r`'s children so the
/// children come last first, each child's own stops kept in their order
/// ([`crate::Div::tab_children_reversed`]).
fn reverse_children(frame: &Frame, stack: &Stack, r: usize, order: &mut [usize]) {
    let held: Vec<Option<usize>> = order.iter().map(|&i| child_holding(frame, stack, r, i)).collect();
    let mut start = 0;
    while start < order.len() {
        if held.get(start).copied().flatten().is_none() {
            start = start.saturating_add(1);
            continue;
        }
        // A run of stops under `r`'s children: its groups, one per child.
        let mut end = start;
        let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
        while let Some(Some(c)) = held.get(end).copied() {
            match groups.last_mut() {
                Some(g) if held.get(g.start).copied().flatten() == Some(c) => g.end = end.saturating_add(1),
                _ => groups.push(end..end.saturating_add(1)),
            }
            end = end.saturating_add(1);
        }
        let run: Vec<usize> = groups
            .iter()
            .rev()
            .flat_map(|g| order.get(g.clone()).unwrap_or_default().iter().copied())
            .collect();
        if let Some(slot) = order.get_mut(start..end) {
            slot.copy_from_slice(&run);
        }
        start = end;
    }
}

/// The last node of node `o`'s subtree (its layer's nodes are in depth-first order).
fn subtree_end(frame: &Frame, o: usize) -> usize {
    let layer = frame.nodes.get(o).map_or(0, |n| n.layer);
    let mut end = o;
    while frame
        .nodes
        .get(end.saturating_add(1))
        .is_some_and(|n| n.layer == layer && within(frame, end.saturating_add(1), o))
    {
        end = end.saturating_add(1);
    }
    end
}

/// The focus without subscribing to it.
fn focused_now() -> Option<FocusId> {
    current().focused.get_untracked()
}

/// Move focus to `to` as the engine does: with its ring when the user is
/// at the keyboard, and all of a text field's text selected (reka's
/// `focus(…, { select: true })`).
pub(crate) fn engine_focus(frame: &Frame, to: Option<FocusId>) {
    // One commit: focus and the selection of the field it lands in.
    crate::engine::in_turn(|t| {
        crate::engine::commit(t, |_| {
            set_focus(to, false);
            if let Some(field) = to
                .and_then(|id| node_of(frame, id))
                .and_then(|i| frame.behaviour(i))
                .and_then(|b| b.field)
            {
                field.edit.update(TextEdit::select_all);
            }
        });
    });
}

/// Focus left on a node `frame` (a frame's last pass) does not hold or
/// does not show, which no focus rule moved: nothing holds it, as Chrome's
/// focus fixup gives focus to the body when the focused element goes or
/// stops being rendered. Its id is dropped, so the same handle laid out
/// again later does not take focus back unasked. Whether it was.
pub(crate) fn drop_lost(frame: &Frame) -> bool {
    let lost = focused_now().is_some_and(|id| node_of(frame, id).is_none());
    if lost {
        set_focus(None, false);
    }
    lost
}

/// A scope open in the last settled frame.
struct Open {
    /// Its region's path.
    path: NodePath,
    /// Its owner's path.
    owner: Option<NodePath>,
    /// What held focus as it opened.
    to: Option<FocusId>,
    /// The node inside it that last held focus.
    last_inside: Option<FocusId>,
    scope: FocusScope,
    /// A press outside it, other than a primary press outside a modal
    /// layer, asked it to close in this frame and was answered `Close`:
    /// focus is not given back (reka's `hasInteractedOutside`).
    pressed_outside: bool,
}

/// What settling focus did.
#[derive(Default)]
pub(crate) struct Settled {
    /// Focus moved.
    pub(crate) moved: bool,
    /// The layers focus came outside of, to ask to close.
    pub(crate) outside: Vec<Rc<Layer>>,
}

/// The focus manager (ADR-0024): the stack of open focus scopes, kept
/// across frames by their regions' paths, and the rules run on each laid
/// out frame: a scope that closed gives focus back, one that opened takes
/// it, the top trapped one keeps it, and focus that came outside a layer
/// asks it to close.
#[derive(Default)]
pub(crate) struct FocusManager {
    open: Vec<Open>,
    /// Focus as the last settle left it.
    last: Option<FocusId>,
    /// Every layer's region path as the last settle saw them: a layer not
    /// among them opened in this frame, and focus its opening moved is no
    /// focus outside it.
    seen: Vec<NodePath>,
    /// The frame (its epoch and change count, which a trim never repeats)
    /// and the focus the last settle left: the rules read nothing else, so
    /// the same again is settled already.
    settled: Option<((u64, usize), Option<FocusId>)>,
}

impl FocusManager {
    /// Apply the rules to `frame`, whose layers are `stack`. A frame and a
    /// focus already settled cost nothing; otherwise, while no scope opens
    /// or closes, nothing is allocated.
    pub(crate) fn settle(&mut self, frame: &Frame, stack: &Stack) -> Settled {
        let before = focused_now();
        let handed = take_hand_on(frame, stack);
        let key = (frame.epoch(), frame.changes.end());
        if !handed && self.settled == Some((key, before)) {
            return Settled::default();
        }
        // Paths are shared: a clone is a count, not an allocation.
        let path = |i: usize| frame.nodes.get(i).map(|n| n.path.clone());
        let now = || {
            stack
                .entries
                .iter()
                .filter(|e| e.spec.scope.is_some())
                .filter_map(move |e| Some((e, path(e.node)?)))
        };
        // Closed: bottom first, so a chain closing together gives focus
        // back to the outermost's (the others then find it taken).
        let present = |o: &Open| now().any(|(_, p)| p == o.path);
        let closed: Vec<Open> = if self.open.iter().all(present) {
            Vec::new()
        } else {
            let (closed, kept) = std::mem::take(&mut self.open).into_iter().partition(|o| !present(o));
            self.open = kept;
            closed
        };
        for c in closed {
            if !c.scope.restore {
                continue;
            }
            // Focus left on a node that went or is below a modal: it was in
            // the scope, or nowhere a reader can be. Focus moved elsewhere
            // or dropped (a press on nothing focusable) since: kept.
            let f = focused_now();
            let gone = f.is_some_and(|id| node_of(frame, id).is_none_or(|i| stack.blocked(i)));
            if !gone {
                continue;
            }
            if c.pressed_outside {
                // Focus left with it: nothing holds it, as a browser's
                // `<body>` does when focused content is removed.
                set_focus(None, false);
                continue;
            }
            let live = |id: &FocusId| node_of(frame, *id).is_some_and(|i| !stack.blocked(i));
            let to = c
                .scope
                .return_to
                .map(FocusHandle::id)
                .filter(live)
                .or_else(|| c.to.filter(live))
                .or_else(|| c.owner.as_ref().and_then(|o| near_owner(frame, stack, o)));
            // Nowhere to give it back to (its opener went with it): left
            // where it was, gone, for an enclosing trap's rule below to take
            // back inside it, else for the fixup at the end to drop.
            if to.is_some() {
                engine_focus(frame, to);
            }
        }
        // Opened: bottom first, so the top one's focus wins.
        let mut opened = false;
        for (e, p) in now() {
            if let Some(o) = self.open.iter_mut().find(|o| o.path == p) {
                o.scope = e.spec.scope.unwrap_or_default();
                continue;
            }
            opened = true;
            let scope = e.spec.scope.unwrap_or_default();
            let held = focused_now();
            let inside = held
                .and_then(|id| node_of(frame, id))
                .is_some_and(|i| stack.in_scope(frame, e, i));
            if !inside {
                let to = match scope.auto_focus {
                    AutoFocus::First => first_in(frame, stack, e),
                    AutoFocus::Region => focus_at(frame, e.node),
                    AutoFocus::Node(h) if node_of(frame, h.id()).is_some() => Some(h.id()),
                    AutoFocus::Node(_) => first_in(frame, stack, e),
                    AutoFocus::Keep => None,
                };
                if to.is_some() {
                    engine_focus(frame, to);
                }
            }
            // What held focus as it opened: focus already moved inside it
            // in the frame it opened (its own code focused it), what held
            // it before that frame.
            let to = if inside { self.last } else { held };
            self.open.push(Open {
                path: p,
                owner: e.owner.and_then(path),
                to,
                last_inside: None,
                scope,
                pressed_outside: false,
            });
        }
        if opened {
            // Kept in the stack's order, bottom first, for the next close.
            self.open
                .sort_by_key(|o| now().position(|(_, p)| p == o.path).unwrap_or(usize::MAX));
        }
        // The top scope, trapped, keeps focus.
        if let Some(top) = stack.top_scope()
            && top.spec.scope.is_some_and(|s| s.trapped)
            && let Some(f) = focused_now()
        {
            let pull = match node_of(frame, f) {
                // The node holding it went away: the region takes it.
                None => Some(focus_at(frame, top.node).or_else(|| first_in(frame, stack, top))),
                Some(i) if !stack.in_scope(frame, top, i) => {
                    let last = path(top.node)
                        .and_then(|p| self.open.iter().find(|o| o.path == p))
                        .and_then(|o| o.last_inside)
                        .filter(|id| node_of(frame, *id).is_some_and(|i| stack.in_scope(frame, top, i)));
                    Some(last.or_else(|| first_in(frame, stack, top)))
                }
                Some(_) => None,
            };
            if let Some(to) = pull {
                engine_focus(frame, to);
            }
        }
        // Focus that came to a node outside a layer asks it to close.
        let f = focused_now();
        let mut outside = Vec::new();
        if f != self.last
            && let Some(i) = f.and_then(|id| node_of(frame, id))
        {
            for e in &stack.entries {
                let opened_now = path(e.node).is_none_or(|p| !self.seen.contains(&p));
                if e.spec.policy.focus_outside && !opened_now && !stack.inside(frame, e, i) {
                    outside.push(Rc::clone(&e.spec));
                }
            }
        }
        if let Some(i) = f.and_then(|id| node_of(frame, id)) {
            for (e, p) in now() {
                if stack.in_scope(frame, e, i)
                    && let Some(o) = self.open.iter_mut().find(|o| o.path == p)
                {
                    o.last_inside = f;
                }
            }
        }
        // A press outside that did not close its layer in this frame says
        // nothing about a later close.
        for o in &mut self.open {
            o.pressed_outside = false;
        }
        // Focus left on a node removed or not shown is dropped once the
        // frame's last pass is laid out ([`drop_lost`]), not here: a later
        // pass of this frame may show it again.
        self.last = f;
        self.seen.clear();
        self.seen.extend(stack.entries.iter().filter_map(|e| path(e.node)));
        self.settled = Some((key, f));
        Settled {
            moved: f != before,
            outside,
        }
    }

    /// A press outside the layer whose region is node `i` closed it, and
    /// it is not a primary press outside a modal layer: its scope gives no
    /// focus back as it closes (ADR-0024).
    pub(crate) fn pressed_outside(&mut self, frame: &Frame, i: usize) {
        let Some(p) = frame.nodes.get(i).map(|n| &n.path) else {
            return;
        };
        if let Some(o) = self.open.iter_mut().find(|o| o.path == *p) {
            o.pressed_outside = true;
        }
    }

    /// Where Tab (Shift+Tab when `back`) moves focus in `frame`: through the
    /// top scope's stops, at its ends cycling, stopping (trapped) or going
    /// on past its owner; with no scope, through the whole order, cycling.
    /// `None` leaves focus where it is.
    pub(crate) fn tab(frame: &Frame, stack: &Stack, back: bool) -> Option<FocusId> {
        let order = tab_order(frame, stack);
        let current = focused_now().and_then(|id| node_of(frame, id));
        let top = stack.top_scope();
        let ring: Vec<usize> = match top {
            Some(e) => order.iter().copied().filter(|&i| stack.in_scope(frame, e, i)).collect(),
            None => order.clone(),
        };
        let step = |ring: &[usize], at: Option<usize>, wrap: bool| -> Option<usize> {
            let len = ring.len();
            let next = match (at, back) {
                (None, false) => Some(0),
                (None, true) => len.checked_sub(1),
                (Some(p), false) if p.saturating_add(1) < len => Some(p.saturating_add(1)),
                (Some(p), true) if p > 0 => Some(p.saturating_sub(1)),
                (Some(_), false) => wrap.then_some(0),
                (Some(_), true) => wrap.then(|| len.saturating_sub(1)),
            };
            next.and_then(|n| ring.get(n).copied())
        };
        let at = current.and_then(|c| ring.iter().position(|&i| i == c));
        let scope = top.and_then(|e| e.spec.scope).unwrap_or_default();
        let to = match step(&ring, at, scope.cycle || top.is_none()) {
            Some(i) => Some(i),
            None if top.is_some() && !scope.trapped && !scope.cycle => {
                // Past its end, focus goes on in the page's order.
                let at = current.and_then(|c| order.iter().position(|&i| i == c));
                step(&order, at, true)
            }
            None => None,
        };
        to.and_then(|i| focus_at(frame, i))
    }
}

/// The first Tab stop inside `e`; with none, its region.
fn first_in(frame: &Frame, stack: &Stack, e: &Entry) -> Option<FocusId> {
    tab_order(frame, stack)
        .into_iter()
        .find(|&i| stack.in_scope(frame, e, i))
        .and_then(|i| focus_at(frame, i))
        .or_else(|| focus_at(frame, e.node))
}

/// Where focus goes back to when what held it is gone: the first focusable
/// node of the owner at `owner` (its trigger), else its nearest focusable
/// ancestor.
fn near_owner(frame: &Frame, stack: &Stack, owner: &NodePath) -> Option<FocusId> {
    let o = frame.nodes.iter().position(|n| n.path == *owner)?;
    if stack.blocked(o) {
        return None;
    }
    (o..=subtree_end(frame, o))
        .find_map(|i| focus_at(frame, i))
        .or_else(|| frame.ancestors(o).find_map(|i| focus_at(frame, i)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opus (F1 review 4): a window dropped while its focus map is borrowed
    /// leaves no entry behind: its removal waits for the map's next use.
    #[test]
    fn a_window_dropped_while_its_focus_is_read_leaves_no_entry() {
        let root = FocusRoot::new();
        crate::engine::turn(&root, |_| window_focus_mut(|_| {}));
        let salt = root.salt;
        let other = FocusRoot::new();
        WINDOW_FOCUS.with_borrow(|_read| drop(root));
        assert!(WINDOW_FOCUS.with_borrow(|w| w.contains_key(&salt)), "busy: deferred");
        crate::engine::turn(&other, |_| window_focus(|_| ()));
        assert!(
            !WINDOW_FOCUS.with_borrow(|w| w.contains_key(&salt)),
            "taken out at the next use"
        );
    }

    /// Codex 6 (F1 review 6): the last owner of a live window dropped in a
    /// fork. Released in the fork, the rollback would put its entry and its
    /// signals back with no window left; it is released once the fork has
    /// rolled back, and leaves nothing.
    #[cfg(debug_assertions)]
    #[test]
    fn a_live_window_a_fork_drops_leaves_no_entry_and_no_signal() {
        let root = FocusRoot::new();
        crate::engine::turn(&root, |_| window_focus_mut(|_| {}));
        let (salt, focused, visible) = (root.salt, root.focused, root.visible);
        let other = FocusRoot::new();
        // Another window's turn: the first is no longer current, so the
        // fork drops its last owner.
        crate::engine::turn(&other, |_| ());
        {
            let _cold = crate::engine::ColdFrame::begin();
            drop(root);
        }
        // The release is live work, run by the live loop after the fork.
        crate::engine::run_releases();
        assert!(!WINDOW_FOCUS.with_borrow(|w| w.contains_key(&salt)), "no entry left");
        assert!(!focused.is_alive() && !visible.is_alive(), "no signal left");
        drop(other);
    }

    /// ADR-0034, S4: a check that clones a window's root and faults (the
    /// clone forgotten, as a runaway's set-aside does) leaves the
    /// window's end to its live owner: closed, it leaves no entry and no
    /// signal, though an `Rc` to it lives on.
    #[cfg(debug_assertions)]
    #[test]
    #[expect(clippy::mem_forget, reason = "a runaway's set-aside clone, as the check makes it")]
    fn a_window_a_checks_forgotten_clone_holds_ends_with_its_owner() {
        let root = FocusRoot::new();
        crate::engine::turn(&root, |_| window_focus_mut(|_| {}));
        let (salt, focused, visible) = (root.salt, root.focused, root.visible);
        {
            let _cold = crate::engine::ColdFrame::begin();
            std::mem::forget(Rc::clone(&root));
        }
        assert!(root.begin_close());
        let settled = crate::engine::turn(&root, |_| root.settle_close());
        root.finish_close(settled == Some(Ok(())));
        drop(root);
        assert!(!WINDOW_FOCUS.with_borrow(|w| w.contains_key(&salt)), "no entry left");
        assert!(!focused.is_alive() && !visible.is_alive(), "no signal left");
        assert_eq!(crate::engine::releases_pending(), 0, "nothing left pending");
    }

    /// Codex 4 (F1 review 3): focus to hand on asked live, then asked
    /// again in a fork: after the rollback the live request is the one
    /// kept, as the window's state a fork writes is logged, not behind the
    /// pointer the registry puts back.
    #[cfg(debug_assertions)]
    #[test]
    fn a_hand_on_a_fork_asks_leaves_the_live_one() {
        let root = FocusRoot::new();
        let (live, forked) = (FocusHandle::new(), FocusHandle::new());
        let asked = crate::engine::turn(&root, |_| {
            hand_on(live.id(), crate::access::NodeRef::new(), None);
            {
                let _cold = crate::engine::ColdFrame::begin();
                hand_on(forked.id(), crate::access::NodeRef::new(), None);
            }
            window_focus(|w| w.and_then(|w| w.hand_on).map(|h| h.from))
        })
        .flatten();
        assert_eq!(asked, Some(live.id()), "the live request, kept");
    }

    /// ADR-0034, S7 (the `WindowFocus` split): a check's `hand_on` copies
    /// the small hand-on, sharing the window's published focusable set,
    /// however large: it never copies the live set. The live set is as it
    /// was after the check, and refilled in place live again.
    #[cfg(debug_assertions)]
    #[test]
    fn a_checks_hand_on_shares_the_published_focusables() {
        let root = FocusRoot::new();
        let into = crate::access::NodeRef::new();
        let (live, shared, same) = crate::engine::turn(&root, |_| {
            window_focus_mut(|w| {
                let f = Rc::make_mut(&mut w.focusables);
                f.hidden = (0..100_000).map(FocusId).collect();
            });
            let live = window_focus(|w| w.map(|w| Rc::as_ptr(&w.focusables)));
            let (shared, same) = crate::engine::in_cold_frame(|| {
                hand_on(FocusId(1), into, None);
                window_focus(|w| {
                    let w = w.map(|w| (Rc::as_ptr(&w.focusables), w.hand_on.map(|h| h.from)));
                    (w.map(|w| w.0) == live, w.and_then(|w| w.1) == Some(FocusId(1)))
                })
            });
            (live, shared, same)
        })
        .unwrap_or_default();
        assert!(
            shared && same,
            "the check's entry shares the set and has its own hand-on"
        );
        let _ = crate::engine::turn(&root, |_| {
            let after = window_focus(|w| {
                w.map(|w| {
                    (
                        Rc::as_ptr(&w.focusables),
                        w.focusables.hidden.len(),
                        w.hand_on.is_none(),
                    )
                })
            });
            assert_eq!(after, live.map(|p| (p, 100_000, true)), "live is whole");
            let unique = WINDOW_FOCUS.with_borrow(|m| m.get(&root.salt).map(|w| Rc::strong_count(&w.focusables)));
            assert_eq!(unique, Some(1), "live owns its set alone again: refilled in place");
        });
    }
}
