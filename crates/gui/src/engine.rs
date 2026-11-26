//! Per-thread engine state, each piece declared once with what a cold frame
//! does to it (ADR-0013's check that incremental equals cold).
//!
//! Every piece of state the engine keeps per thread is an [`engine_local!`]
//! (raw `thread_local!` is banned in gui by `clippy.toml`, without
//! exception: the pieces live in the slots of the `gui-tls` crate), and
//! declares one policy:
//!
//! - **Fresh**: the cold frame sees a new, empty instance, dropped at its
//!   end: queues (timers, mailboxes, hooks due) and every cache, so the
//!   frame reads no cache and warms none for the live frames.
//! - **Fork**: the cold frame may change it, and it is put back exactly as
//!   it was at the end: counters handing out ids, the focus, the reactive
//!   arena and the instance tree (those two by an undo log, so a check
//!   costs what the frame writes, not what the thread holds).
//! - **Shared**: neither: only for what sharing cannot change a result of
//!   (a handle whose state lives in the forked runtime, data never
//!   written), each with its reason.
//!
//! Three rules follow (ADR-0028):
//!
//! - **An id is a leased block and a Fork counter** ([`unique_id`]): the
//!   counters a fork moves are put back, so what it numbered is numbered
//!   again live; a block it leased is wasted, never handed out twice.
//! - **A handle is an id; its state is registered** ([`ForkSlab`]): a
//!   `Clone` type sharing cells behind `Rc` is an alias no log sees, so
//!   `ListState`, `ScrollHandle` and `Columns` are `Copy` ids into a
//!   `[Fork(log)]` slab.
//! - **A fork drains no queue of the outside world**: a mailbox delivers
//!   nothing in one, and what it sends goes with it ([`forking`]).
//!
//! No other process-wide state: `tests/process_state.rs` fails on a
//! `static` that is neither registered, immutable nor listed with its
//! reason.
//!
//! [`ColdFrame`] enters every registered piece by moves of data alone, and
//! refuses to open while a use of one is under way. As it ends, it drops
//! everything the frame made while the frame is still open, each drop
//! under its own catch, and only then puts the live state back by moves,
//! so a value whose `Drop` panics or reaches engine state reaches the
//! frame's, and leaves the live state whole; the panic goes on after
//! ([`ColdFrame::teardown`], ADR-0028).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::sync::atomic::AtomicUsize;

#[cfg(all(test, debug_assertions))]
mod disposal_cost;
mod ends;
pub(crate) use ends::{Ending, Ends};
#[cfg(debug_assertions)]
mod frame;
#[cfg(all(test, debug_assertions))]
mod r14;
mod release;
#[cfg(all(test, debug_assertions))]
mod verify;
mod world_cell;
pub use world_cell::{WorldCell, WorldRefCell};

#[cfg(all(test, debug_assertions))]
pub(crate) use frame::ColdFrame;
#[cfg(debug_assertions)]
pub(crate) use frame::{Ended, fork, in_cold_frame, quarantined};
pub use frame_report::{CheckReport, Mismatch};
mod frame_report;
pub use log_key::{LogCell, LogKey, LogMut};
mod log_key;
pub(crate) use release::released;
pub use release::shutdown;
#[cfg(test)]
pub(crate) use release::{pending as releases_pending, run_releases, when_live};
mod turn;
pub(crate) use turn::{
    Commit, Entering, Failed, Gather, Owned, Stored, ThisTurn, attempt, commit, commit_here, contained, dispatching,
    drop_quietly, go_on, in_commit, in_turn, raise, said_of, stored_kind, turn, turn_own,
};
pub use turn::{Site, Turn, call_out, stored_kinds};

/// What a cold frame does to one piece of state: `enter` gives what to put
/// back; `drain` takes what the frame made out of the piece while the frame
/// still runs (given what `enter` gave), one box per element, so each drops
/// still isolated and on its own (ADR-0034, S9); `exit` puts the live state
/// back by moves and gives nothing back: whatever the frame left is
/// drained before it, so the switch to live runs no user code (S1); `free`
/// says whether no use of the piece is under way (a fork opens only when
/// none is, E1). `enter`, `drain` and `exit` move data and run no user
/// code. Nothing else: a fork never runs live code, so no piece is ever
/// asked to show its live state while a frame is open. An init makes its
/// value from constants alone: one that reads engine state is refused at
/// its first touch (S8), so every piece's value is entered the same way.
///
/// Engine-private (ADR-0034, S7): only `gui` makes one, by the policies
/// [`engine_local!`] names; a crate outside it declares a piece with a
/// policy, never with hooks of its own.
#[derive(Clone, Copy)]
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
pub(crate) struct Entry {
    pub(crate) name: &'static str,
    piece: &'static dyn Piece,
}

#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
impl Entry {
    pub(crate) fn enter(self) -> Box<dyn Any> {
        self.piece.enter()
    }

    pub(crate) fn drain(self, saved: &dyn Any) -> Vec<Box<dyn Any>> {
        self.piece.drain(saved)
    }

    /// The piece's slot.
    pub(crate) fn slot(self) -> usize {
        self.piece.slot()
    }

    pub(crate) fn exit(self, saved: Box<dyn Any>) {
        self.piece.exit(saved);
    }

    pub(crate) fn free(self) -> bool {
        self.piece.free()
    }
}

/// A registered piece as a cold frame reaches it ([`Entry`]).
trait Piece {
    fn enter(&'static self) -> Box<dyn Any>;
    fn drain(&'static self, saved: &dyn Any) -> Vec<Box<dyn Any>>;
    fn exit(&'static self, saved: Box<dyn Any>);
    fn free(&'static self) -> bool;
    /// Its slot: what the switch's watch notes it by.
    fn slot(&'static self) -> usize;
}

/// A piece's policy: its name and how a cold frame enters, drains and
/// exits it. Made only by [`EngineKey`]'s policy constructors.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
pub struct Policy<T: 'static> {
    name: &'static str,
    hooks: Hooks<T>,
}

impl<T: 'static> Clone for Policy<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Policy<T> {}

/// The hooks of a [`Policy`]: one of the stated policies, given the piece,
/// or `gui`'s own custom undo log (`EngineKey::custom`).
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
enum Hooks<T: 'static> {
    Keyed {
        enter: fn(&'static EngineKey<T>) -> Box<dyn Any>,
        drain: fn(&'static EngineKey<T>, &dyn Any) -> Drained,
        exit: fn(&'static EngineKey<T>, Box<dyn Any>),
    },
    Custom {
        enter: fn() -> Box<dyn Any>,
        drain: fn(&dyn Any) -> Drained,
        exit: fn(Box<dyn Any>),
    },
}

/// What a drain takes out of a piece: one box per element (S9).
type Drained = Vec<Box<dyn Any>>;

impl<T: 'static> Clone for Hooks<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Copy for Hooks<T> {}

#[cfg(debug_assertions)]
impl<T: 'static> Piece for EngineKey<T> {
    fn enter(&'static self) -> Box<dyn Any> {
        match self.entry.map(|p| p.hooks) {
            Some(Hooks::Keyed { enter, .. }) => enter(self),
            Some(Hooks::Custom { enter, .. }) => enter(),
            None => Box::new(()),
        }
    }

    fn drain(&'static self, saved: &dyn Any) -> Vec<Box<dyn Any>> {
        match self.entry.map(|p| p.hooks) {
            Some(Hooks::Keyed { drain, .. }) => drain(self, saved),
            Some(Hooks::Custom { drain, .. }) => drain(saved),
            None => Vec::new(),
        }
    }

    fn exit(&'static self, saved: Box<dyn Any>) {
        match self.entry.map(|p| p.hooks) {
            Some(Hooks::Keyed { exit, .. }) => exit(self, saved),
            Some(Hooks::Custom { exit, .. }) => exit(saved),
            None => {}
        }
    }

    fn free(&'static self) -> bool {
        self.is_free()
    }

    fn slot(&'static self) -> usize {
        gui_tls::slot_of(&self.slot)
    }
}

#[cfg(not(debug_assertions))]
impl<T: 'static> Piece for EngineKey<T> {
    fn enter(&'static self) -> Box<dyn Any> {
        Box::new(())
    }

    fn drain(&'static self, _: &dyn Any) -> Vec<Box<dyn Any>> {
        Vec::new()
    }

    fn exit(&'static self, _: Box<dyn Any>) {}

    fn free(&'static self) -> bool {
        true
    }

    fn slot(&'static self) -> usize {
        gui_tls::slot_of(&self.slot)
    }
}

/// `[Fresh]`: the live value moved out at entry, the fork's moved out by
/// the drain, the live one moved back whole at exit.
#[cfg(debug_assertions)]
fn fresh_enter<T: 'static>(key: &'static EngineKey<T>) -> Box<dyn Any> {
    key.drained()
        .map_or_else(|| -> Box<dyn Any> { Box::new(()) }, |v| Box::new(v))
}

/// The fork's value taken apart, one box per element ([`Parts`], S9): two
/// elements whose drops panic never drop in one unwind.
#[cfg(debug_assertions)]
fn fresh_drain<T: Parts>(key: &'static EngineKey<T>, _: &dyn Any) -> Drained {
    let mut out = Vec::new();
    if let Some(v) = key.drained() {
        v.parts(&mut out);
    }
    out
}

#[cfg(debug_assertions)]
fn fresh_exit<T: 'static>(key: &'static EngineKey<T>, saved: Box<dyn Any>) {
    key.restore_whole(saved);
}

/// `[Fork]`: an O(1) copy of the live value kept, the fork's moved out by
/// the drain, the live one moved back at exit ([`Snapshot`]).
#[cfg(debug_assertions)]
fn copy_enter<T: Snapshot>(key: &'static EngineKey<T>) -> Box<dyn Any> {
    key.with(|v| fork_enter(v))
}

#[cfg(debug_assertions)]
fn copy_drain<T: Snapshot>(key: &'static EngineKey<T>, saved: &dyn Any) -> Vec<Box<dyn Any>> {
    key.switch_with(|v| fork_drain(v, saved))
        .flatten()
        .into_iter()
        .collect()
}

#[cfg(debug_assertions)]
fn copy_exit<T: Snapshot>(key: &'static EngineKey<T>, saved: Box<dyn Any>) {
    key.restore(saved);
}

/// `[Fork(log)]`: the collection's own overlay or undo log ([`Logged`]).
#[cfg(debug_assertions)]
fn log_enter<L: Logged>(key: &'static EngineKey<RefCell<L>>) -> Box<dyn Any> {
    key.with(|v| v.borrow_mut().begin());
    Box::new(())
}

#[cfg(debug_assertions)]
fn log_drain<L: Logged>(key: &'static EngineKey<RefCell<L>>, _: &dyn Any) -> Vec<Box<dyn Any>> {
    key.with(|v| v.borrow_mut().drain())
}

#[cfg(debug_assertions)]
fn log_exit<L: Logged>(key: &'static EngineKey<RefCell<L>>, _: Box<dyn Any>) {
    key.switch_with(|v| v.borrow_mut().rollback());
}

/// A piece not yet registered ([`Held`]).
#[cfg(debug_assertions)]
const NEW: u8 = 0;
/// A piece registered, its value made where the thread's code made it.
#[cfg(debug_assertions)]
const MADE: u8 = 1;
/// A `[Fresh]` piece holding nothing while a fork runs: its live value
/// moved out as the fork entered it (E1: entry runs no init), or its fork
/// value moved out by the drain. Its init runs where it is next touched,
/// inside the fork's body or a destructor the drain runs under its catch;
/// the next drain round takes that value out.
#[cfg(debug_assertions)]
const DRAINED: u8 = 4;

/// The cell an [`engine_local!`] holds. In debug builds it may hold no
/// value (`DRAINED`: a `[Fresh]` piece a check emptied), and says where it
/// stands: `NEW`, `MADE` or `DRAINED`. A value is
/// in it only once its init has returned (as `OnceCell` holds one): an
/// init that panics leaves it as it was, and the next touch runs it again.
pub struct Held<T> {
    #[cfg(not(debug_assertions))]
    value: T,
    #[cfg(debug_assertions)]
    value: RefCell<Option<T>>,
    #[cfg(debug_assertions)]
    state: Cell<u8>,
}

impl<T> Held<T> {
    const fn new(value: T) -> Self {
        Self {
            #[cfg(not(debug_assertions))]
            value,
            #[cfg(debug_assertions)]
            value: RefCell::new(Some(value)),
            #[cfg(debug_assertions)]
            state: Cell::new(NEW),
        }
    }

    /// `f` given the value.
    #[cfg(not(debug_assertions))]
    #[inline]
    fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.value)
    }

    /// `f` given the value: a piece unset is made before it is read
    /// ([`EngineKey::registered`]), so one is always there.
    #[cfg(debug_assertions)]
    #[expect(clippy::panic, reason = "a piece read while unset is a bug in gui")]
    fn read<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let value = self.value.borrow();
        match value.as_ref() {
            Some(v) => f(v),
            None => panic!("engine state {} read while unset", std::any::type_name::<T>()),
        }
    }
}

/// A piece of per-thread engine state ([`engine_local!`]): used as a
/// `LocalKey` is (`with`, and `get`/`set`/`replace` and `with_borrow_mut`
/// for cells).
pub struct EngineKey<T: 'static> {
    slot: AtomicUsize,
    init: fn() -> T,
    entry: Option<Policy<T>>,
    /// A Shared piece: first touched in a fork, the fork gets a stand-in.
    #[cfg_attr(
        not(debug_assertions),
        expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
    )]
    shared: bool,
}

impl<T: 'static> EngineKey<T> {
    /// A piece of `gui`'s own, with `entry` (or none: the engine's own
    /// bookkeeping, which no fork enters).
    #[must_use]
    pub(crate) const fn new(init: fn() -> T, entry: Option<Policy<T>>) -> Self {
        Self {
            slot: AtomicUsize::new(gui_tls::UNSET),
            init,
            entry,
            shared: false,
        }
    }

    /// A `[Fresh]` piece ([`engine_local!`]).
    #[doc(hidden)]
    #[must_use]
    pub const fn fresh(name: &'static str, init: fn() -> T) -> Self
    where
        T: Parts,
    {
        #[cfg(debug_assertions)]
        let hooks = Hooks::Keyed {
            enter: fresh_enter::<T>,
            drain: fresh_drain::<T>,
            exit: fresh_exit::<T>,
        };
        #[cfg(debug_assertions)]
        return Self::new(init, Some(Policy { name, hooks }));
        #[cfg(not(debug_assertions))]
        {
            let _ = name;
            Self::new(init, None)
        }
    }

    /// A plain `[Fork]` piece: a cell copied whole in O(1) ([`Snapshot`]).
    #[doc(hidden)]
    #[must_use]
    pub const fn copied(name: &'static str, init: fn() -> T) -> Self
    where
        T: Snapshot,
    {
        #[cfg(debug_assertions)]
        let hooks = Hooks::Keyed {
            enter: copy_enter::<T>,
            drain: copy_drain::<T>,
            exit: copy_exit::<T>,
        };
        #[cfg(debug_assertions)]
        return Self::new(init, Some(Policy { name, hooks }));
        #[cfg(not(debug_assertions))]
        {
            let _ = name;
            Self::new(init, None)
        }
    }

    /// A piece with `gui`'s own undo log: hooks of its own, which move data
    /// and run no user code. Engine-private (S7): no other crate states
    /// hooks.
    #[must_use]
    #[cfg(debug_assertions)]
    pub(crate) const fn custom(
        name: &'static str,
        init: fn() -> T,
        enter: fn() -> Box<dyn Any>,
        drain: fn(&dyn Any) -> Vec<Box<dyn Any>>,
        exit: fn(Box<dyn Any>),
    ) -> Self {
        Self::new(
            init,
            Some(Policy {
                name,
                hooks: Hooks::Custom { enter, drain, exit },
            }),
        )
    }

    /// A Shared piece ([`engine_local!`]): neither entered nor put back by
    /// a fork, and never made by one ([`EngineKey::stand_in`]).
    #[must_use]
    pub const fn shared(init: fn() -> T) -> Self {
        Self {
            slot: AtomicUsize::new(gui_tls::UNSET),
            init,
            entry: None,
            shared: true,
        }
    }

    /// A Shared piece first touched while a fork runs: the fork never runs
    /// live code (ADR-0028), so it is not made here. The fork gets a
    /// stand-in of its own, made by the same init inside it, the same one
    /// each time it is touched again in this fork: what a discarded run
    /// makes is the run's, dropped with it (research-f1: React's Suspense,
    /// Vue's discarded branch, PostgreSQL's relations made in an aborted
    /// transaction). The fork's result stands: the init reads no state a
    /// fork changes ([`shared_init`]), so the stand-in is what live code
    /// would make, and live code makes the piece at its own first touch,
    /// in its own order, as with no check. One per piece per fork: it lives
    /// until the fork has dropped all else it made, its drain included
    /// ([`ColdFrame::teardown`]'s loop), so a drop that reaches the piece reaches
    /// the stand-in the fork's values were made from. `None`: the piece
    /// is made, or no fork runs.
    #[cfg(debug_assertions)]
    #[expect(
        clippy::panic,
        reason = "a stand-in reached after its fork dropped it would be a second one: a bug in a destructor"
    )]
    fn stand_in(&'static self, slot: Option<&gui_tls::Slot>) -> Option<std::rc::Rc<Held<T>>> {
        if !self.shared || slot.is_none_or(|s| s.get().is_some()) || !is_cold() {
            return None;
        }
        let at = gui_tls::slot_of(&self.slot);
        let (made, dropped) = REGISTRY.with(|r| {
            let r = r.borrow();
            let made = r
                .stand_ins
                .iter()
                .find(|(s, _)| *s == at)
                .map(|(_, v)| std::rc::Rc::clone(v));
            (made, r.stood_down.contains(&at))
        });
        if let Some(made) = made {
            return std::rc::Rc::downcast::<Held<T>>(made).ok();
        }
        if dropped {
            panic!(
                "the stand-in of the Shared piece {} was reached after its fork dropped it: a stand-in's drop reaches no other Shared piece",
                std::any::type_name::<T>()
            );
        }
        let held = gui_tls::initing(at, || std::rc::Rc::new(Held::new((self.init)())))
            .unwrap_or_else(|| reaches_itself::<T>());
        REGISTRY.with(|r| {
            let any: std::rc::Rc<dyn Any> = std::rc::Rc::<Held<T>>::clone(&held);
            r.borrow_mut().stand_ins.push((at, any));
        });
        Some(held)
    }

    #[expect(
        clippy::panic,
        reason = "more engine state than gui-tls holds, or a slot of another type: a bug in gui"
    )]
    fn held<'a>(&'static self, slot: Option<&'a gui_tls::Slot>) -> &'a Held<T> {
        let Some(slot) = slot else {
            panic!("more than {} pieces of engine state", gui_tls::CAPACITY);
        };
        let any = slot.get_or_init(|| {
            gui_tls::initing(gui_tls::slot_of(&self.slot), || -> Box<dyn Any> {
                // A registered piece's init: whether it read engine state.
                #[cfg(debug_assertions)]
                if let Some(policy) = self.entry {
                    let (value, read) = made_by(self.init);
                    if read {
                        init_read(policy.name);
                    }
                    return Box::new(Held::new(value));
                }
                Box::new(Held::new((self.init)()))
            })
            .unwrap_or_else(|| reaches_itself::<T>())
        });
        let Some(held) = any.downcast_ref::<Held<T>>() else {
            panic!("an engine state slot holds another type");
        };
        held
    }

    /// A registered piece's first use registers it, and enters it into the
    /// running check if one runs. Its init read no engine state (S8,
    /// checked as it is made), so its value is the one live code makes:
    /// entered, it stands, and its identity sources with it. `check`: a
    /// Shared init may not read it ([`no_forked_read`]); the reactive
    /// arena's own bookkeeping passes `false` and checks each value read
    /// itself ([`no_reactive_read`]).
    #[inline]
    fn registered(&'static self, h: &Held<T>, check: bool) {
        #[cfg(debug_assertions)]
        if let Some(policy) = self.entry {
            let e = Entry {
                name: policy.name,
                piece: self,
            };
            if check {
                no_forked_read(e.name);
            }
            let state = h.state.get();
            if state == MADE {
                return;
            }
            let at = gui_tls::slot_of(&self.slot);
            // A `[Fresh]` piece the fork's entry or drain emptied: made
            // here, by user code inside the fork, entered as it was.
            if state == DRAINED {
                self.refill(h, at);
                return;
            }
            h.state.set(MADE);
            register(e, at, std::any::type_name::<T>());
            // A `[Fresh]` piece first registered in the fork: entering it
            // moved the value just made out, as the one to put back.
            if h.state.get() == DRAINED {
                self.refill(h, at);
            }
        }
        #[cfg(not(debug_assertions))]
        let _ = (h, self.entry, check);
    }

    /// A `DRAINED` piece made by its init, where user code touched it.
    #[cfg(debug_assertions)]
    fn refill(&'static self, h: &Held<T>, at: usize) {
        let value = gui_tls::initing(at, self.init).unwrap_or_else(|| reaches_itself::<T>());
        *h.value.borrow_mut() = Some(value);
        h.state.set(MADE);
    }

    pub fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.access(true, f)
    }

    /// [`Self::with`] for the reactive arena's own bookkeeping, which a
    /// Shared init may reach to make a handle: each value it reads is
    /// checked where it is read ([`no_reactive_read`]).
    pub(crate) fn with_arena<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        self.access(false, f)
    }

    fn access<R>(&'static self, check: bool, f: impl FnOnce(&T) -> R) -> R {
        gui_tls::with(gui_tls::slot_of(&self.slot), |slot| {
            #[cfg(debug_assertions)]
            if let Some(h) = self.stand_in(slot) {
                return h.read(f);
            }
            let h = self.held(slot);
            self.registered(h, check);
            h.read(f)
        })
    }

    /// [`Self::with`], unless the thread is ending.
    ///
    /// # Errors
    /// The thread's state is gone.
    pub fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Result<R, std::thread::AccessError> {
        self.try_access(true, f)
    }

    /// [`Self::try_with`] for the reactive arena's own bookkeeping
    /// ([`Self::with_arena`]).
    pub(crate) fn try_with_arena<R>(&'static self, f: impl FnOnce(&T) -> R) -> Result<R, std::thread::AccessError> {
        self.try_access(false, f)
    }

    fn try_access<R>(&'static self, check: bool, f: impl FnOnce(&T) -> R) -> Result<R, std::thread::AccessError> {
        gui_tls::try_with(gui_tls::slot_of(&self.slot), |slot| {
            #[cfg(debug_assertions)]
            if let Some(h) = self.stand_in(slot) {
                return h.read(f);
            }
            let h = self.held(slot);
            self.registered(h, check);
            h.read(f)
        })
    }
}

