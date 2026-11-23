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
//! [`ColdFrame`] enters every registered piece and, dropped, restores them
//! all before dropping what the frame made, so a value whose `Drop` panics
//! leaves the live state whole; the panic goes on after.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::sync::atomic::AtomicUsize;

/// What a cold frame does to one piece of state: `enter` gives what to put
/// back; `drain` takes what the frame made out of the piece while the frame
/// still runs (given what `enter` gave), so it drops still isolated; `exit`
/// puts the live state back and gives what is left of the frame's.
#[derive(Clone, Copy)]
pub struct Entry {
    pub name: &'static str,
    pub enter: fn() -> Box<dyn Any>,
    pub drain: fn(&dyn Any) -> Box<dyn Any>,
    pub exit: fn(Box<dyn Any>) -> Box<dyn Any>,
}

/// The cell an [`engine_local!`] holds, and whether it was registered.
pub struct Held<T> {
    value: T,
    #[cfg_attr(
        not(debug_assertions),
        expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
    )]
    registered: Cell<bool>,
}

impl<T> Held<T> {
    const fn new(value: T) -> Self {
        Self {
            value,
            registered: Cell::new(false),
        }
    }
}

/// A piece of per-thread engine state ([`engine_local!`]): used as a
/// `LocalKey` is (`with`, and `get`/`set`/`replace` and `with_borrow_mut`
/// for cells).
pub struct EngineKey<T: 'static> {
    slot: AtomicUsize,
    init: fn() -> T,
    entry: Option<Entry>,
}

impl<T: 'static> EngineKey<T> {
    #[must_use]
    pub const fn new(init: fn() -> T, entry: Option<Entry>) -> Self {
        Self {
            slot: AtomicUsize::new(gui_tls::UNSET),
            init,
            entry,
        }
    }

    #[expect(
        clippy::panic,
        reason = "more engine state than gui-tls holds, or a slot of another type: a bug in gui"
    )]
    fn held<'a>(&'static self, slot: Option<&'a gui_tls::Slot>) -> &'a Held<T> {
        let Some(slot) = slot else {
            panic!("more than {} pieces of engine state", gui_tls::CAPACITY);
        };
        let any = slot.get_or_init(|| Box::new(Held::new((self.init)())));
        let Some(held) = any.downcast_ref::<Held<T>>() else {
            panic!("an engine state slot holds another type");
        };
        held
    }

    #[inline]
    fn registered(&'static self, h: &Held<T>) {
        #[cfg(debug_assertions)]
        if let Some(e) = self.entry
            && !h.registered.replace(true)
        {
            register(e, gui_tls::slot_of(&self.slot), std::any::type_name::<T>());
        }
        #[cfg(not(debug_assertions))]
        let _ = (h, self.entry);
    }

    pub fn with<R>(&'static self, f: impl FnOnce(&T) -> R) -> R {
        gui_tls::with(gui_tls::slot_of(&self.slot), |slot| {
            let h = self.held(slot);
            self.registered(h);
            f(&h.value)
        })
    }

    /// [`Self::with`], unless the thread is ending.
    ///
    /// # Errors
    /// The thread's state is gone.
    pub fn try_with<R>(&'static self, f: impl FnOnce(&T) -> R) -> Result<R, std::thread::AccessError> {
        gui_tls::try_with(gui_tls::slot_of(&self.slot), |slot| {
            let h = self.held(slot);
            self.registered(h);
            f(&h.value)
        })
    }
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

/// A cell whose value a cold frame swaps ([`Fresh`](engine_local!) and
/// [`Fork`](engine_local!)).
pub trait Slot: 'static {
    type Inner: 'static;
    fn put(&self, value: Self::Inner) -> Self::Inner;
    fn into_value(self) -> Self::Inner;
}

impl<T: 'static> Slot for Cell<T> {
    type Inner = T;
    fn put(&self, value: T) -> T {
        self.replace(value)
    }
    fn into_value(self) -> T {
        self.into_inner()
    }
}

impl<T: 'static> Slot for RefCell<T> {
    type Inner = T;
    fn put(&self, value: T) -> T {
        self.replace(value)
    }
    fn into_value(self) -> T {
        self.into_inner()
    }
}