/// A registered piece's init read other engine state (ADR-0034, S8): an
/// init is a constant, so the value a check makes is the one live code
/// makes, and no piece is ever the check's alone.
#[cfg(debug_assertions)]
#[expect(
    clippy::panic,
    reason = "an init that reads engine state is a bug in its declaration"
)]
fn init_read(name: &str) -> ! {
    panic!(
        "the init of the engine state {name} read other engine state: an init makes its value from constants \
         alone (ADR-0034, S8); read the state where the piece is used"
    )
}

/// A piece's init reached the piece itself, live or making a fork's
/// stand-in ([`gui_tls::initing`]): it would never end.
#[expect(
    clippy::panic,
    reason = "an init that reaches its own piece never ends: a bug in its declaration"
)]
fn reaches_itself<T>() -> ! {
    panic!(
        "the init of the engine state {} reaches the piece itself",
        std::any::type_name::<T>()
    )
}

impl<T: 'static> EngineKey<Cell<T>> {
    pub fn get(&'static self) -> T
    where
        T: Copy,
    {
        self.with(Cell::get)
    }

    pub fn set(&'static self, value: T) {
        self.with(|c| c.set(value));
    }

    pub fn replace(&'static self, value: T) -> T {
        self.with(|c| c.replace(value))
    }
}

// A `[Fork(log)]` piece's key is a [`LogKey`]: no whole-value `take`,
// `set` or cell access, as its value holds the check's log (S7).

impl<T: 'static> EngineKey<RefCell<T>> {
    pub fn set(&'static self, value: T) {
        self.with(|c| drop(c.replace(value)));
    }

    pub fn take(&'static self) -> T
    where
        T: Default,
    {
        self.with(RefCell::take)
    }

    pub fn with_borrow_mut<R>(&'static self, f: impl FnOnce(&mut T) -> R) -> R {
        self.with(|c| f(&mut c.borrow_mut()))
    }
}

/// A cell whose value a `[Fork]` swaps ([`Fork`](engine_local!)).
pub trait Slot: 'static {
    type Inner: 'static;
    fn put(&self, value: Self::Inner) -> Self::Inner;
    fn from_value(value: Self::Inner) -> Self;
}

impl<T: 'static> Slot for Cell<T> {
    type Inner = T;
    fn put(&self, value: T) -> T {
        self.replace(value)
    }
    fn from_value(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: 'static> Slot for RefCell<T> {
    type Inner = T;
    fn put(&self, value: T) -> T {
        self.replace(value)
    }
    fn from_value(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: 'static> EngineKey<T> {
    /// Run `f` on this piece's `Held`, if made, with no registration, init
    /// or check: the teardown's own moves (T1, T4), which run no user code.
    #[cfg(debug_assertions)]
    fn raw<R>(&'static self, f: impl FnOnce(&Held<T>) -> R) -> Option<R> {
        gui_tls::try_with_quiet(gui_tls::slot_of(&self.slot), |slot| {
            slot.and_then(|s| s.get())
                .and_then(|v| v.downcast_ref::<Held<T>>())
                .map(f)
        })
        .ok()
        .flatten()
    }

    /// `f` given the value, as the switch back to live reaches it: no
    /// registration, init or check, and unnoted by the switch's tripwire
    /// (ADR-0034, S1). `None`: the piece holds no value.
    #[cfg(debug_assertions)]
    pub(crate) fn switch_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Option<R> {
        self.raw(|h| h.value.try_borrow().ok().and_then(|v| v.as_ref().map(f)))
            .flatten()
    }

    /// Whether no use of this piece is under way: a fork opens only then
    /// (E1), so its exit can move every value back.
    #[cfg(debug_assertions)]
    fn is_free(&'static self) -> bool {
        self.raw(|h| h.value.try_borrow_mut().is_ok()).unwrap_or(true)
    }

    /// A `[Fresh]` piece put back (S1): the live value `saved` (what
    /// [`Self::drained`] moved out at entry) moved in whole. The piece holds
    /// nothing by now (the drain loop emptied it); a value still there is a
    /// bug in gui, quarantined unread ([`quarantined`]). No init runs.
    #[cfg(debug_assertions)]
    fn restore_whole(&'static self, saved: Box<dyn Any>) {
        let Ok(live) = saved.downcast::<T>() else {
            return;
        };
        let mut live = Some(*live);
        let left = self.raw(|h| {
            let mut value = h.value.borrow_mut();
            let was = std::mem::replace(&mut *value, live.take());
            h.state.set(MADE);
            was
        });
        if let Some(Some(left)) = left {
            quarantined(self.entry.map_or("a piece", |e| e.name), Box::new(left));
        }
    }

    /// A `[Fresh]` piece's entry (E1) and drain (T1): its value moved out
    /// whole, with no user code, and the piece left `DRAINED`. At entry
    /// that is the live value, to put back; in the drain, the fork's, to
    /// drop under its own catch. No init runs here: user code that touches
    /// the piece later makes it then.
    #[cfg(debug_assertions)]
    fn drained(&'static self) -> Option<T> {
        self.raw(|h| {
            let value = taken(h);
            if value.is_some() {
                h.state.set(DRAINED);
            }
            value
        })
        .flatten()
    }
}

impl<T: Snapshot> EngineKey<T> {
    /// Put back the live value `saved` (S1): a move into the cell, or a cell
    /// made of it where the drain left none. What the cell held must be the
    /// live value itself (the fork never wrote it, or the drain put a copy
    /// of it there, [`fork_drain`]: a `Copy` value or the very handle moved
    /// back), so dropping it is a count-down that runs no code. Anything
    /// else (a value a destructor wrote after the drain's last round, a bug
    /// in gui) is quarantined and the check faults, never dropped here
    /// (Codex r15-1). No registration, init or check runs.
    #[cfg(debug_assertions)]
    fn restore(&'static self, saved: Box<dyn Any>) {
        let Ok(live) = saved.downcast::<T::Inner>() else {
            return;
        };
        let mut live = Some(*live);
        let copy = self.raw(|h| {
            let mut value = h.value.borrow_mut();
            let live = live.take()?;
            match value.as_ref() {
                Some(cell) => {
                    let same = cell.same(&live);
                    Some((cell.put(live), same))
                }
                None => {
                    *value = Some(T::from_value(live));
                    h.state.set(MADE);
                    None
                }
            }
        });
        match copy.flatten() {
            Some((copy, true)) => drop(copy),
            Some((left, false)) => quarantined(self.entry.map_or("a piece", |e| e.name), Box::new(left)),
            None => {}
        }
    }
}

/// What `h` holds, taken out (T1). A fork's teardown runs outside every use
/// of a piece (a fork is scoped by a closure), so none is borrowed here.
#[cfg(debug_assertions)]
#[expect(
    clippy::panic,
    reason = "a piece borrowed across a fork's end is a bug in gui: forks are scoped by a closure"
)]
fn taken<T>(h: &Held<T>) -> Option<T> {
    let Ok(mut value) = h.value.try_borrow_mut() else {
        panic!(
            "a fork ended inside a use of {}: a fork is scoped by a closure",
            std::any::type_name::<T>()
        );
    };
    value.take()
}

/// A cell a plain Fork copies whole, in O(1): a `Copy` value, or one
/// handle (`Rc`, `Arc`). A collection is never one: it keeps an undo log
/// ([`Logged`], `[Fork(log)]`), so a check costs what the frame writes.
///
/// Sealed (ADR-0034, S7): its copy runs no user code, so only `gui`
/// implements it.
pub trait Snapshot: Slot + sealed::Sealed {
    fn copy(value: &Self::Inner) -> Self::Inner;
    fn snapshot(&self) -> Self::Inner;
    /// Whether the cell holds `live` itself (the same handle, or a `Copy`
    /// value, whose drop runs no code): what the fork never wrote, which
    /// its drain leaves alone and the switch may drop as a count-down.
    fn same(&self, live: &Self::Inner) -> bool;
}

/// The seal of the fork's storage traits (S7): what implements them runs
/// no user code where a fork moves or copies it, which only `gui` states.
pub(crate) mod sealed {
    /// Implemented in `gui` only.
    pub trait Sealed {}
}

impl<T: Copy + 'static> sealed::Sealed for Cell<T> {}
impl<T: ?Sized + 'static> sealed::Sealed for RefCell<Option<std::rc::Rc<T>>> {}
impl<T: ?Sized + 'static> sealed::Sealed for RefCell<Option<std::sync::Arc<T>>> {}

impl<T: Copy + 'static> Snapshot for Cell<T> {
    fn copy(value: &T) -> T {
        *value
    }
    fn snapshot(&self) -> T {
        self.get()
    }
    /// A `Copy` value drops nothing: whatever it is, nothing to hand over.
    fn same(&self, _: &T) -> bool {
        true
    }
}

impl<T: ?Sized + 'static> Snapshot for RefCell<Option<std::rc::Rc<T>>> {
    fn copy(value: &Option<std::rc::Rc<T>>) -> Option<std::rc::Rc<T>> {
        value.clone()
    }
    fn snapshot(&self) -> Option<std::rc::Rc<T>> {
        self.borrow().clone()
    }
    fn same(&self, live: &Option<std::rc::Rc<T>>) -> bool {
        self.try_borrow().is_ok_and(|v| match (v.as_ref(), live.as_ref()) {
            (None, None) => true,
            (Some(a), Some(b)) => std::rc::Rc::ptr_eq(a, b),
            _ => false,
        })
    }
}

impl<T: ?Sized + 'static> Snapshot for RefCell<Option<std::sync::Arc<T>>> {
    fn copy(value: &Option<std::sync::Arc<T>>) -> Option<std::sync::Arc<T>> {
        value.clone()
    }
    fn snapshot(&self) -> Option<std::sync::Arc<T>> {
        self.borrow().clone()
    }
    fn same(&self, live: &Option<std::sync::Arc<T>>) -> bool {
        self.try_borrow().is_ok_and(|v| match (v.as_ref(), live.as_ref()) {
            (None, None) => true,
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            _ => false,
        })
    }
}

/// State a cold frame forks by an undo log (`[Fork(log)]`): what the frame
/// overwrote is kept on its first write, and put back at its end.
///
/// Sealed (S7): only `gui`'s collections keep a log.
pub trait Logged: sealed::Sealed + 'static {
    /// The frame starts: the log with it.
    fn begin(&mut self);
    /// What the frame made, taken out while it still runs, one box per
    /// element (ADR-0034, S9).
    fn drain(&mut self) -> Vec<Box<dyn Any>>;
    /// The live state back, by moves alone (S1): what the frame made was
    /// drained before; anything left is a bug in gui, quarantined.
    fn rollback(&mut self);
}

/// What a logged collection ([`ForkMap`], [`ForkSlab`]) may hold (ADR-0034,
/// S7, reviews r16 and r17): a value nothing reached through `&` of it
/// changes. A read lends the live value itself (no copy), so one a shared
/// borrow could change (a `Cell`, a `RefCell`, a logged collection nested
/// inside, whose writes go to a log the outer one never drains) would let a
/// check write live state no log records. What implements it:
///
/// - data ([`ForkData`]): gui's scalars, engine ids, arrays, tuples and
///   `Option`s of data, and gui's own structs of data, a closed set (no
///   reference, `Cell`, `Rc` or pointer is);
/// - an app object the engine keeps by handle ([`AppHandle`]): the engine
///   forks which handles exist under which keys, never the object, which is
///   app heap (the render-purity contract, ADR-0034 S7, "What a check
///   isolates");
/// - `Vec`s of either;
/// - gui's own states, each field's rule audited (the ADR's S7 table).
///
/// Each is taken apart by the drain one box per value whose drop may run
/// app code ([`Self::split`], S9): no container goes in one box.
///
/// Sealed by type: this seal and [`ForkData`]'s are private to `gui`. None
/// of these compiles as a logged value.
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, std::cell::Cell<u8>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, std::cell::RefCell<u8>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, gui::engine::ForkSlab<u8>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, gui::engine::ForkMap<u8, u8>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static S: std::cell::RefCell<gui::engine::ForkSlab<std::rc::Rc<std::cell::RefCell<u8>>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, std::cell::Cell<Option<gui::Root>>>> = Default::default();
/// }
/// ```
///
/// The seal is the type, not a statement: another crate cannot implement
/// it, and data holds no reference, `Cell` or `Rc` (review r17, Opus I2
/// and I3, Codex 2).
///
/// ```compile_fail
/// struct Sneaky(std::cell::Cell<u8>);
/// impl gui::engine::value_seal::Sealed for Sneaky {}
/// impl gui::engine::ForkValue for Sneaky {}
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, std::rc::Rc<dyn std::any::Any>>> = Default::default();
/// }
/// ```
///
/// ```compile_fail
/// gui::engine_local! {
///     [Fork(log)] static M: std::cell::RefCell<gui::engine::ForkMap<u8, &'static std::cell::Cell<u32>>> = Default::default();
/// }
/// ```
///
/// Another crate implements neither [`ForkData`] nor its seal, under any
/// name (review r18, Codex 3 and Opus I3: a hand impl admitted a
/// `&'static Cell`, and a check wrote the live cell through it):
///
/// ```compile_fail
/// #[derive(Clone, Copy)]
/// struct Value(&'static std::cell::Cell<u32>);
/// impl gui::engine::fork_data::Sealed for Value {}
/// impl gui::engine::ForkData for Value {}
/// ```
///
/// ```compile_fail
/// use gui::engine::{fork_data::Sealed as Seal, ForkData as Data};
/// #[derive(Clone, Copy)]
/// struct Value(&'static std::cell::Cell<u32>);
/// impl Seal for Value {}
/// impl Data for Value {}
/// ```
///
/// ```compile_fail
/// #[derive(Clone, Copy)]
/// struct Value(u32);
/// type Alias = Value;
/// impl gui::engine::ForkData for Alias {}
/// ```
///
/// Another crate's own struct is no logged value, `Copy` or not: it goes
/// in as a tuple of data or behind an [`AppHandle`].
///
/// ```compile_fail
/// #[derive(Clone, Copy)]
/// struct Row(u32);
/// gui::engine_local! {
///     [Fork(log)] static B: std::cell::RefCell<gui::engine::ForkMap<u8, Row>> = Default::default();
/// }
/// ```
///
/// What does compile (the positive controls of the fixtures above):
///
/// ```
/// use gui::engine::AppHandle;
/// /// A row: its id, where it is, its tag.
/// type Row = (u32, (f32, f32), Option<[u8; 4]>);
/// gui::engine_local! {
///     [Fork(log)] static A: std::cell::RefCell<gui::engine::ForkMap<u8, u8>> = Default::default();
///     [Fork(log)] static B: std::cell::RefCell<gui::engine::ForkMap<u8, Row>> = Default::default();
///     [Fork(log)] static C: std::cell::RefCell<gui::engine::ForkMap<u8, AppHandle<dyn Fn()>>> = Default::default();
///     [Fork(log)] static D: std::cell::RefCell<gui::engine::ForkSlab<Option<(u8, u64)>>> = Default::default();
///     [Fork(log)] static E: std::cell::RefCell<gui::engine::ForkMap<u8, AppHandle<dyn std::any::Any>>> = Default::default();
/// }
/// B.with_borrow_mut(|b| b.replace(1, (7, (0., 0.), None)));
/// assert_eq!(B.with_borrow(|b| b.get(&1).map(|r| r.0)), Some(7));
/// ```
pub trait ForkValue: value_seal::Sealed + 'static {
    /// Taken apart for a check's drain (S9): one box per value whose drop
    /// may run app code, nothing for data. Never a container in one box.
    #[doc(hidden)]
    fn split(self, out: &mut Vec<Box<dyn Any>>)
    where
        Self: Sized;
}

/// The seal of [`ForkValue`]: implemented in `gui` alone (a public trait in
/// a private module, so no other crate can name it). Another crate's own
/// values come as [`ForkData`] or [`AppHandle`].
pub(crate) mod value_seal {
    /// Implemented in `gui` only.
    pub trait Sealed {}
}

/// Plain data, as `bytemuck`'s `Pod` is (ADR-0034, S7, reviews r17 and
/// r18): `Copy`, and every part of it data, so `&` of it changes nothing
/// and its drop runs nothing. A closed set, implemented in `gui` alone: the
/// integers, floats, `bool`, `char`, `()`, `TypeId`, engine ids and
/// geometry, and arrays, tuples (up to four) and `Option`s of data, and
/// `gui`'s own structs whose every field is data (`fork_data_struct!`,
/// which bounds each field's type). A reference, `Cell`, `RefCell`, `Rc`
/// or raw pointer never is: a `&'static Cell<u32>` is `Copy`, and a check
/// could write the live cell through it (Opus r17 I3, Codex r18 3).
///
/// Sealed by a private supertrait: no other crate can implement it, under
/// any name (the compile-fail fixtures on [`ForkValue`]). A derive cannot
/// be told apart from a hand impl, and `bytemuck`'s answer (an `unsafe`
/// trait) is closed to a crate that forbids `unsafe`; so another crate's
/// own value goes in as a tuple of data or behind an [`AppHandle`].
/// Revisit when a crate needs a logged struct of its own that neither fits.
pub trait ForkData: Copy + fork_data::Sealed + 'static {}

/// The seal of [`ForkData`]: a public trait in a private module, so no
/// other crate can name it.
pub(crate) mod fork_data {
    /// Implemented in `gui` only.
    pub trait Sealed {}
}

/// A `gui` struct of data: the struct as written, and [`ForkData`] for it
/// bounded by every field's type, so a field that is not data does not
/// compile (review r18, decision 1: what the derive did, inside the seal).
macro_rules! fork_data_struct {
    (
        $(#[$m:meta])*
        $vis:vis struct $name:ident {
            $($(#[$fm:meta])* $fvis:vis $field:ident : $ty:ty),* $(,)?
        }
    ) => {
        $(#[$m])*
        $vis struct $name {
            $($(#[$fm])* $fvis $field: $ty),*
        }
        impl $crate::engine::fork_data::Sealed for $name where $($ty: $crate::engine::ForkData),* {}
        impl $crate::engine::ForkData for $name where $($ty: $crate::engine::ForkData),* {}
    };
}
pub(crate) use fork_data_struct;

/// gui's own data types (engine ids, geometry), each `Copy` and holding
/// ids, numbers and the immutable place it was made alone: the closed set.
macro_rules! fork_data_impl {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::engine::fork_data::Sealed for $t {}
            impl $crate::engine::ForkData for $t {}
        )*
    };
}

fork_data_impl!(
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    char,
    bool,
    f32,
    f64,
    (),
    std::any::TypeId,
    SlabId,
);

impl<T: ForkData> fork_data::Sealed for Option<T> {}
impl<T: ForkData> ForkData for Option<T> {}
impl<T: ForkData, const N: usize> fork_data::Sealed for [T; N] {}
impl<T: ForkData, const N: usize> ForkData for [T; N] {}
impl<A: ForkData> fork_data::Sealed for (A,) {}
impl<A: ForkData> ForkData for (A,) {}
impl<A: ForkData, B: ForkData> fork_data::Sealed for (A, B) {}
impl<A: ForkData, B: ForkData> ForkData for (A, B) {}
impl<A: ForkData, B: ForkData, C: ForkData> fork_data::Sealed for (A, B, C) {}
impl<A: ForkData, B: ForkData, C: ForkData> ForkData for (A, B, C) {}
impl<A: ForkData, B: ForkData, C: ForkData, D: ForkData> fork_data::Sealed for (A, B, C, D) {}
impl<A: ForkData, B: ForkData, C: ForkData, D: ForkData> ForkData for (A, B, C, D) {}

impl<T: ForkData> value_seal::Sealed for T {}
impl<T: ForkData> ForkValue for T {
    /// Data drops nothing.
    fn split(self, _: &mut Vec<Box<dyn Any>>) {}
}

/// An app object the engine keeps by handle in a logged collection (a
/// context's value, a tooltip's close callback): `Rc` of it, shared.
///
/// What a check isolates of it is the collection's structure, which
/// handles exist under which keys: a check's insert, replace or remove
/// goes to its overlay. The object behind the handle is app heap, as an
/// `Rc` a component's closure captured is: outside the check's isolation,
/// as no engine can snapshot arbitrary app memory. A render and a setup
/// change app heap only through signals (ADR-0034 S7, "What a check
/// isolates": React's render purity, Compose's snapshot isolation of state
/// objects alone). So a check that takes a live `Root` out of an object
/// behind a handle removes it from the live world, and its unmount follows.
///
/// ```
/// use gui::engine::AppHandle;
/// use std::rc::Rc;
/// gui::engine_local! {
///     [Fork(log)] static NAMES: std::cell::RefCell<gui::engine::ForkMap<u8, AppHandle<str>>> = Default::default();
/// }
/// NAMES.with_borrow_mut(|n| n.replace(1, AppHandle::new(Rc::from("live"))));
/// gui::testing::in_check(|| NAMES.with_borrow_mut(|n| n.replace(1, AppHandle::new(Rc::from("checked")))));
/// assert_eq!(NAMES.with_borrow(|n| n.get(&1).map(|h| h.to_string())), Some("live".to_owned()));
/// ```
pub struct AppHandle<T: ?Sized + 'static>(std::rc::Rc<T>);

impl<T: ?Sized + 'static> AppHandle<T> {
    /// The handle of `object`.
    #[must_use]
    pub const fn new(object: std::rc::Rc<T>) -> Self {
        Self(object)
    }

    /// The object, shared.
    #[must_use]
    pub fn rc(&self) -> std::rc::Rc<T> {
        std::rc::Rc::clone(&self.0)
    }
}

impl<T: ?Sized + 'static> Clone for AppHandle<T> {
    fn clone(&self) -> Self {
        Self(std::rc::Rc::clone(&self.0))
    }
}

impl<T: ?Sized + 'static> std::ops::Deref for AppHandle<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + 'static> value_seal::Sealed for AppHandle<T> {}
impl<T: ?Sized + 'static> ForkValue for AppHandle<T> {
    /// One box: the handle, let go of at once if another holds it (a
    /// count-down, no code).
    fn split(self, out: &mut Vec<Box<dyn Any>>) {
        self.0.parts(out);
    }
}

impl<T: ForkValue> value_seal::Sealed for Vec<T> {}
impl<T: ForkValue> ForkValue for Vec<T> {
    /// Each element on its own (Codex r17 1: two callbacks in one box
    /// dropped the second in the first's unwind, an abort).
    fn split(self, out: &mut Vec<Box<dyn Any>>) {
        for v in self {
            v.split(out);
        }
    }
}

/// gui's own value types, audited (S7): no `&` of them changes them. Each
/// is taken apart by its [`Parts`].
macro_rules! fork_value_audited {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::engine::value_seal::Sealed for $t {}
            impl $crate::engine::ForkValue for $t {
                fn split(self, out: &mut Vec<Box<dyn ::std::any::Any>>) {
                    $crate::engine::Parts::parts(self, out);
                }
            }
        )*
    };
}
pub(crate) use fork_value_audited;

/// A value a check copies on its first write to it (ADR-0034, S7): into a
/// [`ForkMap`]'s overlay, or a [`ForkSlab`]'s shadow, once per key or
/// entry however often the check writes it, never before (a read copies
/// nothing). Its copy runs no user code, and states its cost: O(1) for a
/// `Copy` value and a handle (`Rc`, `Arc`, which share what they point to),
/// O(n) for a `Vec` of `Copy` values, and for `gui`'s own state what its
/// type says (shared roots, an overlay of what it observes, callbacks by
/// identity). A value with no such copy goes in a map by replacement only
/// ([`ForkMap::replace`], [`ForkMap::forget`]): a check never copies it.
///
/// Sealed: only `gui` states a copy.
pub trait ForkClone: ForkValue + sealed::Sealed + 'static {
    /// The check's own copy of a live value.
    #[must_use]
    fn fork_clone(&self) -> Self;
}

impl<T: ForkData> sealed::Sealed for T {}
impl<T: ForkData> ForkClone for T {
    /// O(1) in its size: a bit copy.
    fn fork_clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized + 'static> sealed::Sealed for std::rc::Rc<T> {}
impl<T: ?Sized + 'static> sealed::Sealed for std::sync::Arc<T> {}

impl<T: ?Sized + 'static> sealed::Sealed for AppHandle<T> {}
impl<T: ?Sized + 'static> ForkClone for AppHandle<T> {
    /// O(1): the object is shared, its count bumped.
    fn fork_clone(&self) -> Self {
        self.clone()
    }
}

/// `gui`'s own types whose `Clone` copies handles, ids and `Copy` fields
/// alone (no user value, so no user code): their [`ForkClone`] is that
/// `Clone`, at the cost its type documents. Each is a logged value too
/// ([`ForkValue`], audited: the S7 table).
macro_rules! fork_clone_by_clone {
    ($($t:ty),* $(,)?) => {
        $(
            $crate::engine::fork_value_audited!($t);
            // One value: what it holds (handles, ids) drops as one.
            $crate::engine::parts_whole!($t);
            impl $crate::engine::sealed::Sealed for $t {}
            impl $crate::engine::ForkClone for $t {
                fn fork_clone(&self) -> Self {
                    ::std::clone::Clone::clone(self)
                }
            }
        )*
    };
}
pub(crate) use fork_clone_by_clone;

// Engine handles: an id (and the immutable place it was made), whose state
// is the engine's, forked where it lives (the arena, a registry piece).
fork_data_impl!(
    crate::geometry::Bounds,
    crate::reactive::Scope,
    crate::Interaction,
    crate::focus::FocusHandle,
    ThisTurn,
);
impl<T: 'static> fork_data::Sealed for crate::Signal<T> {}
impl<T: 'static> ForkData for crate::Signal<T> {}

impl<T: Copy + 'static> sealed::Sealed for Vec<T> {}
impl<T: ForkData> ForkClone for Vec<T> {
    /// O(n) in its length: a bit copy.
    fn fork_clone(&self) -> Self {
        self.clone()
    }
}

/// What a `[Fresh]` piece holds, as a check's drain takes it apart: one box
/// per element, each dropped on its own under the drain's catch (ADR-0034,
/// S9), so no container of user values is dropped whole and two panicking
/// drops never meet in one unwind (which aborts). Elements whose type has
/// no drop glue stay together: dropping them runs no code.
///
/// Sealed (S7): only `gui` states how a value comes apart, for the
/// containers a piece holds (`Vec`, `Option`, maps and sets, cells, pairs)
/// and its own types.
pub trait Parts: parts_seal::Sealed + 'static {
    /// Its drop runs no app code (review r16, decision 4): the inert std
    /// types (`String`, `Box<str>`, the scalars, a `Weak`, a `Copy` cell)
    /// and the containers of only those, with a hasher that has no drop
    /// glue. A container of inert values goes whole, one box: dropping it
    /// frees memory and nothing else, so it can fault only as a std drop
    /// would, and splitting it would cost a box per element for no
    /// guarantee. A function, not a constant, so that `Box<str>` can be
    /// told apart from the other boxes by its type (a narrower impl for it
    /// is refused by coherence); it folds to a constant when optimised.
    #[must_use]
    fn inert() -> bool {
        false
    }

    /// `self` taken apart into `out`.
    fn parts(self, out: &mut Vec<Box<dyn Any>>);
}

/// `value`, whole, as one box, if it drops anything: the path of an inert
/// value ([`Parts::inert`]).
pub(crate) fn whole<T: 'static>(value: T, out: &mut Vec<Box<dyn Any>>) {
    if std::mem::needs_drop::<T>() {
        out.push(Box::new(value));
    }
}

/// The seal of [`Parts`].
pub(crate) mod parts_seal {
    /// Implemented in `gui` only.
    pub trait Sealed {}
}

impl<T: Copy + 'static> parts_seal::Sealed for Cell<T> {}
impl<T: Copy + 'static> Parts for Cell<T> {
    fn inert() -> bool {
        true
    }
    fn parts(self, _: &mut Vec<Box<dyn Any>>) {}
}

impl<T: Parts> parts_seal::Sealed for RefCell<T> {}
impl<T: Parts> Parts for RefCell<T> {
    fn inert() -> bool {
        T::inert()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        self.into_inner().parts(out);
    }
}

impl<T: Parts> parts_seal::Sealed for Vec<T> {}
impl<T: Parts> Parts for Vec<T> {
    fn inert() -> bool {
        T::inert()
    }
    /// Each element taken apart in turn (an `Rc` it holds that another
    /// shares let go of, a container in it taken apart too); elements
    /// whose drop runs no app code stay together.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if self.is_empty() {
            return;
        }
        if Self::inert() || !std::mem::needs_drop::<T>() {
            out.push(Box::new(self));
        } else {
            for v in self {
                v.parts(out);
            }
        }
    }
}

impl<T: Parts> parts_seal::Sealed for std::collections::VecDeque<T> {}
impl<T: Parts> Parts for std::collections::VecDeque<T> {
    fn inert() -> bool {
        T::inert()
    }
    /// As a `Vec`'s.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if self.is_empty() {
            return;
        }
        if Self::inert() || !std::mem::needs_drop::<T>() {
            out.push(Box::new(self));
        } else {
            for v in self {
                v.parts(out);
            }
        }
    }
}

impl<K: Parts, V: Parts> parts_seal::Sealed for std::collections::BTreeMap<K, V> {}
impl<K: Parts, V: Parts> Parts for std::collections::BTreeMap<K, V> {
    fn inert() -> bool {
        K::inert() && V::inert()
    }
    /// Whole if inert; else each key, then its value, on its own.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() || !std::mem::needs_drop::<(K, V)>() {
            whole(self, out);
        } else {
            for (k, v) in self {
                k.parts(out);
                v.parts(out);
            }
        }
    }
}

impl<T: Parts> parts_seal::Sealed for Option<T> {}
impl<T: Parts> Parts for Option<T> {
    fn inert() -> bool {
        T::inert()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if let Some(v) = self {
            v.parts(out);
        }
    }
}

impl<A: Parts, B: Parts> parts_seal::Sealed for (A, B) {}
impl<A: Parts, B: Parts> Parts for (A, B) {
    fn inert() -> bool {
        A::inert() && B::inert()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() {
            return whole(self, out);
        }
        self.0.parts(out);
        self.1.parts(out);
    }
}

impl<K: Parts, V: Parts, S: 'static> parts_seal::Sealed for std::collections::HashMap<K, V, S> {}
impl<K: Parts, V: Parts, S: 'static> Parts for std::collections::HashMap<K, V, S> {
    fn inert() -> bool {
        K::inert() && V::inert() && !std::mem::needs_drop::<S>()
    }
    /// Whole, one box, if inert. Else each key and value taken apart in
    /// turn, as a `Vec`'s elements (one destructor per box: Codex r15-5).
    /// The map itself, emptied, goes last, whole: its hasher is a caller's
    /// value too, and nothing is dropped here (Codex r15-1: a drop in the
    /// drain ran before the switch's watch).
    fn parts(mut self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() {
            return whole(self, out);
        }
        if std::mem::needs_drop::<(K, V)>() {
            for (k, v) in self.drain() {
                k.parts(out);
                v.parts(out);
            }
        }
        out.push(Box::new(self));
    }
}

impl<K: Parts, S: 'static> parts_seal::Sealed for std::collections::HashSet<K, S> {}
impl<K: Parts, S: 'static> Parts for std::collections::HashSet<K, S> {
    fn inert() -> bool {
        K::inert() && !std::mem::needs_drop::<S>()
    }
    /// As a map's: whole if inert; else each key, then the set, emptied,
    /// whole.
    fn parts(mut self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() {
            return whole(self, out);
        }
        if std::mem::needs_drop::<K>() {
            for k in self.drain() {
                k.parts(out);
            }
        }
        out.push(Box::new(self));
    }
}

/// A caller's type a `[Fresh]` piece holds in a container (`Vec`, a map,
/// an `Option`), stated as one value to a check's drain: one box, so one
/// destructor under one catch, or nothing if it has no drop glue. What it
/// holds inside is opaque to the drain (a handle in it is not let go of
/// apart); a live obligation it holds ends with its live owner, never with
/// its drop (ADR-0034, S4). The impl states no code: `gui` takes it apart.
///
/// ```
/// struct Touch(u32);
/// impl gui::engine::FreshValue for Touch {}
/// gui::engine_local! {
///     [Fresh] static PENDING: std::cell::RefCell<Vec<Touch>> = std::cell::RefCell::new(Vec::new());
/// }
/// gui::testing::in_check(|| PENDING.with_borrow_mut(|p| p.push(Touch(1))));
/// ```
pub trait FreshValue: 'static {}

impl<T: FreshValue> parts_seal::Sealed for T {}
impl<T: FreshValue> Parts for T {
    /// No drop glue, or a `Box<str>` (it frees its bytes, no more).
    fn inert() -> bool {
        !std::mem::needs_drop::<T>() || std::any::TypeId::of::<T>() == std::any::TypeId::of::<Box<str>>()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if std::mem::needs_drop::<T>() {
            out.push(Box::new(self));
        }
    }
}

/// `gui`'s own types a `[Fresh]` piece holds, each one value to its drain:
/// one box (one destructor under one catch), or nothing if it has no drop
/// glue. For a type whose drop is not one user value's, state its parts.
macro_rules! parts_whole {
    ($($t:ty),* $(,)?) => {
        $(
            impl $crate::engine::parts_seal::Sealed for $t {}
            impl $crate::engine::Parts for $t {
                fn inert() -> bool {
                    !std::mem::needs_drop::<$t>()
                }
                fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
                    if std::mem::needs_drop::<$t>() {
                        out.push(Box::new(self));
                    }
                }
            }
        )*
    };
}
pub(crate) use parts_whole;

parts_whole!(
    crate::component::Priority,
    crate::component::Rendered,
    crate::reactive::Scope,
    crate::layer::LayerHandle,
    crate::mailbox::Address,
);

/// A map of values `gui` does not know (a caller's keys), taken apart one
/// destructor per box: each key, then each value, on its own (ADR-0034,
/// S9; Codex r15-5: a key and its value in one box would run the value's
/// drop in the key's unwind, past the first fault).
pub(crate) fn map_apart<K: 'static, V: 'static, S: 'static>(
    mut map: std::collections::HashMap<K, V, S>,
    out: &mut Vec<Box<dyn Any>>,
) {
    if std::mem::needs_drop::<(K, V)>() {
        for (k, v) in map.drain() {
            if std::mem::needs_drop::<K>() {
                out.push(Box::new(k));
            }
            if std::mem::needs_drop::<V>() {
                out.push(Box::new(v));
            }
        }
    }
    // The map, emptied, whole: its hasher is dropped under the catch too.
    out.push(Box::new(map));
}

impl<T: ?Sized + 'static> parts_seal::Sealed for std::sync::Weak<T> {}
impl<T: ?Sized + 'static> Parts for std::sync::Weak<T> {
    fn inert() -> bool {
        true
    }
    /// A weak handle's drop frees no value: one box.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        out.push(Box::new(self));
    }
}

impl<T: ?Sized + 'static> parts_seal::Sealed for std::rc::Weak<T> {}
impl<T: ?Sized + 'static> Parts for std::rc::Weak<T> {
    fn inert() -> bool {
        true
    }
    /// A weak handle's drop frees no value: one box.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        out.push(Box::new(self));
    }
}

/// Values with no handle in them: one box each, or nothing to drop. Inert:
/// their drop frees memory, no more.
macro_rules! plain_parts {
    ($($t:ty),*) => {
        $(
            impl parts_seal::Sealed for $t {}
            impl Parts for $t {
                fn inert() -> bool {
                    true
                }
                fn parts(self, out: &mut Vec<Box<dyn Any>>) {
                    if std::mem::needs_drop::<$t>() {
                        out.push(Box::new(self));
                    }
                }
            }
        )*
    };
}

plain_parts!(
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    f32,
    f64,
    bool,
    char,
    (),
    String,
    std::any::TypeId
);

impl<A: Parts, B: Parts, C: Parts> parts_seal::Sealed for (A, B, C) {}
impl<A: Parts, B: Parts, C: Parts> Parts for (A, B, C) {
    fn inert() -> bool {
        A::inert() && B::inert() && C::inert()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() {
            return whole(self, out);
        }
        self.0.parts(out);
        self.1.parts(out);
        self.2.parts(out);
    }
}

impl<A: Parts, B: Parts, C: Parts, D: Parts> parts_seal::Sealed for (A, B, C, D) {}
impl<A: Parts, B: Parts, C: Parts, D: Parts> Parts for (A, B, C, D) {
    fn inert() -> bool {
        A::inert() && B::inert() && C::inert() && D::inert()
    }
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if Self::inert() {
            return whole(self, out);
        }
        self.0.parts(out);
        self.1.parts(out);
        self.2.parts(out);
        self.3.parts(out);
    }
}

/// One element: a box, an `Rc` or an `Arc` drops what it points to whole.
macro_rules! whole_parts {
    ($($t:ident),*) => {
        $(
            impl<T: ?Sized + 'static> parts_seal::Sealed for $t<T> {}
            impl<T: ?Sized + 'static> Parts for $t<T> {
                fn parts(self, out: &mut Vec<Box<dyn Any>>) {
                    out.push(Box::new(self));
                }
            }
        )*
    };
}

use std::sync::Arc as ArcPart;
whole_parts!(ArcPart);

/// A box is one value: what it holds is opaque to the drain.
impl<T: ?Sized + 'static> FreshValue for Box<T> {}

/// A shared reference drops nothing.
impl<T: ?Sized + 'static> FreshValue for &'static T {}

impl<T: ?Sized + 'static> parts_seal::Sealed for std::rc::Rc<T> {}
impl<T: ?Sized + 'static> Parts for std::rc::Rc<T> {
    /// One element if it is the last handle; a handle another holds (the
    /// live world, often) is let go of at once: its drop only counts down
    /// and runs no code. Handed back, a runaway would set it aside, and it
    /// would pin what it shares with live for good (a memo that never
    /// folds again, `holder`).
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        if std::rc::Rc::strong_count(&self) > 1 {
            drop(self);
        } else {
            out.push(Box::new(self));
        }
    }
}

impl parts_seal::Sealed for crate::text::TextSystem {}
impl Parts for crate::text::TextSystem {
    /// The engine's own caches: no user value in them.
    fn parts(self, out: &mut Vec<Box<dyn Any>>) {
        out.push(Box::new(self));
    }
}

/// A fork's writes to a [`ForkMap`], by key: `Some` written, `None`
/// removed. The live map under it is never touched while a fork runs.
/// A value the fork displaced from its own overlay (written over or
/// removed again) waits in `displaced` for the fork's drain: never dropped
/// inside the caller's borrow (ADR-0034, S7).
#[cfg(debug_assertions)]
struct Overlay<K, V> {
    map: foldhash::HashMap<K, Option<V>>,
    displaced: Vec<V>,
}

#[cfg(debug_assertions)]
impl<K, V> Default for Overlay<K, V> {
    fn default() -> Self {
        Self {
            map: foldhash::HashMap::default(),
            displaced: Vec::new(),
        }
    }
}

/// A key of a [`ForkMap`] (ADR-0034, S7): `Copy`, compared and hashed by
/// `gui`'s own rules, so no user code runs as a fork looks one up, and two
/// keys that compare equal are the same key (a fork can never keep one
/// where live keeps another). Sealed: the integers, `char`, `bool` and
/// `TypeId`.
///
/// ```compile_fail
/// // A key of the caller's own: its `Eq` and `Hash` are user code.
/// #[derive(Clone, Copy, PartialEq, Eq, Hash)]
/// struct Mine(u64);
/// let _ = gui::engine::ForkMap::<Mine, u8>::default();
/// ```
///
/// ```
/// let _ = gui::engine::ForkMap::<u64, u8>::default();
/// let _ = gui::engine::ForkMap::<std::any::TypeId, u8>::default();
/// ```
pub trait ForkKey: Copy + Eq + std::hash::Hash + sealed::Sealed + 'static {}

// Their `sealed::Sealed` is data's ([`ForkData`]).
macro_rules! fork_keys {
    ($($t:ty),*) => {
        $(
            impl ForkKey for $t {}
        )*
    };
}

fork_keys!(
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    char,
    bool,
    std::any::TypeId
);

/// A map a cold frame forks by an overlay of its writes (F1 review 13):
/// while a fork runs, every write goes to the overlay and every read looks
/// there first, so the live map is never written and the fork costs the
/// keys written ([`fork_work`]). The overlay is moved out whole by the
/// fork's drain and dropped in its drain loop, and the rollback is moves
/// alone: no key is hashed, compared, cloned or dropped once the fork has
/// closed (a key's `Hash`, `Eq`, `Clone` and `Drop` are user code). As a
/// `ForkSlab`'s shadow copies are, and Compose's state records made on a
/// snapshot's first write.
pub struct ForkMap<K: ForkKey, V: ForkValue> {
    map: foldhash::HashMap<K, V>,
    #[cfg(debug_assertions)]
    fork: Option<Overlay<K, V>>,
}

impl<K: ForkKey, V: ForkValue> Default for ForkMap<K, V> {
    fn default() -> Self {
        Self {
            map: foldhash::HashMap::default(),
            #[cfg(debug_assertions)]
            fork: None,
        }
    }
}

impl<K: ForkKey, V: ForkValue> ForkMap<K, V> {
    /// The running fork's writes, if one runs.
    #[cfg(debug_assertions)]
    fn overlay(&self) -> Option<&foldhash::HashMap<K, Option<V>>> {
        self.fork.as_ref().map(|o| &o.map)
    }

    #[cfg(not(debug_assertions))]
    #[expect(clippy::unused_self, reason = "the release half of a debug-only overlay")]
    const fn overlay(&self) -> Option<&foldhash::HashMap<K, Option<V>>> {
        None
    }

    #[must_use]
    pub fn get(&self, k: &K) -> Option<&V> {
        match self.overlay().and_then(|o| o.get(k)) {
            Some(written) => written.as_ref(),
            None => self.map.get(k),
        }
    }

    #[must_use]
    pub fn contains_key(&self, k: &K) -> bool {
        self.get(k).is_some()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        let Some(o) = self.overlay() else {
            return self.map.len();
        };
        o.iter().fold(self.map.len(), |n, (k, v)| {
            match (self.map.contains_key(k), v.is_some()) {
                (false, true) => n.saturating_add(1),
                (true, false) => n.saturating_sub(1),
                _ => n,
            }
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        let overlay = self.overlay();
        self.map
            .iter()
            .filter(move |(k, _)| overlay.is_none_or(|o| !o.contains_key(*k)))
            .chain(
                overlay
                    .into_iter()
                    .flat_map(|o| o.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k, v)))),
            )
    }

    /// `k` set to `v`, by replacement only: what it held is never handed
    /// back, so a check copies nothing (S7, a value with no
    /// [`ForkClone`]). Live, the old value drops here; in a check, an old
    /// value of the check's own waits for its drain.
    pub fn replace(&mut self, k: K, v: V) {
        #[cfg(debug_assertions)]
        if self.fork.is_some() {
            self.written(k, Some(v));
            return;
        }
        drop(self.map.insert(k, v));
    }

    /// `k` removed, by tombstone: as [`Self::replace`], nothing handed back.
    pub fn forget(&mut self, k: &K) {
        #[cfg(debug_assertions)]
        if self.fork.is_some() {
            self.written(*k, None);
            return;
        }
        drop(self.map.remove(k));
    }

    /// In a fork, `k` written as `v` in its overlay; a value of the fork's
    /// it displaces waits for the drain, never dropped here (S7).
    #[cfg(debug_assertions)]
    fn written(&mut self, k: K, v: Option<V>) {
        if let Some(o) = self.fork.as_mut() {
            match o.map.insert(k, v) {
                None => fork_work(1),
                Some(Some(was)) => o.displaced.push(was),
                Some(None) => {}
            }
        }
    }
}

impl<K: ForkKey, V: ForkClone> ForkMap<K, V> {
    /// `k` set to `v`; what it held before. In a fork, a value the fork
    /// wrote is moved out of its overlay; a live one is copied
    /// ([`ForkClone`], its stated cost), as the live map is never written.
    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        #[cfg(debug_assertions)]
        if let Some(o) = self.fork.as_mut() {
            return match o.map.insert(k, Some(v)) {
                Some(was) => was,
                None => {
                    fork_work(1);
                    self.map.get(&k).map(ForkClone::fork_clone)
                }
            };
        }
        self.map.insert(k, v)
    }

    /// `k` removed; what it held, as [`Self::insert`] hands it back.
    pub fn remove(&mut self, k: &K) -> Option<V> {
        #[cfg(debug_assertions)]
        if let Some(o) = self.fork.as_mut() {
            return match o.map.insert(*k, None) {
                Some(was) => was,
                None => {
                    fork_work(1);
                    self.map.get(k).map(ForkClone::fork_clone)
                }
            };
        }
        self.map.remove(k)
    }

    /// `k`'s value to write in place, made default if it has none. In a
    /// cold frame, the fork's copy: the overlay is looked at first, so the
    /// live value is copied once per key however often it is written, on
    /// the first write and not before (ADR-0034, S7; Codex r14-7).
    pub fn entry_mut(&mut self, k: K) -> &mut V
    where
        V: Default,
    {
        #[cfg(debug_assertions)]
        if let Some(o) = self.fork.as_mut() {
            let slot = match o.map.entry(k) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    fork_work(1);
                    e.insert(self.map.get(&k).map(ForkClone::fork_clone))
                }
            };
            return slot.get_or_insert_with(V::default);
        }
        self.map.entry(k).or_default()
    }
}