/// A cell a plain Fork copies whole, in O(1): a `Copy` value, or one
/// handle (`Rc`, `Arc`). A collection is never one: it keeps an undo log
/// ([`Logged`], `[Fork(log)]`), so a check costs what the frame writes.
pub trait Snapshot: Slot {
    fn copy(value: &Self::Inner) -> Self::Inner;
    fn snapshot(&self) -> Self::Inner;
}

impl<T: Copy + 'static> Snapshot for Cell<T> {
    fn copy(value: &T) -> T {
        *value
    }
    fn snapshot(&self) -> T {
        self.get()
    }
}

impl<T: ?Sized + 'static> Snapshot for RefCell<Option<std::rc::Rc<T>>> {
    fn copy(value: &Option<std::rc::Rc<T>>) -> Option<std::rc::Rc<T>> {
        value.clone()
    }
    fn snapshot(&self) -> Option<std::rc::Rc<T>> {
        self.borrow().clone()
    }
}

impl<T: ?Sized + 'static> Snapshot for RefCell<Option<std::sync::Arc<T>>> {
    fn copy(value: &Option<std::sync::Arc<T>>) -> Option<std::sync::Arc<T>> {
        value.clone()
    }
    fn snapshot(&self) -> Option<std::sync::Arc<T>> {
        self.borrow().clone()
    }
}

/// State a cold frame forks by an undo log (`[Fork(log)]`): what the frame
/// overwrote is kept on its first write, and put back at its end.
pub trait Logged: 'static {
    /// The frame starts: the log with it.
    fn begin(&mut self);
    /// What the frame made, taken out while it still runs.
    fn drain(&mut self) -> Box<dyn Any>;
    /// The live state back; what was left of the frame's.
    fn rollback(&mut self) -> Box<dyn Any>;
}

/// A [`ForkMap`]'s undo log: each key written as it was, and the keys
/// written.
#[cfg(debug_assertions)]
type MapLog<K, V> = (Vec<(K, Option<V>)>, foldhash::HashSet<K>);

/// A map a cold frame forks by an undo log: a key's value as it was is
/// kept on the frame's first write of the key, so the fork costs the keys
/// written ([`fork_work`]).
pub struct ForkMap<K, V> {
    map: foldhash::HashMap<K, V>,
    #[cfg(debug_assertions)]
    log: Option<MapLog<K, V>>,
}

impl<K, V> Default for ForkMap<K, V> {
    fn default() -> Self {
        Self {
            map: foldhash::HashMap::default(),
            #[cfg(debug_assertions)]
            log: None,
        }
    }
}

impl<K: Eq + std::hash::Hash + Clone + 'static, V: Clone + 'static> ForkMap<K, V> {
    #[must_use]
    pub fn get(&self, k: &K) -> Option<&V> {
        self.map.get(k)
    }

    #[must_use]
    pub fn contains_key(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.map.iter()
    }

    pub fn insert(&mut self, k: K, v: V) -> Option<V> {
        self.note(&k);
        self.map.insert(k, v)
    }

    pub fn remove(&mut self, k: &K) -> Option<V> {
        self.note(k);
        self.map.remove(k)
    }

    /// In a cold frame, `k` as it was, kept on its first write.
    #[inline]
    fn note(&mut self, k: &K) {
        #[cfg(debug_assertions)]
        if let Some((undo, seen)) = self.log.as_mut()
            && seen.insert(k.clone())
        {
            undo.push((k.clone(), self.map.get(k).cloned()));
            fork_work(1);
        }
        #[cfg(not(debug_assertions))]
        let _ = k;
    }
}

impl<K: Eq + std::hash::Hash + Clone + 'static, V: Clone + 'static> Logged for ForkMap<K, V> {
    fn begin(&mut self) {
        #[cfg(debug_assertions)]
        {
            self.log = Some((Vec::new(), foldhash::HashSet::default()));
        }
    }

    fn drain(&mut self) -> Box<dyn Any> {
        #[cfg(debug_assertions)]
        if let Some((undo, _)) = self.log.as_ref() {
            let made: Vec<V> = undo.iter().filter_map(|(k, _)| self.map.remove(k)).collect();
            return Box::new(made);
        }
        Box::new(())
    }

    fn rollback(&mut self) -> Box<dyn Any> {
        #[cfg(debug_assertions)]
        if let Some((undo, _)) = self.log.take() {
            let mut made = Vec::with_capacity(undo.len());
            for (k, was) in undo.into_iter().rev() {
                let now = match was {
                    Some(v) => self.map.insert(k, v),
                    None => self.map.remove(&k),
                };
                made.extend(now);
            }
            return Box::new(made);
        }
        Box::new(())
    }
}