impl<K: ForkKey, V: ForkValue> sealed::Sealed for ForkMap<K, V> {}

impl<K: ForkKey, V: ForkValue> Logged for ForkMap<K, V> {
    fn begin(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.fork = Some(Overlay::default());
        }
    }

    /// The fork's writes, moved out to drop in its drain loop, each key and
    /// each value in a box of its own (S9): one whose drop panics is a
    /// fault, and the others are still dropped, each under its own catch,
    /// never in that unwind (S2). A write after
    /// this (by a later drop) goes to the overlay again, which the next
    /// round moves out.
    fn drain(&mut self) -> Vec<Box<dyn Any>> {
        #[cfg(debug_assertions)]
        if let Some(o) = self.fork.as_mut() {
            let Overlay { map, displaced } = std::mem::take(o);
            let mut made: Vec<Box<dyn Any>> = Vec::with_capacity(map.len().saturating_add(displaced.len()));
            // Keys are `Copy` (ForkKey): no drop of theirs to isolate.
            // Each value taken apart (S9, Codex r17 1): never a container
            // in one box.
            for v in map.into_values().flatten() {
                v.split(&mut made);
            }
            for v in displaced {
                v.split(&mut made);
            }
            return made;
        }
        Vec::new()
    }

    /// The fork closed: the live map was never written, so nothing is put
    /// back, and the overlay is gone (empty: the drain loop took it).
    ///
    /// A map with no overlay is not the one the check began: it was put in
    /// its place inside `gui` ([`LogMut::engine_mut`]; no key outside lets
    /// it be moved, S7), so the check faults (S2).
    fn rollback(&mut self) {
        #[cfg(debug_assertions)]
        match self.fork.take() {
            None => quarantined("a ForkMap replaced in its check", Box::new(())),
            Some(o) if !(o.map.is_empty() && o.displaced.is_empty()) => {
                quarantined("a ForkMap's overlay", Box::new(o));
            }
            Some(_) => {}
        }
    }
}

/// Which entry of a [`ForkSlab`] a handle names: its slot, and the
/// generation the slot was at when the entry was made, so a handle whose
/// entry was removed finds nothing, even once its slot holds another.
///
/// It names an entry of this thread's slab, so it is neither `Send` nor
/// `Sync`: moved to another thread it would name that thread's entry. The
/// handles made of it are bound the same way:
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::ListState>();
/// ```
///
/// ```compile_fail
/// fn sync<T: Sync>() {}
/// sync::<gui::ScrollHandle>();
/// ```
///
/// ```compile_fail
/// fn send<T: Send>() {}
/// send::<gui::list::Columns>();
/// ```
///
/// ```
/// fn copy<T: Copy>() {}
/// copy::<gui::ListState>();
/// copy::<gui::ScrollHandle>();
/// copy::<gui::list::Columns>();
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SlabId {
    slot: u32,
    generation: u32,
    /// The world that made it (ADR-0034, S6): 0, live; else the serial of
    /// the check that made it, which no later world has ([`world`]).
    world: u64,
    thread: std::marker::PhantomData<*const ()>,
}

/// The top bit of a generation: set on every entry a fork made, never on
/// one live code made ([`Generations`]).
pub(crate) const FORK_GENERATION: u32 = 1 << 31;

/// A generation counter that ran out: its slot is retired.
const SPENT: u32 = FORK_GENERATION - 1;

/// The one slot rule of every arena of handles (ADR-0028, rule 3): the
/// reactive nodes, the component instances, every [`ForkSlab`] (a list's
/// state, a scroll handle's, a `Keyed` memo). Each slot has two next
/// generations, one for what live code makes and one, marked by the top
/// bit, for what forks make. Each only grows. None is logged, put back by
/// a rollback, or cut off with the slots a rollback takes away: an
/// identity source is never transactional state (PostgreSQL's `nextval`,
/// ARIES's log sequence numbers), and the two spaces are the id blocks'
/// two (`IdLeases`). So a handle a fork handed out names nothing after it,
/// live or in another fork, and live code hands out the very ids it would
/// have without the fork. Which space an entry takes is one answer,
/// [`forking`] at the moment it is made. A slot whose generations ran out
/// in either space is retired.
#[derive(Default)]
pub(crate) struct Generations {
    next: Vec<(u32, u32)>,
}

impl Generations {
    /// Slot `slot`'s next generation, taken from the forks' space if
    /// `fork`; `None` once that space ran out (the slot is retired).
    pub(crate) fn take(&mut self, slot: usize, fork: bool) -> Option<u32> {
        if self.next.len() <= slot {
            self.next.resize(slot.saturating_add(1), (0, 0));
        }
        let (live, forks) = self.next.get_mut(slot)?;
        let next = if fork { forks } else { live };
        if *next >= SPENT {
            return None;
        }
        let generation = *next;
        *next = generation.saturating_add(1);
        Some(if fork { generation | FORK_GENERATION } else { generation })
    }

    /// Whether slot `slot` may hold another entry: neither space ran out.
    pub(crate) fn reusable(&self, slot: usize) -> bool {
        self.next.get(slot).is_none_or(|(l, f)| *l < SPENT && *f < SPENT)
    }

    /// The bytes it holds.
    pub(crate) fn held_bytes(&self) -> u64 {
        u64::try_from(self.next.capacity().saturating_mul(std::mem::size_of::<(u32, u32)>())).unwrap_or(u64::MAX)
    }

    /// Test: slot `slot`'s next live and fork generations.
    #[cfg(test)]
    pub(crate) fn set(&mut self, slot: usize, live: u32, fork: u32) {
        if self.next.len() <= slot {
            self.next.resize(slot.saturating_add(1), (0, 0));
        }
        if let Some(n) = self.next.get_mut(slot) {
            *n = (live, fork);
        }
    }
}

/// One slot of a [`ForkSlab`]: its entry while it holds one, in a cell of
/// its own (an entry is written while others are read), and the generation
/// it was made at ([`Generations`] keeps the next ones).
struct SlabSlot<T> {
    generation: u32,
    value: Option<RefCell<T>>,
}

/// The top bit of a slot number: an entry in a fork's own region.
const FORK_SLOT: u32 = 1 << 31;

/// A live entry as a fork sees it once it wrote or removed it: a copy of
/// its own (`None`: removed in the fork), shared out so it is written with
/// nothing else borrowed.
#[cfg(debug_assertions)]
type Shadow<T> = std::rc::Rc<RefCell<Option<T>>>;

/// What a fork does to a [`ForkSlab`], beside the live entries, which it
/// never touches: its copies of the live entries it wrote or removed, by
/// slot and generation, and the entries it made, in a region of its own.
/// So the live region (its entries, its free slots, its numbering) is as
/// without the fork.
#[cfg(debug_assertions)]
#[derive(Default)]
struct ForkPart<T> {
    open: bool,
    shadow: RefCell<foldhash::HashMap<(u32, u32), Shadow<T>>>,
    /// The fork's own entries, numbered `FORK_SLOT | index` in the world
    /// that made them ([`SlabId::world`]): never reused within it, so an
    /// index and a world name one entry, and a handle one check made names
    /// nothing in another or live (ADR-0034, S6).
    slots: Vec<SlabSlot<T>>,
    /// The world `slots` belong to.
    world: u64,
    /// What the fork let go of by [`ForkSlab::forget`]: never dropped in
    /// the caller's borrow, but by the drain (S7).
    displaced: Vec<T>,
}

/// The state of `Copy` handles (rule 4 of ADR-0028: a handle is an id; its
/// state is registered), as a `[Fork(log)]` piece. A fork never writes a
/// live entry: its first write of one makes a copy of its own, which it
/// reads and writes from then on ([`ForkPart`]), so a fork costs the
/// handles it wrote, not those the thread holds, and the rollback is
/// dropping what the fork has. An entry is cloned whole once per fork: a
/// handle's state holds no collection that grows with the data but behind
/// a persistent structure, whose clone is O(1) and whose writes copy only
/// what they change (a list's heights, [`crate::sum_tree`]; a memo's map).
pub struct ForkSlab<T: ForkValue> {
    slots: Vec<SlabSlot<T>>,
    free: Vec<u32>,
    /// The live region's next generations (live code alone makes entries
    /// there).
    generations: Generations,
    #[cfg(debug_assertions)]
    fork: ForkPart<T>,
}

impl<T: ForkValue> Default for ForkSlab<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            generations: Generations::default(),
            #[cfg(debug_assertions)]
            fork: ForkPart {
                open: false,
                shadow: RefCell::default(),
                slots: Vec::new(),
                world: 0,
                displaced: Vec::new(),
            },
        }
    }
}

/// Where an entry is, as the running code sees it.
enum At<'a, T> {
    Cell(&'a RefCell<T>),
    #[cfg(debug_assertions)]
    Shadow(Shadow<T>),
}

impl<T: ForkValue> ForkSlab<T> {
    /// Whether a fork is open: what it does goes to its own part, whether
    /// or not the fork entered this slab (ADR-0034, S6: allocation is
    /// routed by the active world before any slot is touched). Debug only,
    /// as forks are until F2 (ADR-0028).
    #[cfg(debug_assertions)]
    #[expect(clippy::unused_self, reason = "the release half reads the slab")]
    fn forks(&self) -> bool {
        crate::reactive::is_cold()
    }

    /// The fork region's slots, if `id` is of the running world: an id of a
    /// world that ended (or of a check, read live) names nothing.
    #[cfg(debug_assertions)]
    fn fork_slots(&self, id: SlabId) -> Option<&[SlabSlot<T>]> {
        (id.world != 0 && id.world == world() && self.fork.world == id.world).then_some(&self.fork.slots[..])
    }

    /// Entry `id`'s slot in a region (the live one, or the fork's), if it
    /// is held under that generation.
    fn held(slots: &[SlabSlot<T>], index: u32, generation: u32) -> Option<&RefCell<T>> {
        let s = slots.get(usize::try_from(index).ok()?)?;
        if s.generation == generation {
            s.value.as_ref()
        } else {
            None
        }
    }

    /// Entry `id` as the running code sees it: the fork's own, or its copy
    /// of a live one, while it runs; else the live one.
    fn at(&self, id: SlabId) -> Option<At<'_, T>> {
        if id.slot & FORK_SLOT != 0 {
            #[cfg(debug_assertions)]
            return Self::held(self.fork_slots(id)?, id.slot & !FORK_SLOT, id.generation).map(At::Cell);
            #[cfg(not(debug_assertions))]
            return None;
        }
        #[cfg(debug_assertions)]
        if self.forks()
            && let Some(s) = self.fork.shadow.borrow().get(&(id.slot, id.generation))
        {
            return Some(At::Shadow(std::rc::Rc::clone(s)));
        }
        Self::held(&self.slots, id.slot, id.generation).map(At::Cell)
    }

    /// In a fork, its copy of live entry `id`, made on its first write by
    /// `copy` (`None`: the entry is gone; `copy` gives `None` for a
    /// tombstone, which copies nothing).
    #[cfg(debug_assertions)]
    fn shadow_by(&self, id: SlabId, copy: impl FnOnce(&T) -> Option<T>) -> Option<Shadow<T>> {
        if let Some(s) = self.fork.shadow.borrow().get(&(id.slot, id.generation)) {
            return Some(std::rc::Rc::clone(s));
        }
        let copy = copy(&Self::held(&self.slots, id.slot, id.generation)?.borrow());
        let s: Shadow<T> = std::rc::Rc::new(RefCell::new(copy));
        self.fork
            .shadow
            .borrow_mut()
            .insert((id.slot, id.generation), std::rc::Rc::clone(&s));
        fork_work(1);
        Some(s)
    }

    /// A new entry holding `value`: in the fork's own region while a fork
    /// runs ([`forking`]), else live, as without the fork.
    pub fn insert(&mut self, value: T) -> SlabId {
        #[cfg(debug_assertions)]
        if self.forks() {
            let world = world();
            let part = &mut self.fork;
            if part.world != world {
                // A region of an earlier world is emptied by that world's
                // rollback, inside it: one left would be dropped here, in
                // another world.
                assert!(
                    part.slots.is_empty(),
                    "a slab's fork region outlived its check: a bug in gui"
                );
                part.world = world;
            }
            let index = part.slots.len();
            let Ok(slot) = u32::try_from(index) else {
                exhausted_slab();
            };
            if slot & FORK_SLOT != 0 {
                exhausted_slab();
            }
            fork_work(1);
            part.slots.push(SlabSlot {
                generation: FORK_GENERATION,
                value: Some(RefCell::new(value)),
            });
            return SlabId {
                slot: slot | FORK_SLOT,
                generation: FORK_GENERATION,
                world,
                thread: std::marker::PhantomData,
            };
        }
        while let Some(slot) = self.free.pop() {
            // A slot whose generations ran out is retired: let go here,
            // never handed out again.
            let Ok(i) = usize::try_from(slot) else { continue };
            let Some(generation) = self.generations.take(i, false) else {
                continue;
            };
            if let Some(s) = self.slots.get_mut(i) {
                s.generation = generation;
                s.value = Some(RefCell::new(value));
                return SlabId {
                    slot,
                    generation,
                    world: 0,
                    thread: std::marker::PhantomData,
                };
            }
            exhausted_slab();
        }
        self.push(value)
    }

    /// `value` in a live slot added at the end.
    fn push(&mut self, value: T) -> SlabId {
        loop {
            let Ok(slot) = u32::try_from(self.slots.len()) else {
                exhausted_slab();
            };
            if slot & FORK_SLOT != 0 {
                exhausted_slab();
            }
            match self.generations.take(self.slots.len(), false) {
                Some(generation) => {
                    self.slots.push(SlabSlot {
                        generation,
                        value: Some(RefCell::new(value)),
                    });
                    return SlabId {
                        slot,
                        generation,
                        world: 0,
                        thread: std::marker::PhantomData,
                    };
                }
                None => self.slots.push(SlabSlot {
                    generation: SPENT,
                    value: None,
                }),
            }
        }
    }

    /// Take entry `id` out (`None`: it was gone already); its slot is
    /// reused under a generation not handed out before, or retired once
    /// its generations ran out. In a check, a live entry is not the
    /// check's to hand out: it is hidden by a tombstone (nothing copied)
    /// and `None` is given ([`Self::remove`] hands back a copy).
    pub fn forget(&mut self, id: SlabId) {
        #[cfg(debug_assertions)]
        if id.slot & FORK_SLOT == 0 && self.forks() {
            if let Some(s) = self.shadow_by(id, |_| None) {
                // A copy the check made earlier is its own: to the drain.
                let was = s.borrow_mut().take();
                if let Some(was) = was {
                    self.fork.displaced.push(was);
                }
            }
            return;
        }
        if let Some(was) = self.take_out(id) {
            #[cfg(debug_assertions)]
            if id.slot & FORK_SLOT != 0 {
                self.fork.displaced.push(was);
                return;
            }
            drop(was);
        }
    }

    /// Entry `id` taken out of its region: the fork's own, or live (not in
    /// a check).
    fn take_out(&mut self, id: SlabId) -> Option<T> {
        if id.slot & FORK_SLOT != 0 {
            #[cfg(debug_assertions)]
            {
                Self::held(self.fork_slots(id)?, id.slot & !FORK_SLOT, id.generation)?;
                let s = self.fork.slots.get_mut(usize::try_from(id.slot & !FORK_SLOT).ok()?)?;
                return s.value.take().map(RefCell::into_inner);
            }
            #[cfg(not(debug_assertions))]
            return None;
        }
        Self::held(&self.slots, id.slot, id.generation)?;
        let i = usize::try_from(id.slot).ok()?;
        let s = self.slots.get_mut(i)?;
        let value = s.value.take()?.into_inner();
        if self.generations.reusable(i) {
            self.free.push(id.slot);
        }
        Some(value)
    }

    /// How many entries it holds, as the running code sees it (tests): in a
    /// fork, the live entries it did not remove and its own.
    #[must_use]
    pub fn len(&self) -> usize {
        let live = self.slots.iter().filter(|s| s.value.is_some()).count();
        #[cfg(debug_assertions)]
        if self.forks() {
            let removed = self
                .fork
                .shadow
                .borrow()
                .values()
                .filter(|s| s.borrow().is_none())
                .count();
            let own = if self.fork.world == world() {
                self.fork.slots.iter().filter(|s| s.value.is_some()).count()
            } else {
                0
            };
            return live.saturating_sub(removed).saturating_add(own);
        }
        live
    }

    /// Whether it holds no entry.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether entry `id` is held.
    #[must_use]
    pub fn contains(&self, id: SlabId) -> bool {
        match self.at(id) {
            Some(At::Cell(_)) => true,
            #[cfg(debug_assertions)]
            Some(At::Shadow(s)) => s.borrow().is_some(),
            None => false,
        }
    }

    /// Read entry `id`; `None` once it is gone. `f` must not reach the
    /// same entry again (it is borrowed while `f` runs).
    pub fn read<R>(&self, id: SlabId, f: impl FnOnce(&T) -> R) -> Option<R> {
        match self.at(id)? {
            At::Cell(v) => Some(f(&v.borrow())),
            #[cfg(debug_assertions)]
            At::Shadow(s) => s.borrow().as_ref().map(f),
        }
    }

    /// Test: retire slot `id`'s generations as if they ran out.
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn spend(&mut self, id: SlabId) {
        self.set_next(id, SPENT, SPENT);
    }

    /// Test: slot `id`'s next live and fork generations.
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn set_next(&mut self, id: SlabId, live: u32, fork: u32) {
        if let Ok(i) = usize::try_from(id.slot) {
            self.generations.set(i, live, fork);
        }
    }
}

impl<T: ForkClone> ForkSlab<T> {
    /// Write entry `id`: in a fork, its copy of a live entry, made on its
    /// first write ([`ForkClone`]: once per entry, not before); `None` (and
    /// nothing written) once it is gone. `f` must not reach the same entry
    /// again.
    pub fn write<R>(&self, id: SlabId, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        #[cfg(debug_assertions)]
        if id.slot & FORK_SLOT == 0 && self.forks() {
            return self
                .shadow_by(id, |v| Some(v.fork_clone()))?
                .borrow_mut()
                .as_mut()
                .map(f);
        }
        match self.at(id)? {
            At::Cell(v) => Some(f(&mut v.borrow_mut())),
            #[cfg(debug_assertions)]
            At::Shadow(s) => s.borrow_mut().as_mut().map(f),
        }
    }

    /// Take entry `id` out, as [`Self::forget`], and hand it back: in a
    /// check, a live entry's copy ([`ForkClone`]), the entry hidden.
    pub fn remove(&mut self, id: SlabId) -> Option<T> {
        #[cfg(debug_assertions)]
        if id.slot & FORK_SLOT == 0 && self.forks() {
            return self.shadow_by(id, |v| Some(v.fork_clone()))?.borrow_mut().take();
        }
        self.take_out(id)
    }
}

impl<T: ForkValue> sealed::Sealed for ForkSlab<T> {}

impl<T: ForkValue> Logged for ForkSlab<T> {
    fn begin(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.fork.open = true;
        }
    }

    fn drain(&mut self) -> Vec<Box<dyn Any>> {
        #[cfg(debug_assertions)]
        {
            self.take_fork()
        }
        #[cfg(not(debug_assertions))]
        Vec::new()
    }

    fn rollback(&mut self) {
        // Nothing live to put back: the fork wrote none of it. What it had
        // was drained; its ids name nothing from now on (another world,
        // [`SlabId::world`]).
        // A slab not open is not the one the check began: put in its
        // place inside `gui` ([`LogMut::engine_mut`]), a fault (S2).
        #[cfg(debug_assertions)]
        {
            if !std::mem::replace(&mut self.fork.open, false) {
                quarantined("a ForkSlab replaced in its check", Box::new(()));
            }
            let left = self.take_fork();
            if !left.is_empty() {
                quarantined("a ForkSlab's fork region", Box::new(left));
            }
            self.fork.slots.clear();
        }
    }
}

impl<T: ForkValue> ForkSlab<T> {
    /// What the fork has (its copies, its own entries, what it let go of),
    /// taken out, one box per entry (S9).
    #[cfg(debug_assertions)]
    fn take_fork(&mut self) -> Vec<Box<dyn Any>> {
        let shadow = std::mem::take(self.fork.shadow.get_mut());
        let mut made: Vec<Box<dyn Any>> = Vec::with_capacity(shadow.len());
        // Each value taken apart (S9, Codex r17 1): never a container in
        // one box.
        let mut put = |v: T| v.split(&mut made);
        for v in std::mem::take(&mut self.fork.displaced) {
            put(v);
        }
        for s in shadow.into_values() {
            if let Some(v) = s.borrow_mut().take() {
                put(v);
            }
        }
        for s in &mut self.fork.slots {
            if let Some(v) = s.value.take() {
                put(v.into_inner());
            }
        }
        made
    }
}

/// A handle's entry made now lives as long as the running scope: `dispose`
/// (which takes it out of its slab) runs as that scope ends. Made outside
/// any scope, it lives until disposed by hand, as a signal does.
pub(crate) fn owned_by_scope(dispose: impl FnOnce() + 'static) {
    // Making a handle: the engine's own bookkeeping, which a Shared init
    // may reach; nothing read is given back ([`allocating`]).
    allocating(|| {
        if crate::reactive::has_owner() {
            crate::reactive::on_release(dispose);
        }
    });
}

/// A live obligation made now (a root's tree, a list's views and their
/// worker, a mailbox) ends with its live owner (ADR-0034, S4; review r17,
/// decision 5): the scope running, else the window whose code runs, as
/// it ends ([`crate::focus::WindowOwned::end_with_window`]), never later
/// than its last handle. The one registration of every such constructor:
/// each acts on its answer by being refused here. In a check, the owner is
/// the check's (its scope, its copy of the window's work), gone with it.
///
/// # Panics
/// In a debug build, with neither a scope running nor a window's code (a
/// test's top level): nothing would end it but its last handle, which a
/// value a runaway check set aside can pin. Make it in a `Scope`, whose
/// `dispose` ends it, or in a window. In release it is left to its last
/// handle.
#[track_caller]
pub(crate) fn own(what: &str, owned: &std::rc::Rc<dyn crate::focus::WindowOwned>) {
    let weak = std::rc::Rc::downgrade(owned);
    match owner(what) {
        Owner::Scope => allocating(|| {
            crate::reactive::on_release(move || {
                if let Some(owned) = weak.upgrade() {
                    owned.end_with_window();
                }
            });
        }),
        Owner::Window => {
            let taken = crate::focus::owned_by_window(weak);
            debug_assert!(taken, "the window running refused {what}: a bug in gui");
        }
        Owner::Nothing => {}
    }
}