/// Fresh: a new value in, the live one kept to put back.
pub fn fresh_enter<S: Slot>(slot: &S, fresh: S) -> Box<dyn Any> {
    Box::new(slot.put(fresh.into_value()))
}

/// Fresh: what the frame put in taken out, a new value in its place.
pub fn fresh_drain<S: Slot>(slot: &S, fresh: S) -> Box<dyn Any> {
    Box::new(slot.put(fresh.into_value()))
}

/// Fork: a copy of the live value kept to put back.
pub fn fork_enter<S: Snapshot>(slot: &S) -> Box<dyn Any> {
    fork_work(1);
    Box::new(slot.snapshot())
}

/// Fork: the frame's value taken out, a copy of the live one in its place.
pub fn fork_drain<S: Snapshot>(slot: &S, saved: &dyn Any) -> Box<dyn Any> {
    match saved.downcast_ref::<S::Inner>() {
        Some(live) => Box::new(slot.put(S::copy(live))),
        None => Box::new(()),
    }
}

/// Put back what enter kept, giving the frame's value.
pub fn put_back<S: Slot>(slot: &S, saved: Box<dyn Any>) -> Box<dyn Any> {
    match saved.downcast::<S::Inner>() {
        Ok(live) => Box::new(slot.put(*live)),
        Err(other) => other,
    }
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
/// - **Shared**: neither, with its reason; made the first time outside
///   any cold frame ([`live`]), so what it makes is live.
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
            #[cfg(debug_assertions)]
            fn enter() -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::fresh_enter(v, init()))
            }
            #[cfg(debug_assertions)]
            fn drain(_: &dyn ::std::any::Any) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::fresh_drain(v, init()))
            }
            #[cfg(debug_assertions)]
            fn exit(saved: Box<dyn ::std::any::Any>) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::put_back(v, saved))
            }
            #[cfg(debug_assertions)]
            let entry = Some($crate::engine::Entry { name: stringify!($name), enter, drain, exit });
            #[cfg(not(debug_assertions))]
            let entry = None;
            $crate::engine::EngineKey::new(init, entry)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            #[cfg(debug_assertions)]
            fn enter() -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::fork_enter(v))
            }
            #[cfg(debug_assertions)]
            fn drain(saved: &dyn ::std::any::Any) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::fork_drain(v, saved))
            }
            #[cfg(debug_assertions)]
            fn exit(saved: Box<dyn ::std::any::Any>) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::put_back(v, saved))
            }
            #[cfg(debug_assertions)]
            let entry = Some($crate::engine::Entry { name: stringify!($name), enter, drain, exit });
            #[cfg(not(debug_assertions))]
            let entry = None;
            $crate::engine::EngineKey::new(init, entry)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork(log)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            #[cfg(debug_assertions)]
            fn enter() -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::Logged::begin(&mut *v.borrow_mut()));
                Box::new(())
            }
            #[cfg(debug_assertions)]
            fn drain(_: &dyn ::std::any::Any) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::Logged::drain(&mut *v.borrow_mut()))
            }
            #[cfg(debug_assertions)]
            fn exit(_: Box<dyn ::std::any::Any>) -> Box<dyn ::std::any::Any> {
                $name.with(|v| $crate::engine::Logged::rollback(&mut *v.borrow_mut()))
            }
            #[cfg(debug_assertions)]
            let entry = Some($crate::engine::Entry { name: stringify!($name), enter, drain, exit });
            #[cfg(not(debug_assertions))]
            let entry = None;
            $crate::engine::EngineKey::new(init, entry)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Fork($enter:path, $drain:path, $exit:path)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $init
            }
            #[cfg(debug_assertions)]
            let entry = Some($crate::engine::Entry { name: stringify!($name), enter: $enter, drain: $drain, exit: $exit });
            #[cfg(not(debug_assertions))]
            let entry = None;
            $crate::engine::EngineKey::new(init, entry)
        };
        $crate::engine_local! { $($rest)* }
    };
    ($(#[$meta:meta])* [Shared($why:literal)] $vis:vis static $name:ident: $ty:ty = $init:expr; $($rest:tt)*) => {
        $(#[$meta])*
        #[doc = concat!("\n\nShared with a cold frame: ", $why)]
        $vis static $name: $crate::engine::EngineKey<$ty> = {
            fn init() -> $ty {
                $crate::engine::live(|| $init)
            }
            $crate::engine::EngineKey::new(init, None)
        };
        $crate::engine_local! { $($rest)* }
    };
}

#[doc(hidden)]
pub use crate::engine_local;

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
    cold: Option<Vec<(Entry, Box<dyn Any>)>>,
    /// How deep the frame is suspended: a Shared piece is being made.
    suspended: u32,
}