/// Who ends a live obligation made now ([`owner`]).
pub(crate) enum Owner {
    /// The scope running: as it is disposed.
    Scope,
    /// The window whose code runs: as it closes.
    Window,
    /// Neither (a release build: refused in a debug one).
    Nothing,
}

/// Who ends a live obligation made now, `what` ([`own`]): the scope
/// running, else the window whose code runs. A constructor whose window
/// ownership is its own (a mailbox, kept by the window it was made in)
/// asks this and acts on the answer.
///
/// # Panics
/// In a debug build, with neither (see [`own`]).
#[track_caller]
pub(crate) fn owner(what: &str) -> Owner {
    if crate::reactive::has_owner() {
        return Owner::Scope;
    }
    if crate::focus::entered_window().is_some() {
        return Owner::Window;
    }
    #[cfg(debug_assertions)]
    no_owner(what);
    #[cfg(not(debug_assertions))]
    let _ = what;
    Owner::Nothing
}

/// Run `f`, which makes a live obligation (a mailbox, a list's views) at a
/// test's top level, in a scope of the test's own, never disposed: its end
/// is then its last handle's, as a test that drops it means. Where a scope
/// or a window's code runs already, `f` runs as it is, owned by that.
#[cfg(test)]
pub(crate) fn owned<R>(f: impl FnOnce() -> R) -> R {
    if crate::reactive::has_owner() || crate::focus::entered_window().is_some() {
        f()
    } else {
        crate::reactive::Scope::detached().run(f)
    }
}

/// A live obligation made with no owner to end it ([`own`]).
#[cfg(debug_assertions)]
#[track_caller]
#[expect(clippy::panic, reason = "an obligation no owner ends is a bug in its caller")]
fn no_owner(what: &str) {
    panic!(
        "{what} made with no scope running and no window's code: nothing would end it but its last handle. \
         Make it in a `Scope` (whose `dispose` ends it) or in a window"
    );
}

/// A handle used through its owner-facing API after it was disposed (its
/// owner scope ended, or `dispose` ran): in a debug build a loud bug, as a
/// disposed signal read is; in release it stays inert (reads give the empty
/// answer, writes do nothing). The engine's own paths, which may touch a
/// handle once more in a frame being torn down, read it quietly.
#[cfg(debug_assertions)]
#[track_caller]
#[expect(
    clippy::panic,
    reason = "a handle used after its owner ended is a bug, reported as a signal's is"
)]
pub(crate) fn check_alive(alive: bool, what: &str) {
    if !alive {
        panic!(
            "this {what} was disposed (its owner scope ended, or `dispose` ran) and is used after: \
             make it in the scope of everything that uses it, or outside any scope and dispose it by hand"
        );
    }
}

/// More handles alive at once than a slab numbers (2^32): a bug.
#[expect(clippy::panic, reason = "2^32 handles alive at once is a leak in gui")]
fn exhausted_slab() -> ! {
    panic!("gui: more than 2^32 handles of one kind alive at once")
}

/// Where ids are leased from, in blocks: two spaces of block numbers, the
/// live one and one only forks lease from, so a fork never moves what the
/// live frames lease next (ADR-0028, rule 3). Ids stay below 2^63
/// (`ChunkId` tags the top bit): with blocks of 2^16 there are 2^47 block
/// numbers, the lower half live (from 1: no id is 0), the upper half the
/// forks'. Neither counter goes back, so no id is handed out twice; a fork's
/// ids are never a live one's.
pub struct IdLeases {
    block: u64,
    live: std::sync::atomic::AtomicU64,
    forks: std::sync::atomic::AtomicU64,
}

impl IdLeases {
    /// Block numbers there are; the forks' space starts half way.
    const BLOCKS: u64 = 1 << (63 - 16);

    /// Leases of blocks of `block` ids (a power of two up to 2^16).
    #[must_use]
    pub const fn new(block: u64) -> Self {
        Self {
            block,
            live: std::sync::atomic::AtomicU64::new(1),
            forks: std::sync::atomic::AtomicU64::new(Self::BLOCKS / 2),
        }
    }

    /// The first id of a block not leased before: from the forks' space
    /// when `forking`.
    fn lease(&self, forking: bool) -> u64 {
        use std::sync::atomic::Ordering::Relaxed;
        let (space, end) = if forking {
            (&self.forks, Self::BLOCKS)
        } else {
            (&self.live, Self::BLOCKS / 2)
        };
        let leased = space.fetch_add(1, Relaxed);
        if leased >= end {
            exhausted_ids();
        }
        leased.saturating_mul(self.block)
    }

    /// The id after `last` (0: none yet), leasing a block when `last`'s
    /// ran out.
    ///
    /// In a fork the id is always the forks': a counter still in a live
    /// block (the fork began mid-block) leases a fork block first, so the
    /// fork never hands out an id of the live block, which the live counter,
    /// put back as the fork rolls back, hands out after it.
    fn next(&self, last: u64, forking: bool) -> u64 {
        let id = last.saturating_add(1);
        let live = last < self.forks_from();
        if last != 0 && !id.is_multiple_of(self.block) && !(forking && live) {
            return id;
        }
        self.lease(forking)
    }

    /// The first id of the forks' space.
    const fn forks_from(&self) -> u64 {
        (Self::BLOCKS / 2).saturating_mul(self.block)
    }
}

/// The process's id leases, blocks of 2^16: one atomic add per 2^16 ids a
/// thread hands out.
static ID_BLOCKS_LEASED: IdLeases = IdLeases::new(1 << 16);

/// A kind of id [`unique_id`] hands out: a static descriptor, declared
/// once beside what the ids name. It holds no state: the cursors are
/// [`unique_id`]'s own.
pub struct IdKind {
    name: &'static str,
    /// Where it is declared: its key among the cursors. Two kinds of equal
    /// data are two kinds (Opus, F1 review 4: a static's address is not a
    /// key the compiler promises to keep apart).
    site: &'static std::panic::Location<'static>,
}

impl IdKind {
    #[must_use]
    #[track_caller]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            site: std::panic::Location::caller(),
        }
    }

    /// What the ids name (diagnostics).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// One kind's cursors on one thread, for one lease allocator: the last id
/// handed out live and the last a fork handed out (0: none yet).
#[derive(Clone, Copy, Default)]
struct IdCursor {
    live: u64,
    fork: u64,
}

engine_local! {
    /// Every kind's id cursors on this thread, by the kind's declaration
    /// site and the allocator's address. Not forked state: an identity
    /// source is never rolled back, as a database keeps its sequences out
    /// of the undo log (PostgreSQL's `nextval`, ARIES's log sequence
    /// numbers), so no rollback, nor a value made live while a check runs,
    /// ever has an id handed out again.
    [Shared("the id cursors: identity is never rolled back (ADR-0028, rule 3); a cursor only moves on, and a fork's is its own")] static ID_CURSORS: RefCell<foldhash::HashMap<CursorKey, IdCursor>> = RefCell::default();
}

/// A kind's place among the cursors: where it is declared (file, line,
/// column), and the allocator it leases from.
type CursorKey = (&'static str, u32, u32, usize);

/// An id no other has had in this process, of kind `kind`: the next in the
/// block of ids this thread leased for the kind, live or, in a fork, from
/// the forks' space. The cursors are kept here, outside the fork registry:
/// they start at none (the first id leases a block, so two threads' first
/// ids differ) and nothing puts them back. An id costs one map lookup and,
/// once per block, one atomic add. Nothing runs out: a thread that ends
/// takes its unused ids with it, and there are 2^46 blocks of each space.
///
/// Declare each kind once, beside what it names:
///
/// ```ignore
/// static TIMERS: IdKind = IdKind::new("timer");
/// let id = unique_id(&TIMERS);
/// ```
pub fn unique_id(kind: &'static IdKind) -> u64 {
    unique_id_in(kind, &ID_BLOCKS_LEASED)
}

/// [`unique_id`] leasing from `leases` (tests: an allocator of their own).
pub(crate) fn unique_id_in(kind: &'static IdKind, leases: &IdLeases) -> u64 {
    // The internal, unchecked read (R4 is checked where state is read): a
    // Shared init may take an id. Its id differs in a fork and live, which
    // is fine: a stand-in's id is a fork id and goes with the stand-in.
    #[cfg(debug_assertions)]
    let forking = is_cold();
    #[cfg(not(debug_assertions))]
    let forking = false;
    let key = (
        kind.site.file(),
        kind.site.line(),
        kind.site.column(),
        std::ptr::from_ref(leases).addr(),
    );
    ID_CURSORS.with(|c| {
        let mut c = c.borrow_mut();
        let cursor = c.entry(key).or_default();
        let last = if forking { &mut cursor.fork } else { &mut cursor.live };
        let id = leases.next(*last, forking);
        *last = id;
        id
    })
}

/// Ids run out (2^63 handed out in one process): never in practice;
/// reported rather than an id reused.
#[expect(clippy::panic, reason = "an id handed out twice would be a silent bug")]
fn exhausted_ids() -> ! {
    panic!("gui: the process handed out 2^63 ids")
}

/// Whether a fork is open on this thread (a check, ADR-0028): what a
/// handle, a sender or a sweep reads to keep the fork's effects inside it.
/// Always `false` while the fork is built in debug builds only (F2).
/// Never read by a Shared init: its value would differ in a fork and live
/// ([`no_forked_read`]); the engine's own allocation paths read
/// `reactive::is_cold`.
#[must_use]
pub fn forking() -> bool {
    #[cfg(debug_assertions)]
    no_forked_read("whether a fork runs (`forking()`)");
    #[cfg(debug_assertions)]
    return is_cold();
    #[cfg(not(debug_assertions))]
    false
}

/// Fork: a copy of the live value kept to put back.
#[cfg(debug_assertions)]
fn fork_enter<S: Snapshot>(slot: &S) -> Box<dyn Any> {
    fork_work(1);
    Box::new(slot.snapshot())
}

/// Fork: the frame's value taken out, a copy of the live one in its place.
/// A cell the fork never wrote (it holds the live handle itself) gives
/// nothing: the drain never holds a live handle in a batch a runaway could
/// set aside, so even a runaway pins nothing live (ADR-0034, S2; Codex
/// r15-3).
#[cfg(debug_assertions)]
fn fork_drain<S: Snapshot>(slot: &S, saved: &dyn Any) -> Option<Box<dyn Any>> {
    let live = saved.downcast_ref::<S::Inner>()?;
    if slot.same(live) {
        return None;
    }
    Some(Box::new(slot.put(S::copy(live))))
}

/// A registered piece's value made by `init`, and whether `init` read
/// engine state (a slot of another piece): what makes a value a fork
/// made the fork's ([`Entry`]).
#[cfg(debug_assertions)]
fn made_by<T>(init: fn() -> T) -> (T, bool) {
    gui_tls::noting(init)
}

/// Declare per-thread engine state, each with its policy:
///
/// ```ignore
/// engine_local! {
///     /// Timers due.
///     [Fresh] static TIMERS: RefCell<Vec<Timer>> = RefCell::new(Vec::new());
///     [Fork] static NEXT: Cell<u64> = const { Cell::new(1) };
///     [Fork(log)] static BOUND: RefCell<ForkMap<u64, Flags>> = RefCell::default();
///     [Fork(enter_fn, drain_fn, exit_fn)] static TREE: RefCell<Tree> = RefCell::new(Tree::default());
///     [Shared("a handle; its value is the forked runtime's")] static SIZE: Signal<u32> = make();
/// }
/// ```
///
/// - **Fresh**: the cold frame sees a new, empty instance (queues, caches).
/// - **Fork**: put back exactly at its end; a plain `[Fork]` is a `Copy`
///   cell or one handle ([`Snapshot`], O(1)); anything larger is
///   `[Fork(log)]` ([`Logged`]) or has its own undo log.
/// - **Shared**: neither, with its reason; made live, owned by no scope
///   ([`shared_init`]), by an init that reads no state a fork changes.
///   First touched in a fork, the fork gets a stand-in of its own, dropped
///   with it; live code makes the piece at its own first touch (ADR-0028).
#[macro_export]
#[doc(hidden)]
macro_rules! engine_local {
    () => {};
    ($(#[$meta:meta])* [Fresh] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            $crate::engine::EngineKey::fresh(stringify!($name), init)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            $crate::engine::EngineKey::copied(stringify!($name), init)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork(log)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::LogKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            $crate::engine::LogKey::logged(stringify!($name), init)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork($enter:path, $drain:path, $exit:path)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            // Engine-private (ADR-0034, S7): outside `gui`, this does not
            // compile.
            #[cfg(debug_assertions)]
            let key = $crate::engine::EngineKey::custom(stringify!($name), init, $enter, $drain, $exit);
            #[cfg(not(debug_assertions))]
            let key = $crate::engine::EngineKey::new(init, None);
            key
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Shared($why:literal)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        #[doc = concat!("\n\nShared with a cold frame: ", $why)]
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $crate::engine::shared_init(stringify!($name), || $init)
            }
            $crate::engine::EngineKey::shared(init)
        };
        $crate::engine_local! { $($rest)* }
    };
}

#[doc(hidden)]
pub use crate::engine_local;

/// What an entered piece's exit puts back (T0): never moved, boxed again or
/// replaced until the restore (T4); the drain reads it through a shared
/// handle, so no drain hook can leave it half-taken.
type Undo = std::rc::Rc<Box<dyn Any>>;

/// The pieces registered on this thread, and the cold frame's saved
/// values while one runs.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
#[derive(Default)]
struct Registry {
    entries: Vec<Entry>,
    /// Each piece registered: its slot, its name and its type.
    named: Vec<(usize, &'static str, &'static str)>,
    /// While a cold frame runs: what each entered piece's exit puts back,
    /// by entry.
    cold: Option<Vec<(Entry, Undo)>>,
    /// What the running fork's result is worth ([`Doom`]).
    #[cfg(debug_assertions)]
    doom: Doom,
    /// The first site that doomed it to be dropped ([`drop_fork`]).
    #[cfg(debug_assertions)]
    dropped_at: Option<&'static str>,
    /// The running check's owner's flag, set from any thread as live work
    /// arrives ([`CancelFlag`]): read at each cancellation point.
    #[cfg(debug_assertions)]
    cancel: Option<CancelFlag>,
    /// The flag the next check opens with ([`cancel_next_check_by`]).
    #[cfg(debug_assertions)]
    next_cancel: Option<CancelFlag>,
    /// The running check is being torn down: cancellation points are
    /// inert ([`cancel_point`]).
    #[cfg(debug_assertions)]
    ending: bool,
    /// Tests: the thread CPU time between cancellation points (V6).
    #[cfg(all(test, debug_assertions))]
    slices: SliceLog,
    /// The stand-ins of the Shared pieces the running fork touched first,
    /// by slot, in the order made ([`EngineKey::stand_in`]): the fork's,
    /// dropped last of all it made.
    #[cfg(debug_assertions)]
    stand_ins: Vec<(usize, std::rc::Rc<dyn Any>)>,
    /// The slots whose stand-in the running fork has dropped: one reached
    /// again would be a second ([`EngineKey::stand_in`]).
    #[cfg(debug_assertions)]
    stood_down: Vec<usize>,
    /// The world of the running fork, or of the last one opened on this
    /// thread ([`world`]): taken from [`WORLDS`], so unique in the process
    /// (ADR-0035, D5). Nothing puts it back (an identity source).
    #[cfg(debug_assertions)]
    serial: u64,
    /// Why no check opens on this thread: an earlier one faulted
    /// (ADR-0034, S2). Never cleared.
    #[cfg(debug_assertions)]
    poisoned: Option<String>,
    /// The disposals of another world's reactive nodes the running fork
    /// was refused (ADR-0035, D3): each a fault of the check.
    #[cfg(debug_assertions)]
    foreign: Vec<String>,
    /// The pieces whose exit found a value left at the switch, quarantined
    /// ([`quarantined`]).
    #[cfg(debug_assertions)]
    left: Vec<&'static str>,
}

/// What a fork's result is worth, once it ends (ADR-0028). A fork never
/// runs live code, so what it cannot do inside itself it does not do: it
/// runs on to its end (nothing unwinds through the sites that catch a
/// panic), and its result is thrown away.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(crate) enum Doom {
    /// Its result stands.
    #[default]
    Kept,
    /// It reached a site that cannot answer inside a fork
    /// ([`crate::element::InFork::Refuse`]): it is dropped (a sampled
    /// check skips the sample), as running it again reaches the same site.
    Dropped,
    /// Its owner no longer wants it (live work arrived: ADR-0034, S10): it
    /// stops at its next cancellation point and is torn down as any other.
    Cancelled,
}

/// Cancel the running check: it stops at its next cancellation point
/// ([`cancelled`]) and reports `Cancelled`. Nothing outside one. The
/// release sampler cancels a check as input arrives (F2); a debug check
/// is cancelled by a test.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the release sampler cancels (F2); until it lands, tests do")
)]
pub(crate) fn cancel_check() {
    #[cfg(debug_assertions)]
    REGISTRY
        .try_with(|r| {
            let mut r = r.borrow_mut();
            if r.cold.is_some() {
                r.doom = Doom::Cancelled;
            }
        })
        .ok();
}

/// Whether the running check was cancelled: by [`cancel_check`] on its
/// thread, or by its owner's [`CancelFlag`] from any thread.
#[must_use]
#[cfg(debug_assertions)]
pub(crate) fn cancelled() -> bool {
    REGISTRY
        .try_with(|r| {
            r.try_borrow().is_ok_and(|r| {
                r.cold.is_some()
                    && !r.ending
                    && (r.doom == Doom::Cancelled || r.cancel.as_ref().is_some_and(CancelFlag::is_set))
            })
        })
        .unwrap_or(false)
}

/// A cancellation point inside a phase of a check (ADR-0028: abandoned at
/// a step boundary as input arrives, inside layout and paint too): if the
/// check was cancelled, it unwinds now to the check's own catch, which
/// tears it down and reports `Cancelled`. A no-op outside a check, and in
/// one not cancelled (one thread-local read). The unwind is the check's
/// own marker, raised without the panic hook (nothing is reported), and it
/// never leaves the check. While the thread unwinds (a drop on the way out
/// of a check that reaches a render), it does nothing: unwinding again
/// from inside an unwind aborts the process, and the check ends anyway.
pub(crate) fn cancel_point() {
    #[cfg(all(test, debug_assertions))]
    if is_cold() {
        slice_point(false);
    }
    #[cfg(debug_assertions)]
    if is_cold() && !std::thread::panicking() && cancelled() {
        #[cfg(test)]
        REGISTRY.try_with(|r| r.borrow_mut().slices.fired = cpu_now()).ok();
        std::panic::resume_unwind(Box::new(CancelUnwind));
    }
}

/// Tests (V6): the thread CPU time a check spends between cancellation
/// points, the longest it runs uninterruptible, and where a cancel fired:
/// time a contended machine cannot stretch, as wall time is.
#[cfg(all(test, debug_assertions))]
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct SliceLog {
    pub(crate) last: Option<std::time::Duration>,
    /// The longest slice so far in the running (or last) check.
    pub(crate) max: std::time::Duration,
    /// Points the running check passed.
    pub(crate) seen: u64,
    /// Cancel the running check at this point.
    pub(crate) cancel_at: Option<u64>,
    /// The thread CPU time at which a cancel fired.
    pub(crate) fired: Option<std::time::Duration>,
}

#[cfg(all(test, debug_assertions))]
fn cpu_now() -> Option<std::time::Duration> {
    cpu_time::ThreadTime::try_now().ok().map(|t| t.as_duration())
}

/// One cancellation point passed (or, `edge`, the check opening or ending:
/// a slice's bound that is no point).
#[cfg(all(test, debug_assertions))]
pub(crate) fn slice_point(edge: bool) {
    let now = cpu_now();
    REGISTRY
        .try_with(|r| {
            let Ok(mut r) = r.try_borrow_mut() else { return };
            let s = &mut r.slices;
            if let (Some(now), Some(last)) = (now, s.last) {
                s.max = s.max.max(now.saturating_sub(last));
            }
            s.last = now;
            if !edge {
                s.seen = s.seen.saturating_add(1);
                if s.cancel_at == Some(s.seen) {
                    r.doom = Doom::Cancelled;
                }
            }
        })
        .ok();
}

/// Tests: start a slice log for the next check, cancelling it at point
/// `cancel_at` if given.
#[cfg(all(test, debug_assertions))]
pub(crate) fn log_slices(cancel_at: Option<u64>) {
    REGISTRY
        .try_with(|r| {
            r.borrow_mut().slices = SliceLog {
                cancel_at,
                ..SliceLog::default()
            };
        })
        .ok();
}

/// Tests: the slice log so far, and the thread CPU time now.
#[cfg(all(test, debug_assertions))]
pub(crate) fn slices() -> (SliceLog, Option<std::time::Duration>) {
    (REGISTRY.with(|r| r.borrow().slices), cpu_now())
}

/// The unwind of a cancelled check ([`cancel_point`]).
#[cfg(debug_assertions)]
pub(crate) struct CancelUnwind;

/// A check's cancellation, as its owner holds it: set from any thread (the
/// input thread, as input arrives), read by the check at its cancellation
/// points. Shared by clones.
#[derive(Clone, Default, Debug)]
pub(crate) struct CancelFlag(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl CancelFlag {
    /// Cancel the check it is given to: it stops at its next cancellation
    /// point. Stays set: a flag cancels one check.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the input thread sets it (F2); until then, tests")
    )]
    pub(crate) fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }

    #[cfg_attr(
        not(debug_assertions),
        expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
    )]
    fn is_set(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Give the next check on this thread `flag` to be cancelled by (taken by
/// the check as it opens; [`fork`] clears it as the check ends).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the input thread cancels (F2); until then, tests")
)]
pub(crate) fn cancel_next_check_by(flag: &CancelFlag) {
    #[cfg(debug_assertions)]
    REGISTRY
        .try_with(|r| r.borrow_mut().next_cancel = Some(flag.clone()))
        .ok();
    #[cfg(not(debug_assertions))]
    let _ = flag;
}

/// Doom the running fork, to be dropped (ADR-0028): it reached `site`,
/// which cannot answer inside a fork. Nothing outside one (no fork runs
/// in a release build yet, F2).
pub(crate) fn drop_fork(site: &'static str) {
    #[cfg(debug_assertions)]
    REGISTRY
        .try_with(|r| {
            let mut r = r.borrow_mut();
            if r.cold.is_some() && r.doom != Doom::Cancelled {
                r.doom = Doom::Dropped;
                r.dropped_at.get_or_insert(site);
            }
        })
        .ok();
    #[cfg(not(debug_assertions))]
    let _ = site;
}

/// What the running fork is worth so far ([`Doom::Kept`] outside one).
#[cfg(debug_assertions)]
pub(crate) fn doomed() -> Doom {
    REGISTRY.try_with(|r| r.borrow().doom).unwrap_or_default()
}

/// The registry itself: in a slot as the pieces are, entered by no frame.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
static REGISTRY: EngineKey<RefCell<Registry>> = EngineKey::new(|| RefCell::new(Registry::default()), None);

/// This thread's arena serial ([`arena`]): an id, set as the thread first
/// asks, never written after.
static ARENA: EngineKey<u64> = EngineKey::new(|| unique_id(&ARENAS), None);

/// The arenas' serials, numbered.
static ARENAS: IdKind = IdKind::new("arena");

/// This thread's serial: what names its arenas to a value that crosses
/// threads ([`crate::reactive::OwnerId`]), never another thread's.
pub(crate) fn arena() -> u64 {
    ARENA.with(|a| *a)
}

/// What the cold frames forked so far on this thread: a copy or an undo
/// entry each, over every Fork piece.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
static FORK_WORK: EngineKey<Cell<u64>> = EngineKey::new(|| Cell::new(0), None);

/// Count `n` copies or undo entries a cold frame's fork made.
#[inline]
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
pub(crate) fn fork_work(n: u64) {
    #[cfg(debug_assertions)]
    FORK_WORK.try_with(|w| w.set(w.get().wrapping_add(n))).ok();
    #[cfg(not(debug_assertions))]
    let _ = n;
}

/// [`fork_work`] counted on this thread so far (tests).
#[cfg(all(debug_assertions, test))]
pub(crate) fn fork_work_done() -> u64 {
    FORK_WORK.get()
}

/// A piece used the first time on this thread: entered at once if a cold
/// frame runs and `enter` (its value is the one it starts with, kept to
/// put back). A piece the fork made from engine state is not: its value
/// is the fork's, and no undo record holds it (R1).
#[cfg(debug_assertions)]
fn register(e: Entry, slot: usize, ty: &'static str) {
    REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.entries.push(e);
        r.named.push((slot, e.name, ty));
    });
    enter_now(e);
}

/// Enter `e` into the running cold frame, if one runs: the engine's own
/// work, no read of a Shared init that first touched the piece
/// ([`no_forked_read`]).
#[cfg(debug_assertions)]
fn enter_now(e: Entry) {
    if is_cold() {
        let _engine = making().is_some().then(|| Making::push(Mark::Engine));
        let saved = e.enter();
        REGISTRY.with(|r| {
            if let Some(c) = r.borrow_mut().cold.as_mut() {
                c.push((e, std::rc::Rc::new(saved)));
            }
        });
    }
}

/// What runs inside a Shared init, innermost last ([`shared_init`]).
#[cfg(debug_assertions)]
static MAKING: EngineKey<RefCell<Vec<Mark>>> = EngineKey::new(RefCell::default, None);

/// One level of [`MAKING`]: a Shared piece's init, by the piece's name, or
/// the engine's own bookkeeping inside one ([`allocating`]). The second is
/// a variant of a private type, set only by the engine: no value a caller
/// passes (a name, the empty one included) can stand for it, so the R4
/// exemption cannot be forged (F1 review 13).
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
enum Mark {
    Piece(&'static str),
    Engine,
}

/// A level of [`MAKING`], until it drops.
#[cfg(debug_assertions)]
struct Making;

#[cfg(debug_assertions)]
impl Making {
    fn push(mark: Mark) -> Self {
        MAKING.with(|m| m.borrow_mut().push(mark));
        Self
    }
}

#[cfg(debug_assertions)]
impl Drop for Making {
    fn drop(&mut self) {
        MAKING.try_with(|m| m.borrow_mut().pop()).ok();
    }
}

/// What runs inside a Shared init now, if any (innermost).
#[cfg(debug_assertions)]
fn making() -> Option<Mark> {
    MAKING.try_with(|m| m.borrow().last().copied()).ok().flatten()
}

/// A Shared piece is made from nothing: no component's context, nor the
/// instance tree (which a cold frame forks, so what it read could be the
/// frame's). Called where those are read.
#[cfg(debug_assertions)]
#[expect(
    clippy::panic,
    reason = "a Shared value made from a component's context is a bug in its declaration"
)]
pub(crate) fn no_component_context() {
    if making().is_some() {
        panic!("a Shared engine value's init may not read component context");
    }
}

/// A Shared init reads no state a fork changes (ADR-0028, F1 review 9 and
/// 10, R4): a registered piece (a `[Fork]` or `[Fresh]` cell, the focus,
/// the tree), a reactive value ([`no_reactive_read`]: a signal's or a
/// computed's, a Shared piece's included, as a fork may have written it),
/// nor whether a fork runs. Its value is then the same in a fork and live
/// (a pure init, as a GHC top-level thunk's), so a fork's stand-in is what
/// live code makes, and neither is rolled back nor rerun (research-f1).
/// Making a handle is no read: the arena's bookkeeping passes
/// ([`EngineKey::with_arena`]). Panics naming the piece.
#[cfg(debug_assertions)]
#[expect(
    clippy::panic,
    reason = "a Shared init that reads forked state is a bug in its declaration"
)]
pub(crate) fn no_forked_read(what: &str) {
    if let Some(Mark::Piece(piece)) = making() {
        panic!(
            "the init of the Shared piece `{piece}` reads {what}, state a fork changes: a Shared init reads none (make the piece [Fork], or pass the value in)"
        );
    }
}

/// `value` inserted into `slab` as a handle's new state: an allocation, as
/// a node in the reactive arena is, which a Shared init may make (a fork
/// puts it in its own region, gone with it; live code, live). Reading or
/// writing an entry is not one ([`no_forked_read`]).
pub(crate) fn allocate<T: ForkValue>(slab: &'static LogKey<RefCell<ForkSlab<T>>>, value: T) -> SlabId {
    allocating(|| slab.with_borrow_mut(|s| s.insert(value)))
}

/// Run `f`, the engine's own bookkeeping as it makes a handle (a node
/// allocated in the reactive arena and filled, a label registered, what
/// a failed Shared init made undone): the unchecked internal path, which a
/// Shared init may reach. Every other read is checked where the state is
/// read ([`no_forked_read`]). `f` runs no caller code.
pub(crate) fn allocating<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(debug_assertions)]
    let _allocating = making().is_some().then(|| Making::push(Mark::Engine));
    f()
}

/// A Shared piece's init ([`engine_local!`]), `name` the piece's: owned by
/// no scope and tracked by nothing (what first touched it is not what it
/// lives as long as), and as one step, all of what it made undone if it
/// panics. Live outside a fork; in one, it makes the fork's stand-in, never
/// the piece ([`EngineKey::stand_in`]): a fork never runs live code
/// (ADR-0028). It reads no state a fork changes ([`no_forked_read`]).
pub fn shared_init<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    // The engine's own steps around the init (no owner, its allocations
    // recorded, undone if it panics) are outside the piece's making, and
    // the engine's own inside another's ([`allocating`]): only `f` is
    // checked ([`no_forked_read`]).
    allocating(|| {
        crate::reactive::detached(|| {
            crate::reactive::making_shared(|| {
                #[cfg(debug_assertions)]
                let _making = Making::push(Mark::Piece(name));
                #[cfg(not(debug_assertions))]
                let _ = name;
                f()
            })
        })
    })
}

/// Never in a release build (F2).
#[cfg(not(debug_assertions))]
pub(crate) const fn is_cold() -> bool {
    false
}

/// Whether a cold frame runs on this thread: the engine's own, unchecked
/// read (a Shared init's goes through `forking()` or `reactive::is_cold`,
/// which refuse it).
#[cfg(debug_assertions)]
pub(crate) fn is_cold() -> bool {
    REGISTRY.try_with(|r| r.borrow().cold.is_some()).unwrap_or(false)
}

/// The live world's id ([`world`]).
pub(crate) const LIVE: u64 = 0;

/// The worlds' ids, process-wide (ADR-0035, D5): a fork's is taken here as
/// it opens, so no two forks of any threads share one, and a world cell
/// sent to another thread matches no check there. Live is [`LIVE`].
#[cfg(debug_assertions)]
static WORLDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A new fork's world id ([`WORLDS`]).
#[cfg(debug_assertions)]
pub(crate) fn new_world() -> u64 {
    WORLDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// The running world (ADR-0034, S6; ADR-0035, D5): [`LIVE`]; in a fork, its
/// id, which no other fork in the process has had or will have. The one
/// source of world identity in every build (world cells, [`Ends`], slab
/// ids): it follows [`is_cold`], so a build that runs checks sets both.
#[cfg(debug_assertions)]
pub(crate) fn world() -> u64 {
    REGISTRY
        .try_with(|r| {
            let r = r.borrow();
            if r.cold.is_some() { r.serial } else { LIVE }
        })
        .unwrap_or(LIVE)
}

/// [`world`] in a build that runs no check: live, as [`is_cold`] is false.
#[cfg(not(debug_assertions))]
pub(crate) const fn world() -> u64 {
    LIVE
}

/// How many lends of world state (a world cell's or a logged piece's
/// `with` and `with_mut` closures) run on this thread now: a check opened
/// meanwhile is refused (ADR-0035, D4). The registry's own, entered by no
/// check.
#[cfg(debug_assertions)]
static LENDS: EngineKey<Cell<u32>> = EngineKey::new(|| Cell::new(0), None);

/// One count of user code running ([`LENDS`]), held while it runs and
/// ended however it ends. The reactive runtime holds one in each guard it
/// switches the running subscriber or owner with (`RunGuard`:
/// a computed's getter, an effect, a tracked render, binding or paint;
/// `OwnerGuard`: `untracked`, `detached`, a scope's `run`), so every call
/// from the engine into user code is counted at that one gate (ADR-0035,
/// D4); [`lend`] holds one around a lend of world state.
pub(crate) struct UserCode(());

impl UserCode {
    #[inline]
    pub(crate) fn enter() -> Self {
        #[cfg(debug_assertions)]
        LENDS.try_with(|l| l.set(l.get().saturating_add(1))).ok();
        Self(())
    }
}

impl Drop for UserCode {
    #[inline]
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        LENDS.try_with(|l| l.set(l.get().saturating_sub(1))).ok();
    }
}

/// Run `f` as a lend of world state ([`LENDS`]), counted while it runs,
/// however it ends.
#[inline]
pub(crate) fn lend<R>(f: impl FnOnce() -> R) -> R {
    let _lent = UserCode::enter();
    f()
}

/// Whether a lend of world state runs on this thread ([`lend`]).
#[cfg(debug_assertions)]
pub(crate) fn lent() -> bool {
    LENDS.try_with(|l| l.get() > 0).unwrap_or(false)
}