/// The registry itself: in a slot as the pieces are, entered by no frame.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
static REGISTRY: EngineKey<RefCell<Registry>> = EngineKey::new(|| RefCell::new(Registry::default()), None);

/// What the cold frames forked so far on this thread: a copy or an undo
/// entry each, over every Fork piece.
#[cfg_attr(
    not(debug_assertions),
    expect(dead_code, reason = "what a cold frame does: the cold check is debug only")
)]
static FORK_WORK: EngineKey<Cell<u64>> = EngineKey::new(|| Cell::new(0), None);

/// Count `n` copies or undo entries a cold frame's fork made.
#[inline]
pub fn fork_work(n: u64) {
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
/// frame runs (its live value is the one it starts with, kept to put back).
#[cfg(debug_assertions)]
fn register(e: Entry, slot: usize, ty: &'static str) {
    let cold = REGISTRY.with(|r| {
        let mut r = r.borrow_mut();
        r.entries.push(e);
        r.named.push((slot, e.name, ty));
        r.cold.is_some()
    });
    if cold {
        let saved = (e.enter)();
        REGISTRY.with(|r| {
            if let Some(c) = r.borrow_mut().cold.as_mut() {
                c.push((e, saved));
            }
        });
    }
}

/// How deep this thread is in making Shared pieces ([`live`]).
#[cfg(debug_assertions)]
static MAKING: EngineKey<Cell<u32>> = EngineKey::new(|| Cell::new(0), None);

/// A Shared piece being made, until it drops.
#[cfg(debug_assertions)]
struct Making;

#[cfg(debug_assertions)]
impl Making {
    fn enter() -> Self {
        MAKING.with(|m| m.set(m.get().saturating_add(1)));
        Self
    }
}

#[cfg(debug_assertions)]
impl Drop for Making {
    fn drop(&mut self) {
        MAKING.try_with(|m| m.set(m.get().saturating_sub(1))).ok();
    }
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
    if MAKING.try_with(Cell::get).unwrap_or(0) > 0 {
        panic!("a Shared engine value's init may not read component context");
    }
}

/// Make a Shared piece: `f` runs outside any cold frame (suspended around
/// it), so what it makes (a signal's handle) is the live arena's, never
/// one the frame undoes; owned by no scope and tracked by nothing (what
/// first touched it is not what it lives as long as); and as one step,
/// all of what it made undone if it panics.
pub fn live<R>(f: impl FnOnce() -> R) -> R {
    #[cfg(debug_assertions)]
    let _making = Making::enter();
    let f = || crate::reactive::making_shared(f);
    #[cfg(debug_assertions)]
    if is_cold() {
        /// The frame resumes however `f` ends.
        struct Resume;
        impl Drop for Resume {
            fn drop(&mut self) {
                REGISTRY
                    .try_with(|r| {
                        let mut r = r.borrow_mut();
                        r.suspended = r.suspended.saturating_sub(1);
                    })
                    .ok();
            }
        }
        REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            r.suspended = r.suspended.saturating_add(1);
        });
        let _resume = Resume;
        return crate::reactive::outside_cold(f);
    }
    crate::reactive::detached(f)
}

/// Run `f` as a cold frame does, every piece of engine state entered and
/// put back after: for tests of the per-thread state other crates declare.
#[cfg(debug_assertions)]
pub fn in_cold_frame<R>(f: impl FnOnce() -> R) -> R {
    let _frame = ColdFrame::begin();
    f()
}

/// Whether a cold frame runs on this thread (and is not suspended).
#[cfg(debug_assertions)]
pub(crate) fn is_cold() -> bool {
    REGISTRY
        .try_with(|r| {
            let r = r.borrow();
            r.cold.is_some() && r.suspended == 0
        })
        .unwrap_or(false)
}

/// How many times dropping what a cold frame made may make more before
/// the check fails ([`ColdFrame`]'s end).
#[cfg(debug_assertions)]
const DRAIN_ROUNDS: usize = 16;

/// A frame built cold, from [`ColdFrame::begin`] until it drops: every
/// piece of engine state on the thread entered by its policy. Dropped, on
/// return or unwind: first what the frame made is dropped while the frame
/// still runs (so what those drops do, a timer set, a signal written, is
/// the frame's), again while dropping touched engine state; then every
/// piece is put back, what is left of the frame's dropped, and the first
/// panic goes on.
#[cfg(debug_assertions)]
#[must_use]
pub(crate) struct ColdFrame {
    ended: bool,
}

#[cfg(debug_assertions)]
impl ColdFrame {
    pub(crate) fn begin() -> Self {
        let entries = REGISTRY.with(|r| {
            let mut r = r.borrow_mut();
            assert!(r.cold.is_none(), "a cold frame inside a cold frame");
            r.cold = Some(Vec::new());
            r.entries.clone()
        });
        // Entering one may register another: that one enters as it does.
        for e in entries {
            let saved = (e.enter)();
            REGISTRY.with(|r| {
                if let Some(c) = r.borrow_mut().cold.as_mut() {
                    c.push((e, saved));
                }
            });
        }
        Self { ended: false }
    }

    /// What the frame made, taken out of every entered piece (last
    /// entered first) while it still runs.
    fn drain() -> Vec<(&'static str, Box<dyn Any>)> {
        let n = REGISTRY
            .try_with(|r| r.borrow().cold.as_ref().map_or(0, Vec::len))
            .unwrap_or(0);
        let mut made = Vec::with_capacity(n);
        for i in (0..n).rev() {
            // Its saved value taken out while its drain runs, which may
            // use the registry.
            let Ok(Some((e, saved))) = REGISTRY.try_with(|r| {
                r.borrow_mut()
                    .cold
                    .as_mut()
                    .and_then(|c| c.get_mut(i))
                    .map(|(e, s)| (*e, std::mem::replace(s, Box::new(()))))
            }) else {
                continue;
            };
            let m = (e.drain)(&*saved);
            REGISTRY
                .try_with(|r| {
                    if let Some(slot) = r.borrow_mut().cold.as_mut().and_then(|c| c.get_mut(i)) {
                        slot.1 = saved;
                    }
                })
                .ok();
            made.push((e.name, m));
        }
        made
    }