/// Refuse the disposal of reactive node `what`, another world's, inside the
/// running fork (ADR-0035, D3): the fork's fault, said at its teardown.
#[cfg(debug_assertions)]
pub(crate) fn foreign_disposal(what: String) {
    REGISTRY
        .try_with(|r| {
            if let Ok(mut r) = r.try_borrow_mut() {
                r.foreign.push(what);
            }
        })
        .ok();
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{ColdFrame, is_cold};
    use crate::reactive::{Effect, Signal};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    crate::engine_local! {
        [Fork(log)] static BIG: std::cell::RefCell<super::ForkMap<u64, u64>> = std::cell::RefCell::default();
    }

    /// F1 review 13: a `ForkMap` forks by an overlay of its writes, so a
    /// fork costs the keys it writes, not the map's size (a whole-map copy
    /// at its first touch would cost 100,000 here), and its exit moves
    /// alone. Reads see the fork's writes; the live map is as it was.
    #[test]
    fn a_fork_map_costs_its_writes_not_its_size() {
        BIG.with_borrow_mut(|m| {
            for k in 0..100_000 {
                m.insert(k, k);
            }
        });
        let before = super::fork_work_done();
        let started = std::time::Instant::now();
        let seen = super::in_cold_frame(|| {
            BIG.with_borrow_mut(|m| {
                m.insert(7, 70);
                m.insert(100_001, 1);
                m.remove(&9);
                *m.entry_mut(11) += 1;
            });
            BIG.with_borrow(|m| {
                (
                    m.get(&7).copied(),
                    m.get(&9).copied(),
                    m.get(&11).copied(),
                    m.len(),
                    m.iter().count(),
                )
            })
        });
        let took = started.elapsed();
        let work = super::fork_work_done() - before;
        eprintln!("a fork writing 4 keys of a 100,000-key ForkMap: {work} steps, {took:?}");
        assert_eq!(
            seen,
            (Some(70), None, Some(12), 100_000, 100_000),
            "the fork reads its writes"
        );
        assert!(work < 100, "the fork's cost is its writes: {work} steps");
        BIG.with_borrow(|m| {
            assert_eq!(
                (m.get(&7), m.get(&9), m.get(&11), m.len()),
                (Some(&7), Some(&9), Some(&11), 100_000)
            );
        });
    }

    /// A timer only a cold frame made is gone with it: it never fires.
    #[test]
    fn a_timer_a_cold_frame_set_never_fires() {
        crate::clock::script(Duration::ZERO);
        let fired = Rc::new(Cell::new(false));
        {
            let _cold = ColdFrame::begin();
            let f = Rc::clone(&fired);
            crate::clock::after(Duration::from_millis(1), move || f.set(true));
        }
        crate::clock::advance_to(Duration::from_secs(1));
        crate::clock::run_due_timers();
        assert!(!fired.get(), "the cold frame's timer fired");
        assert!(crate::clock::next_deadline().is_none(), "nothing pending");
    }

    /// Codex 3 (F1 review 3): no id a cold frame hands out is handed out
    /// again live after it, of any kind a caller can hold (rule 3), and a
    /// signal it made names nothing after it.
    #[test]
    fn a_cold_frame_hands_out_no_id_the_live_run_hands_out_again() {
        let make = || {
            (
                crate::clock::after(Duration::from_secs(9), || {}),
                crate::focus::FocusHandle::new(),
                Signal::new(0_u8),
                crate::access::NodeRef::new(),
                crate::events::CaptureGroup::new(),
                crate::layer::LayerHandle::new(),
                crate::holder::Registration::new(),
            )
        };
        // Each counter in use before the frame.
        let _ = make();
        let cold = {
            let _cold = ColdFrame::begin();
            make()
        };
        let live = make();
        assert!(cold.0 != live.0, "timer");
        assert!(cold.1 != live.1, "focus");
        assert!(cold.2 != live.2, "signal");
        assert!(cold.3 != live.3, "access ref");
        assert!(cold.4 != live.4, "capture group");
        assert!(cold.5 != live.5, "layer");
        assert!(cold.6 != live.6, "registration");
        assert!(!cold.2.is_alive() && live.2.is_alive(), "the cold signal names nothing");
        crate::clock::cancel_all();
    }

    /// The content ids (ADR-0028, rule 3: a leased block and a counter of
    /// the thread's, registered Fork) a cold frame hands out are the forks'
    /// (Codex, F1 review 2: never ids the live counters go on to), and the
    /// live ids after it are as on a thread that ran no check: each kind's
    /// step from the ids before is the same. What the frame made equals
    /// the live one by content, as the cold check compares it.
    #[test]
    fn a_cold_frame_leaves_the_content_ids_as_they_were() {
        fn make() -> (Vec<u64>, crate::chunk::Bytes) {
            let mut series = crate::chunk::series::SeriesState::new(None, false);
            series.append(&[0], &[0.0]);
            let page = series.snapshot().pages().next().map_or(0, |(p, ..)| p.seq);
            let mut scene = crate::scene::Scene::new((1, 1), 1.0, None);
            scene.push(Vec::new());
            let chunk = scene.chunks.first().map_or(0, |c| c.id.0);
            let grid = crate::chunk::GridSnapshot::new(1, 1, vec![0.0]).data().id;
            let readback = crate::engine::owned(crate::chunk::readback).to().id();
            let text = crate::edit::TextEdit::new("").revision();
            let ids = vec![series.id(), page, chunk, grid, readback, text];
            (ids, crate::chunk::Bytes::new(vec![1, 2]))
        }
        /// On a thread of its own: the steps from the ids before to the
        /// live ones after, and the cold frame's ids, if it ran one.
        fn run(check: bool) -> (Vec<u64>, Option<Vec<u64>>) {
            std::thread::spawn(move || {
                let before = make();
                let cold = check.then(|| {
                    let _cold = ColdFrame::begin();
                    make()
                });
                let live = make();
                if let Some(cold) = &cold {
                    assert!(cold.1 != live.1 && cold.1.data() == live.1.data(), "bytes: by content");
                    for (c, l) in cold.0.iter().zip(&live.0) {
                        assert_ne!(c, l, "the fork's id is never the live one");
                    }
                }
                let steps = live.0.iter().zip(&before.0).map(|(l, b)| l.wrapping_sub(*b)).collect();
                (steps, cold.map(|c| c.0))
            })
            .join()
            .expect("a thread")
        }
        let (plain, _) = run(false);
        let (checked, cold) = run(true);
        assert_eq!(checked, plain, "the live ids step as without the check");
        assert!(cold.is_some());
    }

    /// Each thread hands out ids from blocks of its own: two threads never
    /// hand out the same one, and a cold frame on one leaves the other's
    /// alone.
    #[test]
    fn ids_of_two_threads_never_meet() {
        static NEXT: super::IdKind = super::IdKind::new("test");
        let here: Vec<u64> = (0..3).map(|_| super::unique_id(&NEXT)).collect();
        let there = std::thread::spawn(|| {
            let first: Vec<u64> = (0..3).map(|_| super::unique_id(&NEXT)).collect();
            let cold = {
                let _cold = ColdFrame::begin();
                super::unique_id(&NEXT)
            };
            (first, cold, super::unique_id(&NEXT))
        })
        .join()
        .expect("the other thread");
        assert!(here.iter().all(|id| !there.0.contains(id)), "{here:?} {there:?}");
        assert_ne!(there.1, there.2, "the cold id is the fork's, never a live one");
        assert_eq!(
            Some(there.2),
            there.0.get(2).and_then(|id| id.checked_add(1)),
            "the live counter goes on as without the check"
        );
    }

    /// Codex 1, Opus 2 (F1 review 4): a thread's first timer, focus handle
    /// and node ref are its own. A cursor starts at none, so each thread's
    /// first id leases a block of its own, never block 0 that every thread
    /// would share.
    #[test]
    fn the_first_ids_of_two_threads_differ() {
        let first = || {
            std::thread::spawn(|| {
                let timer = crate::clock::after(Duration::from_secs(9), || {});
                crate::clock::cancel_all();
                (timer, crate::focus::FocusHandle::new(), crate::access::NodeRef::new())
            })
            .join()
            .expect("a thread")
        };
        let (one, other) = (first(), first());
        assert_ne!(one.0, other.0, "timer");
        assert_ne!(one.1, other.1, "focus");
        assert_ne!(one.2, other.2, "node ref");
    }

    crate::engine_local! {
        /// A handle made first inside a check, kept across it.
        [Shared("a test of a Shared init that mints an id inside a check")]
        static KEPT_FOCUS: crate::focus::FocusHandle = crate::focus::FocusHandle::new();
    }

    /// Codex 2 (F1 review 4), F1 review 8: a Shared piece first touched in
    /// a check is made live after it (ADR-0028) and takes a live id; the
    /// fork's stand-in had an id of the forks' space. The next live handle
    /// gets an id of its own, neither of theirs.
    #[test]
    fn a_shared_init_in_a_check_keeps_an_id_no_live_handle_gets_again() {
        let before = crate::focus::FocusHandle::new();
        let stand_in = {
            let _cold = ColdFrame::begin();
            KEPT_FOCUS.with(|h| *h)
        };
        let kept = KEPT_FOCUS.with(|h| *h);
        let next = crate::focus::FocusHandle::new();
        assert_ne!(kept, stand_in, "the live piece is not the fork's stand-in");
        assert_ne!(kept, next, "the next live handle names the kept one");
        assert_ne!(stand_in, next);
        assert_ne!(before, next);
    }

    /// Codex 3 (F1 review 4): two checks that make a signal in the same
    /// slot give it two generations. A signal kept from the first is dead
    /// in the second: it reads nothing, and a write to it reaches nothing.
    #[test]
    fn a_signal_kept_from_one_check_is_dead_in_the_next() {
        let first = {
            let _cold = ColdFrame::begin();
            Signal::new(1_u32)
        };
        let _cold = ColdFrame::begin();
        let second = Signal::new(2_u32);
        assert_eq!(first.key().0, second.key().0, "the same slot");
        assert_ne!(first.key(), second.key(), "two generations");
        assert!(!first.is_alive(), "the first check's signal names nothing");
        assert_eq!(second.get_untracked(), 2);
    }

    crate::engine_local! {
        /// A signal made first inside a check, kept live past it.
        [Shared("a test of a Shared node made above a check's nodes")]
        static KEPT_SIGNAL: Signal<u32> = Signal::new(7);
    }

    /// Opus 1 (F1 review 4): a Shared node made live above a check's own
    /// nodes keeps the arena long past the frame's, whose slots below it
    /// are freed. A signal each later check makes in the freed slot, kept
    /// by a caller, names nothing in the check after it or live, and the
    /// live signal made there after is a new one.
    #[test]
    fn a_kept_fork_handle_names_nothing_over_two_checks() {
        let check = |make: &dyn Fn() -> Signal<u32>| {
            let _cold = ColdFrame::begin();
            make()
        };
        let first = check(&|| {
            let forked = Signal::new(1_u32);
            assert_eq!(KEPT_SIGNAL.with(|s| s.get_untracked()), 7);
            forked
        });
        assert!(KEPT_SIGNAL.with(Signal::is_alive), "the Shared signal is kept");
        let second = check(&|| Signal::new(2_u32));
        let third = check(&|| {
            let third = Signal::new(3_u32);
            assert!(
                !first.is_alive() && !second.is_alive(),
                "a kept fork handle names nothing"
            );
            third
        });
        assert_eq!(first.key().0, second.key().0, "the freed slot");
        assert_eq!(second.key().0, third.key().0, "the freed slot");
        assert!(
            first.key() != second.key() && second.key() != third.key(),
            "three generations"
        );
        let live = Signal::new(4_u32);
        assert!(
            ![first, second, third].iter().any(Signal::is_alive),
            "nor live after them"
        );
        assert!([first, second, third].iter().all(|s| s.key() != live.key()));
        assert_eq!(live.get_untracked(), 4);
    }

    /// Ids are leased in blocks: a block that runs out leases the next, a
    /// fork leases from a space of its own, and threads that come and go
    /// take no number of their own, so nothing runs out however many there
    /// are.
    #[test]
    fn ids_lease_blocks_and_never_run_out() {
        let leases = super::IdLeases::new(4);
        let mut ids = vec![leases.next(0, false)];
        for _ in 0..9 {
            let last = *ids.last().expect("an id");
            ids.push(leases.next(last, false));
        }
        assert_eq!(ids, [4, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
        // A fork at 15 leases from its own space; the live one is untouched.
        let fork = leases.next(15, true);
        assert!(fork >= (super::IdLeases::BLOCKS / 2) * 4, "{fork}: the forks' space");
        // Codex (F1 review 2): a fork that begins mid-block (live at 6)
        // hands out none of the ids the live counter, put back, goes on to.
        let mut fork_ids = vec![leases.next(6, true)];
        for _ in 0..5 {
            let last = *fork_ids.last().expect("an id");
            fork_ids.push(leases.next(last, true));
        }
        assert!(fork_ids.iter().all(|id| *id >= leases.forks_from()), "{fork_ids:?}");
        assert_eq!(leases.next(6, false), 7, "the live block goes on where it was");
        assert_eq!(
            leases.next(15, false),
            16,
            "the live lease after it is the one before it"
        );
        // Many short-lived threads, each one id: no number per thread.
        static NEXT: super::IdKind = super::IdKind::new("test");
        let mut seen = foldhash::HashSet::default();
        for _ in 0..2_000 {
            let id = std::thread::spawn(|| super::unique_id(&NEXT)).join().expect("a thread");
            assert!(seen.insert(id), "{id} handed out twice");
        }
    }

    /// Coordinator (F1 review 1, ADR-0028's contract): a check leaves the
    /// live id sequence as it was. Two runs from the same allocator state
    /// and the same schedule, the second with cold frames at its first use
    /// and across a block's end: the live ids are equal. Each run has an
    /// allocator and counter of its own, so tests running beside it on
    /// other threads do not move them.
    #[test]
    fn a_check_leaves_the_live_id_sequence_as_it_was() {
        static NEXT_A: super::IdKind = super::IdKind::new("test a");
        static NEXT_B: super::IdKind = super::IdKind::new("test b");
        let (one, other) = (super::IdLeases::new(4), super::IdLeases::new(4));
        let take = |next: &'static super::IdKind, leases: &super::IdLeases, n: usize| {
            (0..n).map(|_| super::unique_id_in(next, leases)).collect::<Vec<_>>()
        };
        let plain = [take(&NEXT_A, &one, 3), take(&NEXT_A, &one, 8)].concat();
        let cold = |n| {
            let _cold = ColdFrame::begin();
            take(&NEXT_B, &other, n)
        };
        // A check before the first live id, one that runs past the block's end.
        let first_cold = cold(6);
        let mut checked = take(&NEXT_B, &other, 3);
        let _ = cold(9);
        checked.extend(take(&NEXT_B, &other, 8));
        assert_eq!(checked, plain, "the live ids, with and without checks");
        assert!(
            first_cold.iter().all(|id| !checked.contains(id)),
            "a fork's ids are never live ones"
        );
    }

    /// A slot a fork reused, and one it added, are free after it under a
    /// later generation: the fork's ids never name what is made live
    /// after it (Codex, F1 review 1).
    #[test]
    fn a_forks_slab_ids_never_alias_a_later_entry() {
        use super::{ForkSlab, Logged};
        let made = |fork: bool| {
            let mut slab = ForkSlab::<u8>::default();
            let gone = slab.insert(0);
            slab.remove(gone);
            let forked = fork.then(|| {
                let _cold = ColdFrame::begin();
                slab.begin();
                let reused = slab.insert(1);
                let added = slab.insert(2);
                // As a check's end does: drained, then rolled back.
                drop(slab.drain());
                slab.rollback();
                [reused, added]
            });
            let later: Vec<_> = (0..3).map(|v| slab.insert(v)).collect();
            (slab, gone, forked, later)
        };
        let (mut slab, gone, forked, later) = made(true);
        for fork in forked.into_iter().flatten().chain([gone]) {
            assert!(!slab.contains(fork));
            assert!(slab.remove(fork).is_none(), "the fork's id takes nothing out");
        }
        assert!(later.iter().all(|&id| slab.contains(id)), "the later entries stay");
        assert_eq!(
            later,
            made(false).3,
            "live code hands out the ids it would have without the fork"
        );
    }

    /// Codex 1 (F1 review 6): an element's event handler and a layer's
    /// dismiss handler are callers' closures, gated where they are called:
    /// a fork (a check, input replayed in one) runs neither, and a live
    /// call does.
    #[test]
    fn a_fork_calls_no_event_or_dismiss_handler() {
        let clicks = Rc::new(Cell::new(0_u32));
        let c = Rc::clone(&clicks);
        let el = crate::div().on_click(move |_| c.set(c.get() + 1));
        let click = el.common.handlers.click.clone().expect("a click handler");
        let asked = Rc::new(Cell::new(0_u32));
        let a = Rc::clone(&asked);
        let layer = crate::Layer::new().on_dismiss(move |_| {
            a.set(a.get() + 1);
            crate::Dismiss::Close
        });
        let event = crate::events::ClickEvent {
            x: 0.0,
            y: 0.0,
            bounds: crate::geometry::Bounds::default(),
            modifiers: crate::events::Modifiers::default(),
        };
        let reason = crate::DismissReason::Escape;
        {
            let _cold = ColdFrame::begin();
            click(&event);
            assert_eq!(layer.ask(&reason), None, "no one asked in a fork");
        }
        assert_eq!((clicks.get(), asked.get()), (0, 0), "neither ran in the fork");
        click(&event);
        assert_eq!(layer.ask(&reason), Some(crate::Dismiss::Close));
        assert_eq!((clicks.get(), asked.get()), (1, 1), "both run live");
    }

    /// Opus nit 2 (F1 review 7), F1 review 8: a handler whose answer
    /// decides what the engine does next (consumed) has no answer a fork
    /// can take that keeps it on live's path: dispatched in a fork, it
    /// dooms the fork, which is dropped (its result thrown away, never
    /// taken with a default answer, ADR-0028); it runs on to its end. Live,
    /// it answers as the caller.
    #[test]
    fn a_fork_dispatching_a_consumed_flag_is_dropped() {
        let el = crate::div().on_key_down(|_| true);
        let key = el.common.handlers.key_down.clone().expect("a key handler");
        let event = crate::events::KeyEvent {
            key: crate::events::Key::Named(crate::events::NamedKey::Enter),
            text: None,
            modifiers: crate::events::Modifiers::default(),
        };
        let cold = ColdFrame::begin();
        key(&event);
        assert_eq!(super::doomed(), super::Doom::Dropped, "doomed at the site");
        assert!(super::forking(), "and runs on to its end");
        assert!(
            matches!(cold.close(Ok(())), super::Ended::Dropped(_)),
            "its result is dropped"
        );
        assert_eq!(super::doomed(), super::Doom::Kept, "nothing doomed live");
        assert!(key(&event), "live, the handler answers");
    }

    /// Something that dooms the fork as it drops (a destructor reaching a
    /// site that cannot answer inside one).
    struct DropsTheFork;

    impl super::FreshValue for DropsTheFork {}

    impl Drop for DropsTheFork {
        fn drop(&mut self) {
            super::drop_fork("a test destructor");
        }
    }

    /// A result whose drop writes forked state.
    struct WritesAsItDrops;

    impl Drop for WritesAsItDrops {
        fn drop(&mut self) {
            DROP_WRITES.set(DROP_WRITES.get().wrapping_add(1));
        }
    }

    crate::engine_local! {
        /// What the fork's drain drops ([`DropsTheFork`]).
        [Fresh] static DOOMS_AS_DRAINED: std::cell::RefCell<Vec<DropsTheFork>> = std::cell::RefCell::new(Vec::new());
        /// The writes of [`WritesAsItDrops`].
        [Fork] static DROP_WRITES: Cell<u32> = const { Cell::new(0) };
    }

    /// Codex 1 (F1 review 9), as it stands with no rerun: a destructor in
    /// the fork's drain dooms it (dropped) after its closure returned. The
    /// verdict is read once the drain is done, so the result is dropped
    /// inside the fork (its write put back), never live.
    #[test]
    fn a_fork_its_drain_dooms_drops_its_result_inside_it() {
        let failed = std::panic::catch_unwind(|| {
            super::in_cold_frame(|| {
                DOOMS_AS_DRAINED.with_borrow_mut(|d| d.push(DropsTheFork));
                WritesAsItDrops
            })
        });
        assert!(failed.is_err(), "a dropped fork gives no result");
        assert!(!is_cold(), "the fork ended");
        assert_eq!(DROP_WRITES.get(), 0, "the result was dropped inside the fork");
    }

    crate::engine_local! {
        /// A Shared piece that makes a node and takes an id as it is made.
        [Shared("a test of a Shared piece a fork touches first")]
        static FIRST_TOUCHED: (Signal<u32>, u64) = (Signal::new(7), super::unique_id(&TOUCHED));
    }

    /// The ids [`FIRST_TOUCHED`] and the code around it take.
    static TOUCHED: super::IdKind = super::IdKind::new("first touched");

    /// F1 review 9 (ADR-0028, research-f1): a fork never runs live code. A
    /// Shared piece it touches first is a stand-in of its own (made in the
    /// fork, the same one each touch, gone with it), and the fork's result
    /// stands: no doom, no rerun, nothing made live as it ends. Live code
    /// makes the piece at its own first touch, so what live code holds
    /// after it, the piece's node and id and the next ones made, is what a
    /// run with no check holds (each run on a thread of its own: the same
    /// arena to start from, and ids counted from the thread's first).
    #[test]
    fn a_shared_piece_a_check_touches_first_is_a_stand_in_and_made_at_the_first_live_touch() {
        type Seen = ((usize, u32), u64, (usize, u32), u64);
        let run = |check: bool| -> Seen {
            std::thread::spawn(move || {
                let freed = Signal::new(0_u8);
                freed.dispose();
                let first = super::unique_id(&TOUCHED);
                if check {
                    let cold = ColdFrame::begin();
                    let _forks = Signal::new(1_u8);
                    let stand_in = FIRST_TOUCHED.with(|p| *p);
                    assert!(stand_in.0.made_in_fork(), "the fork's stand-in");
                    assert_eq!(stand_in.0.get_untracked(), 7);
                    assert_eq!(
                        FIRST_TOUCHED.with(|p| p.0),
                        stand_in.0,
                        "the same one again in the fork"
                    );
                    assert_eq!(super::doomed(), super::Doom::Kept, "its result stands");
                    assert!(
                        matches!(cold.close(Ok(())), super::Ended::Kept(())),
                        "its result stands"
                    );
                    assert!(!stand_in.0.is_alive(), "gone with the fork");
                    let kept = super::in_cold_frame(|| FIRST_TOUCHED.with(|p| *p));
                    assert!(
                        kept.0.made_in_fork() && !kept.0.is_alive(),
                        "a second fork's stand-in, gone with it"
                    );
                }
                let piece = FIRST_TOUCHED.with(|p| *p);
                assert!(!piece.0.made_in_fork() && piece.0.is_alive(), "made live, here");
                let next = (Signal::new(3_u8).key(), super::unique_id(&TOUCHED));
                (
                    piece.0.key(),
                    piece.1.wrapping_sub(first),
                    next.0,
                    next.1.wrapping_sub(first),
                )
            })
            .join()
            .expect("a run")
        };
        assert_eq!(
            run(true),
            run(false),
            "the live keys and ids, with and without the check"
        );
    }

    crate::engine_local! {
        /// A Shared piece whose init touches another first: it holds the
        /// other's handle (it reads no value, R4) and one of its own.
        [Shared("a test of nested first touches")]
        static NESTED_OUTER: (Signal<u8>, Signal<u8>) = (NESTED_INNER.with(|s| *s), Signal::new(2));
        [Shared("a test of nested first touches")]
        static NESTED_INNER: Signal<u8> = Signal::new(1);
        /// A Shared piece whose init reads whether a fork runs.
        [Shared("a test of a Shared init that reads forked state")]
        static READS_FORKING: bool = super::forking();
        /// A forked cell, and a Shared piece whose init reads it.
        [Fork] static FORKED_FLAG: Cell<bool> = const { Cell::new(false) };
        [Shared("a test of a Shared init that reads forked state")]
        static READS_FORK_CELL: bool = FORKED_FLAG.get();
    }

    /// The text of a panic.
    fn said(p: &(dyn std::any::Any + Send)) -> String {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default()
    }

    /// Codex 5 and 6 (F1 review 9), rewritten for the rule that replaced
    /// the rerun: a Shared init reads no state a fork changes, neither
    /// whether a fork runs nor a `[Fork]` cell (its value would differ in
    /// a fork and live, so a stand-in would not be what live code makes).
    /// Live and in a fork, the read panics, naming the piece.
    #[test]
    fn a_shared_init_reading_forked_state_panics_naming_the_piece() {
        std::thread::spawn(|| {
            type Touch = (&'static str, fn() -> bool);
            let touches: [Touch; 2] = [
                ("READS_FORKING", || READS_FORKING.with(|v| *v)),
                ("READS_FORK_CELL", || READS_FORK_CELL.with(|v| *v)),
            ];
            for (piece, touch) in touches {
                let live = std::panic::catch_unwind(touch).expect_err("live, it panics");
                let forked = std::panic::catch_unwind(|| {
                    let _cold = ColdFrame::begin();
                    touch()
                })
                .expect_err("in a fork, it panics");
                for p in [live, forked] {
                    let said = said(&*p);
                    assert!(said.contains(piece) && said.contains("state a fork changes"), "{said}");
                }
            }
        })
        .join()
        .expect("the thread ends");
    }

    crate::engine_local! {
        /// A Shared piece whose init reaches the piece itself.
        [Shared("a test of a Shared init that reaches its own piece")]
        static SELF_REACH: u8 = SELF_REACH.with(|v| *v);
    }

    /// Opus nit (F1 review 9): an init that reaches its own piece panics,
    /// naming it, rather than recursing until the stack overflows: making
    /// a fork's stand-in, and live (where `OnceCell` tells reentrance only
    /// once the init returns, which it never does).
    #[test]
    fn a_shared_init_reaching_itself_panics_cleanly() {
        let text = |p: Box<dyn std::any::Any + Send>| said(&*p);
        let (forked, live) = std::thread::spawn(|| {
            let forked = std::panic::catch_unwind(|| {
                let _cold = ColdFrame::begin();
                SELF_REACH.with(|v| *v)
            })
            .expect_err("a panic in the fork");
            let live = std::panic::catch_unwind(|| SELF_REACH.with(|v| *v)).expect_err("a panic live");
            (forked, live)
        })
        .join()
        .expect("the thread ends, not aborts");
        for said in [text(forked), text(live)] {
            assert!(said.contains("reaches the piece itself"), "{said}");
        }
    }

    /// F1 review 9: first touches nested in a fork (a Shared init that
    /// touches another Shared piece) give stand-ins of both, consistent
    /// with each other; the fork stands, and live code makes both at its
    /// own first touch.
    #[test]
    fn nested_first_touches_in_a_fork_are_stand_ins_and_made_live_later() {
        std::thread::spawn(|| {
            let seen = super::in_cold_frame(|| {
                let (inner, own) = NESTED_OUTER.with(|p| *p);
                assert!(
                    inner == NESTED_INNER.with(|s| *s),
                    "the outer holds the inner's stand-in"
                );
                inner.get_untracked().wrapping_add(own.get_untracked())
            });
            assert_eq!(seen, 3, "the fork stands, read through both stand-ins");
            let (inner, own) = NESTED_OUTER.with(|p| *p);
            assert!(!own.made_in_fork() && own.is_alive());
            assert!(NESTED_INNER.with(|s| !s.made_in_fork() && s.is_alive() && *s == inner));
            assert_eq!(inner.get_untracked().wrapping_add(own.get_untracked()), 3);
        })
        .join()
        .expect("a run");
    }

    /// Opus I1 (F1 review 8): an interaction handle's number is an id
    /// ([`super::unique_id`]): one a fork handed out is never handed out
    /// again live.
    #[test]
    fn a_forks_interaction_is_never_one_live_code_makes() {
        let forked = super::in_cold_frame(crate::Interaction::new);
        assert!(forked != crate::Interaction::new());
    }

    /// Codex 1 (F1 review 7): the one node identity that crosses threads
    /// (an instance the render thread carries) names nothing on another
    /// thread, though that thread's arena has a live node in the same slot
    /// and generation; the handles themselves are `!Send` (compile-fail
    /// tests on `Signal`).
    #[test]
    fn an_owner_id_names_nothing_on_another_thread() {
        let run = || {
            let scope = crate::reactive::Scope::detached();
            (scope, scope.id().owner_id())
        };
        let (there, here) = std::thread::spawn(move || {
            let (scope, id) = run();
            (scope.id().index, id)
        })
        .join()
        .expect("a thread");
        let (mine, same_slot) = run();
        assert_eq!(mine.id().index, there, "the same slot on this thread");
        assert!(crate::reactive::is_live(same_slot), "its own names its node");
        assert!(!crate::reactive::is_live(here), "the other thread's names none here");
        mine.dispose();
    }

    /// Codex nit (F1 review 5): a scroll handle's `on_layout` is the
    /// caller's closure, so a fork's layout pass never calls it (one gate,
    /// rule 5) and asks for no pass again; the live pass calls it.
    #[test]
    fn a_check_calls_no_scroll_layout_hook() {
        let ran = Rc::new(Cell::new(0_u32));
        let h = crate::ScrollHandle::new();
        let r = Rc::clone(&ran);
        h.on_layout(move |_| {
            r.set(r.get() + 1);
            true
        });
        h.laid_out(crate::ScrollLayout::default());
        {
            let _cold = ColdFrame::begin();
            assert!(!h.after_layout(None), "no pass asked for in a fork");
        }
        assert_eq!(ran.get(), 0, "the hook never ran in the fork");
        assert!(h.after_layout(None), "the live pass calls it");
        assert_eq!(ran.get(), 1);
        h.dispose();
    }

    /// A slot whose generations ran out is retired: never handed out again.
    #[test]
    fn a_slot_whose_generations_ran_out_is_retired() {
        let mut slab = super::ForkSlab::<u8>::default();
        let spent = slab.insert(0);
        slab.spend(spent);
        slab.remove(spent);
        let next = slab.insert(1);
        assert_ne!(next.slot, spent.slot, "a new slot");
        assert!(!slab.contains(spent));
    }

    /// Codex (F1 review 2): a slot one generation short of running out. A
    /// fork takes its last generation and rolls back, which puts it back on
    /// the free list; it is retired there, so a second fork and the live
    /// code never hand out that last generation again.
    #[test]
    fn a_slot_whose_last_generation_a_fork_took_stays_retired() {
        use super::{ForkSlab, Logged};
        let mut slab = ForkSlab::<u8>::default();
        let id = slab.insert(0);
        slab.set_next(id, super::SPENT - 1, super::SPENT - 1);
        slab.remove(id);
        let mut handed = Vec::new();
        for _ in 0..2 {
            let _cold = ColdFrame::begin();
            slab.begin();
            handed.push(slab.insert(1));
            drop(slab.drain());
            slab.rollback();
        }
        handed.push(slab.insert(2));
        // An id is its slot, generation and world (ADR-0034, S6).
        let unique: foldhash::HashSet<_> = handed.iter().collect();
        assert_eq!(unique.len(), handed.len(), "no id handed out twice: {handed:?}");
    }

    /// A fork costs what it touched (Codex, F1 review 1): with a million
    /// free slots, entering it, taking and freeing one, and putting it
    /// back copies nothing of the free list.
    #[test]
    fn a_fork_of_a_slab_with_a_million_free_slots_costs_what_it_touched() {
        use super::{ForkSlab, Logged};
        let mut slab = ForkSlab::<u8>::default();
        let ids: Vec<_> = (0..1_000_000).map(|_| slab.insert(0)).collect();
        for id in ids {
            slab.remove(id);
        }
        let _cold = ColdFrame::begin();
        let work = super::fork_work_done();
        let (made, bytes) = crate::heap::allocated_bytes(|| {
            slab.begin();
            let id = slab.insert(1);
            slab.remove(id);
            slab.rollback();
            id
        });
        assert!(bytes < 4 * 1024, "{bytes} bytes");
        assert!(
            super::fork_work_done() - work < 8,
            "{} pieces of work",
            super::fork_work_done() - work
        );
        assert!(!slab.contains(made));
        assert_eq!(slab.free.len(), 1_000_000, "the free slots as they were");
    }

    /// An effect made in a cold frame never runs, in it or after.
    #[test]
    fn an_effect_a_cold_frame_made_never_runs() {
        let ran = Rc::new(Cell::new(0_u32));
        let source = Signal::new(0_u8);
        {
            let _cold = ColdFrame::begin();
            let r = Rc::clone(&ran);
            let _effect = Effect::new(move || {
                source.get();
                r.set(r.get() + 1);
            });
        }
        source.set(1);
        assert_eq!(ran.get(), 0, "the cold frame's effect ran");
    }

    /// Text a cold frame lays out is shaped into its own caches: the live
    /// ones hold and count what they did, and the cold frame's text is
    /// not shaped for the live frame.
    #[test]
    fn cold_text_layout_leaves_the_live_text_caches_as_they_were() {
        let mut ts = crate::text::TextSystem::new();
        let t = crate::style::ResolvedText::root(crate::color::Color::BLACK);
        ts.measure("laid out live", &t, None);
        let held = ts.footprint();
        let live = ts.enter_cold();
        ts.measure("laid out cold", &t, None);
        ts.measure("laid out live", &t, None);
        assert_eq!(ts.take_shaped(), 2, "the cold frame shapes from nothing");
        ts.exit_cold(live);
        assert_eq!(ts.footprint(), held, "the live caches hold what they held");
        assert_eq!(ts.take_shaped(), 1, "and count what they counted");
        ts.measure("laid out cold", &t, None);
        assert_eq!(ts.take_shaped(), 1, "the cold text is shaped live anew");
    }

    /// Sets a timer and writes a signal as it drops.
    struct Busy(Signal<u32>);

    impl Drop for Busy {
        fn drop(&mut self) {
            crate::clock::after(Duration::from_millis(1), || {});
            self.0.set(7);
        }
    }

    /// What a cold frame made drops while the frame still runs: the timer
    /// its destructor sets and the signal it writes are the frame's, and
    /// go with it.
    #[test]
    fn what_a_cold_value_does_as_it_drops_stays_in_the_cold_frame() {
        crate::clock::script(Duration::ZERO);
        let live = Signal::new(1_u32);
        {
            let _cold = ColdFrame::begin();
            let busy = Busy(live);
            crate::clock::after(Duration::from_secs(1), move || {
                let _ = &busy;
            });
        }
        assert_eq!(live.get_untracked(), 1, "the live signal is unchanged");
        assert!(crate::clock::next_deadline().is_none(), "no live timer");
        assert!(!is_cold(), "no cold frame runs");
    }

    crate::engine_local! {
        /// Made, wrongly, from a component's context.
        [Shared("a test of what a Shared init may read")]
        static FROM_CONTEXT: Signal<u32> = {
            let made = Signal::new(0_u32);
            let read = crate::component::use_context::<u32>();
            made.set(read.unwrap_or(0));
            made
        };
    }

    /// A Shared piece whose init reads a component's context, made first
    /// in a cold frame, fails loudly, and keeps nothing it made.
    #[test]
    fn a_shared_init_reading_component_context_fails() {
        crate::component::provide(7_u32);
        let tried = {
            let _cold = ColdFrame::begin();
            let tried = std::panic::catch_unwind(|| FROM_CONTEXT.with(|s| *s));
            assert!(is_cold(), "the frame runs on after it");
            tried
        };
        let message = tried
            .err()
            .and_then(|p| p.downcast::<&str>().ok().map(|m| (*m).to_owned()).or(None))
            .expect("the init fails");
        assert_eq!(message, "a Shared engine value's init may not read component context");
        assert!(
            std::panic::catch_unwind(|| FROM_CONTEXT.with(|s| *s)).is_err(),
            "no value was kept: it is made, and fails, again"
        );
    }

    crate::engine_local! {
        /// Makes a signal, then fails.
        [Shared("a test of a Shared init that fails")]
        static FAILS: Signal<u32> = {
            let made = Signal::new(0_u32);
            if made.get_untracked() == 0 {
                std::panic::resume_unwind(Box::new("the init fails after making a signal"));
            }
            made
        };
    }

    /// A Shared init that makes a signal and then fails, tried again and
    /// again, live and in a cold frame: what each try made is undone, so
    /// the arena is as it was.
    #[test]
    fn a_shared_init_that_fails_leaves_the_arena_as_it_was() {
        let fails = || std::panic::catch_unwind(|| FAILS.with(|s| *s)).is_err();
        let (slots, live) = crate::reactive::arena();
        for _ in 0..100 {
            assert!(fails(), "the init fails");
        }
        let (slots_after, live_after) = crate::reactive::arena();
        assert_eq!(live_after, live, "no signal it made lives on");
        assert!(
            slots_after <= slots + 1,
            "its one slot is reused: {slots} -> {slots_after}"
        );
        let (slots, live) = crate::reactive::arena();
        {
            let _cold = ColdFrame::begin();
            for _ in 0..100 {
                assert!(fails(), "the init fails in a cold frame");
            }
        }
        assert_eq!(
            crate::reactive::arena(),
            (slots, live),
            "the cold frame's tries are undone with it"
        );
    }

    crate::engine_local! {
        /// Made, with no `detached` of its own, first under a scope.
        [Shared("a test of what owns a Shared init's signal")]
        static UNOWNED_LIVE: Signal<u32> = Signal::new(3_u32);
        /// The same, made first in a cold frame.
        [Shared("a test of what owns a Shared init's signal")]
        static UNOWNED_COLD: Signal<u32> = Signal::new(4_u32);
    }

    /// What a Shared init makes is owned by no scope, whatever scope first
    /// touched it, live or in a cold frame ([`live`] detaches it): the
    /// scope disposed, the signal lives on, so no init needs a
    /// `detached` of its own.
    #[test]
    fn a_shared_inits_signal_outlives_the_scope_that_first_touched_it() {
        let scope = crate::reactive::Scope::new();
        let live = scope.run(|| UNOWNED_LIVE.with(|s| *s));
        scope.dispose();
        assert_eq!(live.get_untracked(), 3, "made live, it lives on");
        let scope = crate::reactive::Scope::new();
        {
            let _cold = ColdFrame::begin();
            scope.run(|| UNOWNED_COLD.with(|s| *s));
        }
        scope.dispose();
        assert_eq!(
            UNOWNED_COLD.with(|s| *s).get_untracked(),
            4,
            "made in a cold frame, it lives on"
        );
    }

    /// How many times [`INNER`]'s init ran.
    static INNER_MADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    /// How many times [`OUTER`]'s init ran: its first fails.
    static OUTER_TRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    crate::engine_local! {
        /// Made by [`OUTER`]'s init.
        [Shared("a test of a Shared init within another")]
        static INNER: Signal<u32> = {
            INNER_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Signal::new(7_u32)
        };
        /// Makes a signal, then [`INNER`], then fails the first time (it
        /// reads no value: a Shared init reads none, R4).
        [Shared("a test of a Shared init within another")]
        static OUTER: Signal<u32> = {
            let made = Signal::new(0_u32);
            let _inner = INNER.with(|s| *s);
            if OUTER_TRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                std::panic::resume_unwind(Box::new("the outer init fails after the inner returned"));
            }
            made
        };
    }

    /// An init that makes another Shared piece and then fails: the inner
    /// one, returned, is made for good (live, and not made again); only
    /// what the outer one made itself is undone.
    #[test]
    fn a_shared_init_failing_after_one_within_it_returned_keeps_that_one() {
        let (_, live) = crate::reactive::arena();
        assert!(
            std::panic::catch_unwind(|| OUTER.with(|s| *s)).is_err(),
            "the outer init fails"
        );
        assert_eq!(
            crate::reactive::arena().1,
            live + 1,
            "the inner signal lives; the outer's is gone"
        );
        let inner = INNER.with(|s| *s);
        assert_eq!(inner.get_untracked(), 7, "the inner signal is usable");
        inner.set(8);
        assert_eq!(inner.get_untracked(), 8);
        assert!(
            std::panic::catch_unwind(|| OUTER.with(|s| *s)).is_ok(),
            "tried again, the outer one is made"
        );
        assert_eq!(
            INNER_MADE.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the inner one was made once"
        );
    }

    /// The fork counts what it copies: a live owner the frame writes is
    /// copied with its list of what it owns, and counted so.
    #[test]
    fn copying_a_live_owner_counts_what_it_owns() {
        let scope = crate::reactive::Scope::new();
        scope.run(|| {
            for _ in 0..1_000 {
                let _ = Signal::new(0_u8);
            }
        });
        // A live signal a live effect reads: a reader the frame adds links
        // after that effect's link, which it writes (a live link copied).
        let read = Signal::new(0_u8);
        let effect = crate::reactive::Effect::new(move || {
            read.get();
        });
        let done = {
            let _cold = ColdFrame::begin();
            // What the frame copies, not what entering the pieces did (a
            // `[Fork]` cell's O(1) copy each).
            let before = super::fork_work_done();
            scope.run(|| Signal::new(1_u8));
            let reader = crate::reactive::Computed::new(move || read.get());
            reader.get();
            super::fork_work_done() - before
        };
        // The owner and its list of 1,000; the signal read; the effect's link.
        assert_eq!(done, 1_001 + 1 + 1, "the owner, its 1,000, and the live link written");
        effect.dispose();
        scope.dispose();
    }

    /// Sets a timer that owns another of itself as it drops, and writes a
    /// signal: dropped, it makes more, forever.
    struct Again(Signal<u32>);

    impl Drop for Again {
        fn drop(&mut self) {
            let again = Again(self.0);
            crate::clock::after(Duration::from_millis(1), move || {
                let _ = &again;
            });
            self.0.set(self.0.get_untracked().wrapping_add(1));
        }
    }

    /// A destructor that keeps scheduling work fails the cold check, naming
    /// what its drops wrote, and nothing of it reaches the live state.
    #[test]
    fn a_cold_value_that_makes_more_forever_as_it_drops_fails_the_check() {
        crate::clock::script(Duration::ZERO);
        let live = Signal::new(1_u32);
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _cold = ColdFrame::begin();
            let again = Again(live);
            crate::clock::after(Duration::from_secs(1), move || {
                let _ = &again;
            });
        }));
        let message = failed
            .err()
            .and_then(|p| p.downcast::<String>().ok())
            .expect("the check fails");
        assert!(message.contains("schedules work again"), "{message}");
        assert!(
            message.contains("TIMERS (") && message.contains("Queue"),
            "names the piece and its type: {message}"
        );
        assert_eq!(live.get_untracked(), 1, "the live signal is unchanged");
        assert!(crate::clock::next_deadline().is_none(), "no live timer");
        assert!(!is_cold(), "no cold frame runs");
    }

    struct Boom;

    impl Drop for Boom {
        #[expect(clippy::panic, reason = "the destructor under test panics")]
        fn drop(&mut self) {
            panic!("a cold value's destructor panicked");
        }
    }

    /// A value only the cold frame made panics as it drops: the check
    /// fails stop (ADR-0034, S2): the live state is back all the same, the
    /// fault is reported, and no later check opens on the thread.
    #[test]
    fn a_panicking_cold_value_still_puts_the_live_state_back() {
        let kept = Signal::new(1_u32);
        // The runtime enters before the timers, and so is put back after.
        crate::clock::cancel_all();
        let ended = super::fork(|| {
            let _made = Signal::new(2_u32);
            let boom = Boom;
            crate::clock::after(Duration::from_secs(1), move || {
                let _ = &boom;
            });
        });
        let super::Ended::Faulted(fault) = ended else {
            panic!("the check faulted: {ended:?}");
        };
        assert_eq!(
            (fault.phase, fault.piece.as_str()),
            (super::frame::Phase::Disposal, "TIMERS")
        );
        assert!(!is_cold(), "no cold frame runs");
        assert_eq!(kept.get_untracked(), 1);
        let _live = Signal::new(3_u32);
        assert_eq!(crate::reactive::cold_steps(), 0, "the runtime logs no more");
        assert_eq!(crate::component::cold_tree_steps(), 0, "the tree logs no more");
        assert!(crate::clock::next_deadline().is_none(), "the cold timer is gone");
        assert!(matches!(super::fork(|| ()), super::Ended::Poisoned), "checks are off");
    }

    /// Fault injection over every user hook of a fork (F1 review 13,
    /// addendum): rounds 9 to 13 each found one more moment where user
    /// code (an init, a `Drop`, a `Hash`, `Eq` or `Clone`, a release, a
    /// panic payload's `Drop`) ran at a bad time. This enumerates them.
    /// Every hook of the user types below counts itself; the hook at index
    /// `k` injects one fault (a panic, a write of a `[Fork]` piece, a touch
    /// of a `[Shared]` piece, a read of a signal). Every scenario runs with
    /// the fault at each index in turn, on a thread of its own, and the
    /// invariants are checked after each run.
    mod faults {
        use super::super::{Ended, ForkMap, drop_fork, fork, is_cold, run_releases, when_live};
        use crate::reactive::Signal;
        use std::cell::{Cell, RefCell};
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering::Relaxed};

        /// Whether hooks count and may fault (off while a case sets up and
        /// checks).
        static ARMED: AtomicBool = AtomicBool::new(false);
        /// Hooks run while armed.
        static COUNT: AtomicU32 = AtomicU32::new(0);
        /// The hook index that faults.
        static TARGET: AtomicU32 = AtomicU32::new(u32::MAX);
        /// The fault: 0 panic, 1 write, 2 touch, 3 read.
        static FAULT: AtomicU8 = AtomicU8::new(0);
        /// The phase a panic fault fired in: 0 none, 1 the check's body, 4
        /// its teardown, 3 the live releases.
        static FIRED: AtomicU8 = AtomicU8::new(0);
        /// The phase any fault was injected in (a write, touch or read the
        /// engine refuses inside a Shared init, or a stand-in reached after
        /// its drop, panics loudly by design).
        static INJECTED: AtomicU8 = AtomicU8::new(0);
        /// 1 while the check's body runs, 4 while its teardown does, 3
        /// while the live releases run.
        static PHASE: AtomicU8 = AtomicU8::new(0);
        /// Hooks that ran inside the fork call once the fork had closed.
        static LATE: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

        const FAULTS: [&str; 4] = ["panic", "write", "touch", "read"];

        /// A panic payload with a `Drop` (`panic_any`): a hook too.
        struct Payload;

        impl Drop for Payload {
            fn drop(&mut self) {
                hook("payload drop");
            }
        }

        #[expect(clippy::panic, reason = "the harness injects a panic with a payload of its own")]
        fn hook(name: &'static str) {
            if !ARMED.load(Relaxed) {
                return;
            }
            let phase = PHASE.load(Relaxed);
            if (phase == 1 || phase == 4) && !is_cold() {
                LATE.lock().map(|mut l| l.push(name)).ok();
            }
            if COUNT.fetch_add(1, Relaxed) != TARGET.load(Relaxed) {
                return;
            }
            INJECTED.store(phase, Relaxed);
            match FAULT.load(Relaxed) {
                0 => {
                    FIRED.store(phase, Relaxed);
                    std::panic::panic_any(Payload);
                }
                1 => LIVE.set(LIVE.get().wrapping_add(100)),
                2 => SH.with(|_| ()),
                _ => drop(SIG.with(|s| s.get())),
            }
        }

        /// A user value whose every hook is one.
        struct Val(u32);

        impl crate::engine::FreshValue for Val {}
        crate::engine::fork_value_audited!(Val);

        impl Drop for Val {
            fn drop(&mut self) {
                hook("val drop");
            }
        }

        impl Clone for Val {
            fn clone(&self) -> Self {
                hook("val clone");
                Self(self.0)
            }
        }

        // A key is a sealed `ForkKey` (ADR-0034, S7): its hash and compare
        // are gui's, so only values carry hooks.

        /// A Shared piece's value, with a `Drop` (a stand-in drops it).
        struct Mark;

        impl Drop for Mark {
            fn drop(&mut self) {
                hook("shared drop");
            }
        }

        crate::engine_local! {
            [Fork] static LIVE: Cell<u32> = Cell::new(1);
            [Fresh] static FRESH: RefCell<Vec<Val>> = {
                hook("fresh init");
                RefCell::new(Vec::new())
            };
            [Fork(log)] static MAP: RefCell<ForkMap<u32, Val>> = RefCell::default();
            [Shared("the harness's marker: made first in a fork, a stand-in")] static SH: Mark = {
                hook("shared init");
                Mark
            };
            [Shared("the harness's signal handle, made live")] static SIG: Signal<u32> = Signal::new(0);
        }

        #[derive(Clone, Copy, Debug)]
        struct Scenario {
            kept: bool,
            body_panics: bool,
            /// 0 none, 1 insert a new key, 2 overwrite one, 3 remove one.
            map: u8,
            /// Bit 0 a `[Fresh]` piece, 1 a `[Fork]` one, 2 a `[Shared]` one.
            touch: u8,
            release: bool,
        }

        fn scenarios() -> Vec<Scenario> {
            let mut all = Vec::new();
            for kept in [true, false] {
                for body_panics in [false, true] {
                    for map in 0..4 {
                        for touch in 0..8 {
                            for release in [false, true] {
                                all.push(Scenario {
                                    kept,
                                    body_panics,
                                    map,
                                    touch,
                                    release,
                                });
                            }
                        }
                    }
                }
            }
            all
        }

        /// The live state the invariants compare: `LIVE`, the map's
        /// entries, the `[Fresh]` piece's values.
        fn live_state() -> (u32, Vec<(u32, u32)>, Vec<u32>) {
            let mut map: Vec<(u32, u32)> = MAP.with_borrow(|m| m.iter().map(|(k, v)| (*k, v.0)).collect());
            map.sort_unstable();
            (
                LIVE.get(),
                map,
                FRESH.with(|f| f.borrow().iter().map(|v| v.0).collect()),
            )
        }

        fn said(p: &(dyn std::any::Any + Send)) -> Option<String> {
            p.downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| p.downcast_ref::<String>().cloned())
        }

        /// One case: `s` with the fault `fault` at hook `target` (none if
        /// `u32::MAX`). How many hooks ran, or what broke.
        fn run(s: Scenario, target: u32, fault: u8) -> Result<u32, String> {
            std::thread::spawn(move || {
                ARMED.store(false, Relaxed);
                COUNT.store(0, Relaxed);
                TARGET.store(target, Relaxed);
                FAULT.store(fault, Relaxed);
                FIRED.store(0, Relaxed);
                INJECTED.store(0, Relaxed);
                LATE.lock().map(|mut l| l.clear()).ok();
                // Live set-up: the map holds two entries, the `[Fresh]`
                // piece one value, the signal is made live.
                LIVE.set(1);
                MAP.with_borrow_mut(|m| {
                    m.replace(1, Val(10));
                    m.replace(2, Val(20));
                });
                FRESH.with_borrow_mut(|f| f.push(Val(5)));
                SIG.with(|_| ());
                let before = live_state();
                PHASE.store(1, Relaxed);
                ARMED.store(true, Relaxed);
                /// The body ended, on return or unwind: the teardown runs.
                struct BodyEnds;
                impl Drop for BodyEnds {
                    fn drop(&mut self) {
                        PHASE.store(4, Relaxed);
                    }
                }
                let out = std::panic::catch_unwind(|| {
                    fork(|| {
                        let _ends = BodyEnds;
                        if s.touch & 1 != 0 {
                            FRESH.with_borrow_mut(|f| f.push(Val(6)));
                        }
                        if s.touch & 2 != 0 {
                            LIVE.set(2);
                        }
                        if s.touch & 4 != 0 {
                            SH.with(|_| ());
                        }
                        MAP.with_borrow_mut(|m| match s.map {
                            // A user value is replace-only (S7): a check
                            // never copies it, so its clone never runs.
                            1 => m.replace(3, Val(30)),
                            2 => m.replace(1, Val(11)),
                            3 => m.forget(&2),
                            _ => {}
                        });
                        if s.release {
                            when_live(|| hook("release"));
                        }
                        if !s.kept {
                            drop_fork("the harness");
                        }
                        assert!(!s.body_panics, "body");
                        Val(99)
                    })
                });
                ARMED.store(false, Relaxed);
                PHASE.store(2, Relaxed);
                let late: Vec<&str> = LATE.lock().map(|l| l.clone()).unwrap_or_default();
                let (fired, injected) = (FIRED.load(Relaxed), INJECTED.load(Relaxed));
                let mut broke = Vec::new();
                if !late.is_empty() {
                    broke.push(format!("user code ran after the switch: {late:?}"));
                }
                // ADR-0034's outcomes, each with its exact cause: the body's
                // own panic (or an injected one in the body) is recovered;
                // a teardown fault fails stop; a clean run keeps or drops.
                let faulted = matches!(&out, Ok(Ended::Faulted(_)));
                match &out {
                    Ok(Ended::Kept(v)) => {
                        if !s.kept || s.body_panics || fired != 0 || v.0 != 99 {
                            broke.push(String::from("a result kept that may not be"));
                        }
                    }
                    Ok(Ended::Dropped(_)) => {
                        if s.kept || s.body_panics || fired != 0 {
                            broke.push(String::from("a panic lost, or a kept result dropped"));
                        }
                    }
                    Ok(Ended::Panicked(m)) => {
                        if injected != 1 && !(s.body_panics && m == "body") {
                            broke.push(format!("the wrong panic won: {m}"));
                        }
                    }
                    Ok(Ended::Faulted(f)) => {
                        if injected != 4 {
                            broke.push(format!("a fault from nowhere: {f}"));
                        }
                        if s.body_panics && injected != 1 && f.body.as_deref() != Some("body") {
                            broke.push(format!("the body's panic lost in the fault: {f}"));
                        }
                    }
                    Ok(Ended::Poisoned) => broke.push(String::from("the first check refused as poisoned")),
                    Ok(Ended::Cancelled) => broke.push(String::from("a check no one cancelled was")),
                    Err(p) => broke.push(format!("a panic left the check: {:?}", said(&**p))),
                }
                if fired == 4 && !faulted {
                    broke.push(String::from("a teardown panic did not fault the check"));
                }
                // A kept result is the caller's: its drop is the caller's,
                // unarmed (its hook does nothing).
                drop(out);
                if live_state() != before {
                    broke.push(format!("live state {:?}, was {before:?}", live_state()));
                }
                if is_cold() {
                    broke.push(String::from("a fork still open"));
                }
                // The live releases: live work, after the hand-over.
                PHASE.store(3, Relaxed);
                ARMED.store(true, Relaxed);
                let released = std::panic::catch_unwind(run_releases);
                ARMED.store(false, Relaxed);
                if FIRED.load(Relaxed) == 3 && released.is_ok() {
                    broke.push(String::from("a release's panic lost"));
                }
                drop(released);
                // The next check opens and closes, and changes nothing; after
                // a fault, none opens (S2).
                LIVE.set(1);
                let next = std::panic::catch_unwind(|| fork(|| LIVE.set(7)));
                let as_due = if faulted {
                    matches!(next, Ok(Ended::Poisoned))
                } else {
                    matches!(next, Ok(Ended::Kept(())))
                };
                if !as_due || LIVE.get() != 1 {
                    broke.push(String::from("the next check is not as due"));
                }
                if broke.is_empty() {
                    Ok(COUNT.load(Relaxed))
                } else {
                    Err(broke.join("; "))
                }
            })
            .join()
            .unwrap_or_else(|p| Err(format!("the case itself panicked: {:?}", said(&*p))))
        }

        #[test]
        fn every_fault_at_every_user_hook_leaves_the_live_state_whole() {
            let mut cases = 0_usize;
            let mut found = Vec::new();
            for s in scenarios() {
                let hooks = match run(s, u32::MAX, 0) {
                    Ok(n) => n,
                    Err(e) => {
                        found.push(format!("{s:?} with no fault: {e}"));
                        continue;
                    }
                };
                cases += 1;
                for target in 0..hooks {
                    for (fault, name) in FAULTS.iter().enumerate() {
                        cases += 1;
                        let fault = u8::try_from(fault).unwrap_or(0);
                        if let Err(e) = run(s, target, fault) {
                            found.push(format!("{s:?}, {name} at hook {target}: {e}"));
                        }
                    }
                }
            }
            eprintln!("fork fault injection: {cases} cases, {} found", found.len());
            assert!(
                found.is_empty(),
                "{} of {cases} cases broke:\n{}",
                found.len(),
                found.join("\n")
            );
        }
    }
}