    fn end(&mut self) {
        if std::mem::replace(&mut self.ended, true) {
            return;
        }
        let mut first = None;
        let drop_all = |made: Vec<(&'static str, Box<dyn Any>)>, first: &mut Option<Box<dyn Any + Send>>| {
            for (name, m) in made {
                if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(m))) {
                    eprintln!("gui: dropping what a cold frame made in {name} panicked; the live state is put back");
                    first.get_or_insert(p);
                }
            }
        };
        // What the frame made dropped while it still runs: what those drops
        // do is the frame's too, and dropped in turn.
        let mut settled = false;
        let mut wrote = Vec::new();
        for _ in 0..DRAIN_ROUNDS {
            let made = Self::drain();
            let before = gui_tls::touches();
            gui_tls::watch();
            drop_all(made, &mut first);
            wrote = gui_tls::watched();
            if gui_tls::touches() == before {
                settled = true;
                break;
            }
        }
        // Drops that still make more after every round are a destructor
        // that keeps scheduling work: what they made is never dropped
        // (dropped live it would reach the live state), and the check fails.
        let runaway = (!settled).then(|| {
            Self::leak(Self::drain());
            Self::named(&wrote)
        });
        let Ok(Some(saved)) = REGISTRY.try_with(|r| r.borrow_mut().cold.take()) else {
            return;
        };
        // Then everything put back, last entered first.
        let mut left = Vec::with_capacity(saved.len());
        for (e, s) in saved.into_iter().rev() {
            left.push((e.name, (e.exit)(s)));
        }
        if runaway.is_some() {
            Self::leak(left);
        } else {
            drop_all(left, &mut first);
        }
        if std::thread::panicking() {
            if let Some(pieces) = runaway {
                eprintln!("{}", Self::runaway_message(&pieces));
            }
            return;
        }
        if let Some(pieces) = runaway {
            Self::fail(&pieces);
        }
        if let Some(p) = first {
            std::panic::resume_unwind(p);
        }
    }

    /// Never drop `made`: what a runaway destructor made, which dropped
    /// would schedule more, live.
    #[expect(
        clippy::mem_forget,
        reason = "a runaway destructor's values are leaked by design: the cold check fails on them"
    )]
    fn leak(made: Vec<(&'static str, Box<dyn Any>)>) {
        std::mem::forget(made);
    }

    /// The registered pieces among `slots`: each name and type.
    fn named(slots: &[usize]) -> Vec<(&'static str, &'static str)> {
        REGISTRY
            .try_with(|r| {
                let r = r.borrow();
                slots
                    .iter()
                    .filter_map(|s| r.named.iter().find(|(slot, ..)| slot == s))
                    .map(|&(_, name, ty)| (name, ty))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn runaway_message(pieces: &[(&'static str, &'static str)]) -> String {
        let pieces: Vec<String> = pieces.iter().map(|(name, ty)| format!("{name} ({ty})")).collect();
        format!(
            "gui: what a cold frame made still made more as it dropped after {DRAIN_ROUNDS} rounds: \
             a destructor that schedules work again (a bug). The last drops wrote to {}. \
             The live state is put back; what they made is leaked, not dropped live.",
            pieces.join(", ")
        )
    }

    #[expect(
        clippy::panic,
        reason = "the cold check fails loudly on a destructor that never settles"
    )]
    fn fail(pieces: &[(&'static str, &'static str)]) -> ! {
        panic!("{}", Self::runaway_message(pieces));
    }
}

#[cfg(debug_assertions)]
impl Drop for ColdFrame {
    fn drop(&mut self) {
        self.end();
    }
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{ColdFrame, is_cold};
    use crate::reactive::{Effect, Signal};
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

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

    /// The ids a cold frame hands out are handed out again live: every
    /// counter is as it was before it.
    #[test]
    fn a_cold_frame_leaves_the_id_counters_as_they_were() {
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
        assert!(cold.0 == live.0, "timer");
        assert!(cold.1 == live.1, "focus");
        assert!(cold.2 == live.2, "signal");
        assert!(cold.3 == live.3, "access ref");
        assert!(cold.4 == live.4, "capture group");
        assert!(cold.5 == live.5, "layer");
        assert!(cold.6 == live.6, "registration");
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

    /// How many times [`INNER`]'s init ran.
    static INNER_MADE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    crate::engine_local! {
        /// Made by [`OUTER`]'s init.
        [Shared("a test of a Shared init within another")]
        static INNER: Signal<u32> = {
            INNER_MADE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Signal::new(7_u32)
        };
        /// Makes a signal, then [`INNER`], then fails.
        [Shared("a test of a Shared init within another")]
        static OUTER: Signal<u32> = {
            let made = Signal::new(0_u32);
            let inner = INNER.with(|s| *s);
            if inner.get_untracked() == 7 {
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
        let before = super::fork_work_done();
        {
            let _cold = ColdFrame::begin();
            scope.run(|| Signal::new(1_u8));
            let reader = crate::reactive::Computed::new(move || read.get());
            reader.get();
        }
        let done = super::fork_work_done() - before;
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

    /// A value only the cold frame made panics as it drops: the live state
    /// is back all the same, and the panic goes on.
    #[test]
    fn a_panicking_cold_value_still_puts_the_live_state_back() {
        let kept = Signal::new(1_u32);
        // The runtime enters before the timers, and so is put back after.
        crate::clock::cancel_all();
        let caught = std::panic::catch_unwind(|| {
            let _cold = ColdFrame::begin();
            let _made = Signal::new(2_u32);
            let boom = Boom;
            crate::clock::after(Duration::from_secs(1), move || {
                let _ = &boom;
            });
        });
        assert!(caught.is_err(), "the panic goes on");
        assert!(!is_cold(), "no cold frame runs");
        assert_eq!(kept.get_untracked(), 1);
        let _live = Signal::new(3_u32);
        assert_eq!(crate::reactive::cold_steps(), 0, "the runtime logs no more");
        assert_eq!(crate::component::cold_tree_steps(), 0, "the tree logs no more");
        assert!(crate::clock::next_deadline().is_none(), "the cold timer is gone");
        // And another begins and ends.
        drop(ColdFrame::begin());
    }
}
