//! Sources a bound setter reads without a tracker of its own (review
//! §12.5, level 2): a flag of an [`Interaction`] (`i.hover(rest, lit)`),
//! one [`Signal`] (`signal.map(f)`, or the signal itself), and a [`Group`]
//! of signals read once for all its readers (`group.map(f)`; shadcn's theme).
//!
//! A closure given to a setter runs under a tracker of its own on each
//! node it is mounted on: a node of the reactive arena, and a link to each
//! signal it read. A source needs neither. Its mount registers with what
//! it reads, in a list of [`Waiters`] that is the source's own. A write
//! wakes exactly the mounts registered there, each by the mark of the node
//! it is mounted on (the one a tracker of that node would run), and flags
//! each of them woken; a mount that was not woken does not run when its
//! node resolves for another reason.
//!
//! Who owns what:
//! - The flag sources of every [`Interaction`] handle share one list
//!   ([`HANDLE_WAITERS`]), a chain per handle, whatever node the handle is
//!   bound to: a handle costs no allocation of its own. Writing a flag of
//!   a node wakes the chain of the handle bound to it, for that flag;
//!   binding the handle to a node, or unbinding it, wakes the whole chain.
//! - A signal read by one mount is read by that mount's own tracker, as a
//!   closure is: one source costs what the closure it replaces does. Read
//!   by more, all of them share one hub, one tracker: the second mount
//!   makes it, and the first moves in when its node resolves next
//!   ([`HUBS`]). It goes with the last.
//! - A group's hub is the group's ([`Group::new`]), one tracker for every
//!   mount of every map of it.
//! - A registration ([`Wait`]) is the mount's: it leaves the list when the
//!   mount goes (its node, or its write no longer bound), or moves to the
//!   node's parent when the node is hidden (`display: none`), as a
//!   tracker's reads do; not one on a handle bound inside the hidden
//!   subtree, which is never hovered nor focused while hidden.
//!
//! A map given to a source (`signal.map(f)`, `group.map(f)`), and the
//! conversion of what it gives to the setter's type, read only what they
//! are given: they run untracked, so a signal they read would not wake
//! them. A debug build reports such a read, naming the setter's call.
//!
//! A cold build (the cold check) registers nothing and touches no hub: its
//! mounts read their sources directly at each resolve, so it changes no
//! live state.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::Location;
use std::rc::{Rc, Weak};

use crate::engine::{WorldCell, WorldRefCell};

use crate::interact::{Flag, Interaction};
use crate::reactive::{Mark, RenderNode, Signal};

/// Wake every waiter, whatever it registered for.
pub(crate) const EVERY: u8 = u8::MAX;

/// No position: a key whose waiter went.
const GONE: usize = usize::MAX;

/// Whether a cold frame runs: its mounts register nowhere.
pub(crate) fn cold() -> bool {
    #[cfg(debug_assertions)]
    {
        crate::engine::is_cold()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

/// One mount waiting on a source: the mark of its node, what it waits
/// for (a bit per [`Flag`], or [`EVERY`]), whether it was woken since it
/// last ran, its key, and in a list of chains, its chain and the keys
/// before and after it there.
struct Waiter {
    mark: Mark,
    mask: u8,
    woken: WorldCell<bool>,
    key: usize,
    chain: Option<Link>,
}

/// A waiter's place in its chain: the chain, and the keys before and after
/// it ([`GONE`] at either end).
#[derive(Clone, Copy)]
struct Link {
    id: u64,
    prev: usize,
    next: usize,
}

/// The mounts waiting on one source, kept dense: a wake visits the mounts
/// that wait now, not as many as ever did. A key names a mount for as long
/// as it waits, whatever its place; keys and places freed by mounts that
/// went are reused, so registering in steady state allocates nothing.
///
/// One list may hold many sources, a chain each (the flags of every
/// [`Interaction`] handle, a chain per handle): a handle costs no list of
/// its own, and a wake of a chain visits that chain's mounts alone.
///
/// Of the world that made it (review r18, decision 2): the live mounts'
/// lists are written live alone; a live mount's registration a check lets
/// go leaves its list once the check has switched back ([`Wait`]'s drop).
pub(crate) struct Waiters {
    list: WorldRefCell<List>,
    /// Waiters a wake visited, for a test.
    #[cfg(test)]
    visits: WorldCell<usize>,
}

impl Default for Waiters {
    fn default() -> Self {
        Self {
            list: WorldRefCell::new("a source's waiting mounts (`Waiters`)", List::default()),
            #[cfg(test)]
            visits: WorldCell::new("a source's wake visits (`Waiters`, a test's)", 0),
        }
    }
}

#[derive(Default)]
struct List {
    items: Vec<Waiter>,
    /// Each key's place in `items`, [`GONE`] once free.
    at: Vec<usize>,
    free: Vec<usize>,
    /// The first key of each chain.
    heads: HashMap<u64, usize>,
}

impl List {
    fn get_mut(&mut self, key: usize) -> Option<&mut Waiter> {
        let at = *self.at.get(key)?;
        self.items.get_mut(at)
    }
}

impl Waiters {
    /// Wake the waiters of any of `mask`: each is flagged woken and its
    /// node's mark runs. Allocates nothing.
    pub(crate) fn wake(&self, mask: u8) {
        self.list.try_with(|list| {
            for w in &list.items {
                self.visit(w, mask);
            }
        });
    }

    /// Wake the waiters of chain `id` of any of `mask`. Allocates nothing.
    pub(crate) fn wake_chain(&self, id: u64, mask: u8) {
        self.list.try_with(|list| {
            let mut key = list.heads.get(&id).copied().unwrap_or(GONE);
            while let Some(w) = list.at.get(key).and_then(|&at| list.items.get(at)) {
                self.visit(w, mask);
                key = w.chain.map_or(GONE, |c| c.next);
            }
        });
    }

    fn visit(&self, w: &Waiter, mask: u8) {
        #[cfg(test)]
        self.visits.set(self.visits.get().saturating_add(1));
        if w.mask & mask != 0 {
            w.woken.set(true);
            (w.mark)();
        }
    }

    /// Register a mount, woken through `mark` by a wake of any of `mask`
    /// (of chain `chain`, if given); `keep` is what the registration holds
    /// alive (a signal's hub).
    ///
    /// Panics if the list is waking: a wake runs marks alone, which push a
    /// node to resolve and run no mount, so no mount registers during one.
    fn wait(self: &Rc<Self>, mark: &Mark, mask: u8, chain: Option<u64>, keep: Option<Rc<dyn Any>>) -> Wait {
        let key = self.list.with_mut(|list| Self::join_list(list, mark, mask, chain));
        Wait::new(Waiting {
            on: Some(Rc::clone(self)),
            key,
            keep,
            solo: None,
        })
    }

    /// [`Self::wait`]'s place in `list`: its key.
    fn join_list(list: &mut List, mark: &Mark, mask: u8, chain: Option<u64>) -> usize {
        let place = list.items.len();
        let key = match list.free.pop() {
            Some(k) => {
                if let Some(at) = list.at.get_mut(k) {
                    *at = place;
                }
                k
            }
            None => {
                list.at.push(place);
                list.at.len().saturating_sub(1)
            }
        };
        // First in its chain.
        let chain = chain.map(|id| {
            let next = list.heads.insert(id, key).unwrap_or(GONE);
            if let Some(n) = list.get_mut(next).and_then(|n| n.chain.as_mut()) {
                n.prev = key;
            }
            Link { id, prev: GONE, next }
        });
        list.items.push(Waiter {
            mark: Rc::clone(mark),
            mask,
            woken: WorldCell::new("whether a mount was woken (`Waiter::woken`)", false),
            key,
            chain,
        });
        key
    }

    /// How many mounts wait on it, and how many places its lists hold.
    #[cfg(test)]
    fn sizes(&self) -> (usize, usize) {
        self.list.with(|list| (list.items.len(), list.at.len()))
    }
}

/// A mount's registration: with a source's [`Waiters`], leaving it when
/// dropped; or, for the one mount of a signal, its claim to read it under
/// its own tracker. It ends in the world that made it (ADR-0035): a live
/// mount's registration a check lets go leaves its list live, after the
/// switch; a check's that escaped ends nowhere.
pub(crate) struct Wait(crate::engine::Ends<Waiting>);

impl Wait {
    fn new(waiting: Waiting) -> Self {
        Self(crate::engine::Ends::new(waiting))
    }
}

impl std::ops::Deref for Wait {
    type Target = Waiting;
    fn deref(&self) -> &Waiting {
        &self.0
    }
}

/// What a [`Wait`] holds.
pub(crate) struct Waiting {
    on: Option<Rc<Waiters>>,
    key: usize,
    /// What it holds alive: a signal's hub.
    keep: Option<Rc<dyn Any>>,
    /// The signal it reads alone, under its own tracker, and its claim.
    solo: Option<((usize, u32), u64)>,
}

impl Waiting {
    fn with<R>(&self, f: impl FnOnce(&Waiter) -> R) -> Option<R> {
        self.on
            .as_ref()?
            .list
            .try_with(|list| {
                let at = *list.at.get(self.key)?;
                list.items.get(at).map(f)
            })
            .flatten()
    }

    /// Whether the source woke it since it last ran. A solo mount's
    /// tracker says so for it, until the hub took its claim: it then runs
    /// to move in.
    fn woken(&self) -> bool {
        if self.solo.is_some() {
            return !self.is_solo();
        }
        self.on.is_some() && self.with(|w| w.woken.get()).unwrap_or(true)
    }

    /// It runs now: what wakes it from here on is new.
    fn clear(&self) {
        self.with(|w| w.woken.set(false));
    }

    /// Whether it waits on `list` for `mask`; on `list` for another mask,
    /// it waits for `mask` from now on. False when it could not say so (the
    /// list is waking): registered anew.
    fn holds(&self, list: &Rc<Waiters>, mask: u8) -> bool {
        if !self.on.as_ref().is_some_and(|on| Rc::ptr_eq(on, list)) {
            return false;
        }
        list.list
            .try_with_mut(
                |l| match l.at.get(self.key).copied().and_then(|at| l.items.get_mut(at)) {
                    Some(w) => {
                        w.mask = mask;
                        true
                    }
                    None => false,
                },
            )
            .unwrap_or(false)
    }

    /// The handle whose flag it waits on, for a flag source.
    pub(crate) fn handle(&self) -> Option<u64> {
        self.with(|w| w.chain.map(|c| c.id)).flatten()
    }

    /// Whether it is the solo read of a signal, its claim standing: its
    /// mount runs under a tracker of its own.
    pub(crate) fn is_solo(&self) -> bool {
        self.solo.is_some_and(|(key, id)| claim(key, id, |_| ()).is_some())
    }

    /// Wake `mark`'s node from now on (a hidden node's parent, which
    /// builds the node again when what hid it changes).
    pub(crate) fn repoint(&self, mark: &Mark) {
        if let Some((key, id)) = self.solo {
            claim(key, id, |c| c.mark = Rc::clone(mark));
        }
        if let Some(on) = &self.on {
            on.list.try_with_mut(|l| {
                if let Some(&at) = l.at.get(self.key)
                    && let Some(w) = l.items.get_mut(at)
                {
                    w.mark = Rc::clone(mark);
                }
            });
        }
    }
}

impl crate::engine::Ending for Waiting {
    /// Its claim goes, and its place in its list, in its own world: a live
    /// mount's registration a check lets go (a live pipeline dropped in a
    /// check) leaves the live list once the check has switched back, never
    /// from the check (review r18, decision 2; ADR-0035).
    fn end(mut self) {
        if let Some((key, id)) = self.solo {
            solo_gone(key, id);
        }
        if let Some(on) = self.on.take() {
            leave(&on, self.key);
        }
    }
}

/// Take waiter `key` out of `on`'s list.
fn leave(on: &Waiters, key: usize) {
    // Borrowed only while it wakes, and a mark drops no registration:
    // should one, the waiter stays, waking a node for nothing.
    on.list.try_with_mut(|l| {
        if let Some(at) = l.at.get(key).copied().filter(|&a| a != GONE)
            && at < l.items.len()
        {
            // Out of its chain first.
            if let Some(Link { id, prev, next }) = l.items.get(at).and_then(|w| w.chain) {
                match l.get_mut(prev).and_then(|p| p.chain.as_mut()) {
                    Some(p) => p.next = next,
                    None if next == GONE => {
                        l.heads.remove(&id);
                    }
                    None => {
                        l.heads.insert(id, next);
                    }
                }
                if let Some(n) = l.get_mut(next).and_then(|n| n.chain.as_mut()) {
                    n.prev = prev;
                }
            }
            l.items.swap_remove(at);
            if let Some(moved) = l.items.get(at).map(|w| w.key)
                && let Some(m) = l.at.get_mut(moved)
            {
                *m = at;
            }
            if let Some(a) = l.at.get_mut(key) {
                *a = GONE;
            }
            l.free.push(key);
        }
    });
}

/// One tracker standing for every mount of a source (a signal, a group):
/// what it read changing wakes its waiters, once, and it stays marked
/// until a mount runs again and reads anew ([`Hub::arm`]).
pub(crate) struct Hub {
    /// Disposed in the world that made it as the hub ends (ADR-0035).
    node: crate::engine::Ends<RenderNode>,
    waiters: Rc<Waiters>,
    armed: WorldCell<bool>,
}

impl Hub {
    fn new() -> Self {
        let waiters = Rc::new(Waiters::default());
        let woken = Rc::downgrade(&waiters);
        let node = crate::reactive::detached(|| {
            RenderNode::new(move || {
                if let Some(w) = woken.upgrade() {
                    w.wake(EVERY);
                }
            })
        });
        Self {
            node: crate::engine::Ends::new(node),
            waiters,
            armed: WorldCell::new("whether a source's hub read (`Hub::armed`)", false),
        }
    }

    /// Run `read` under the hub's tracker when what it last read changed,
    /// or it never ran to its end; whether it ran. Armed only once `read`
    /// returned: one that panicked leaves the hub to read again.
    fn arm(&self, read: impl FnOnce()) -> bool {
        if self.armed.get() && !self.node.is_marked() {
            return false;
        }
        self.armed.set(false);
        self.node.track(read);
        self.armed.set(true);
        true
    }
}

/// Run `f`, a map given to a source at `at`, which must read no signal
/// (it runs untracked: what it read would not wake it). A debug build
/// panics on such a read.
#[cfg_attr(not(debug_assertions), expect(unused_variables, reason = "the check is debug only"))]
fn pure<R>(at: &'static Location<'static>, f: impl FnOnce() -> R) -> R {
    #[cfg(debug_assertions)]
    if !cold() {
        let probe = PROBE.with(|p| *p);
        let out = probe.track(f);
        if probe.has_deps() {
            probe.track(|| ());
            report_read(at);
        }
        return out;
    }
    f()
}

#[cfg(debug_assertions)]
#[expect(
    clippy::panic,
    reason = "a map reading a signal is a bug in the caller, reported where it was given"
)]
fn report_read(at: &'static Location<'static>) -> ! {
    panic!(
        "the map given to a setter at {at} read a signal: a source's map runs untracked, so a change of that \
         signal would not reach it; read it in a closure, or put it in a gui::value::Group"
    )
}

#[cfg(debug_assertions)]
crate::engine::engine_local! {
    /// What a source's map reads, in a debug build: nothing, or a report.
    [Shared("a tracker made live; what it reads is undone with the frame that read it")]
    static PROBE: RenderNode = RenderNode::new(|| {});
}

/// What a bound setter reads where it is mounted: a closure, under a
/// tracker of the mount's own, or a source the mount registers with.
pub(crate) trait Read: 'static {
    type Out;
    /// Whether it runs under a tracker of the mount's own (a closure).
    const TRACKED: bool;
    /// What it gives now, tracked by whatever is running: a closure as it
    /// is, a source through what its signals give (a bare
    /// [`crate::style::Style`], a span, a prop given as a closure).
    fn get(&self) -> Self::Out;
    /// What it gives now, for its mount: a source reads untracked (its
    /// mount is woken through its registration, not a tracker), or, the
    /// solo read of a signal, tracked by the mount's own tracker.
    fn peek(&self, _solo: bool) -> Self::Out {
        self.get()
    }
    /// Keep `wait` registered with what it reads, woken through `mark`.
    fn watch(&self, _mark: &Mark, _wait: &mut Option<Wait>) {}
}

/// A closure a setter was given.
pub(crate) struct Closure<F>(pub(crate) F);

impl<F: Fn() -> T + 'static, T: 'static> Read for Closure<F> {
    type Out = T;
    const TRACKED: bool = true;

    fn get(&self) -> T {
        (self.0)()
    }
}

/// A source whose value converts to the setter's ([`super::Convert`]).
/// The conversion is part of its map: it reads only what it is given (a
/// debug build reports a signal it reads, naming the setter's call), or
/// its mounts would give what the order they ran in made them read.
pub(crate) struct Via<R, T> {
    read: R,
    at: &'static Location<'static>,
    to: std::marker::PhantomData<fn() -> T>,
}

impl<R, T> Via<R, T> {
    #[track_caller]
    pub(crate) fn new(read: R) -> Self {
        Self {
            read,
            at: Location::caller(),
            to: std::marker::PhantomData,
        }
    }
}

impl<R: Read, T: 'static> Read for Via<R, T>
where
    R::Out: super::Convert<T>,
{
    type Out = T;
    const TRACKED: bool = R::TRACKED;

    fn get(&self) -> T {
        super::Convert::convert(self.read.get())
    }

    fn peek(&self, solo: bool) -> T {
        let v = self.read.peek(solo);
        pure(self.at, || super::Convert::convert(v))
    }

    fn watch(&self, mark: &Mark, wait: &mut Option<Wait>) {
        self.read.watch(mark, wait);
    }
}

/// Register `wait` with `list` for `mask`, unless it waits there already
/// (for another mask, it waits for `mask` from now on).
fn keep(
    wait: &mut Option<Wait>,
    list: &Rc<Waiters>,
    mark: &Mark,
    mask: u8,
    hold: impl FnOnce() -> Option<Rc<dyn Any>>,
) {
    if !wait.as_ref().is_some_and(|w| w.holds(list, mask)) {
        *wait = Some(list.wait(mark, mask, None, hold()));
    }
}

// ── an interaction's flag ─────────────────────────────────────────────

/// One value while a flag of an [`Interaction`] is off, another while it
/// is on (`i.hover(rest, lit)`, `i.when(Flag::FocusVisible, off, ring)`):
/// a setter given it reads the flag of the node the handle is bound to,
/// and its mount registers with the handle, waking on that flag alone and
/// when the handle is bound to another node.
#[derive(Clone)]
pub struct Toggle<T> {
    handle: Interaction,
    flag: Flag,
    off: T,
    on: T,
}

impl Interaction {
    /// `on` while `flag` holds, else `off`, as a setter's value.
    #[must_use]
    pub fn when<T>(self, flag: Flag, off: T, on: T) -> Toggle<T> {
        Toggle {
            handle: self,
            flag,
            off,
            on,
        }
    }

    /// `hovered` while the pointer is over the node (CSS `:hover`), else
    /// `rest`, as a setter's value: `.background(i.hover(rest, lit))`.
    #[must_use]
    pub fn hover<T>(self, rest: T, hovered: T) -> Toggle<T> {
        self.when(Flag::Hovered, rest, hovered)
    }
}

impl<T: Clone> Toggle<T> {
    fn pick(&self, on: bool) -> T {
        if on { self.on.clone() } else { self.off.clone() }
    }
}

impl<T: Clone + 'static> Read for Toggle<T> {
    type Out = T;
    const TRACKED: bool = false;

    fn get(&self) -> T {
        self.pick(self.handle.flag(self.flag))
    }

    fn peek(&self, _: bool) -> T {
        self.pick(self.handle.peek(self.flag))
    }

    fn watch(&self, mark: &Mark, wait: &mut Option<Wait>) {
        let (bit, id) = (self.flag.bit(), self.handle.id());
        let list = HANDLE_WAITERS.with(Rc::clone);
        // Registered with this handle already (for this flag from now on).
        if let Some(w) = wait.as_ref()
            && w.handle() == Some(id)
            && w.holds(&list, bit)
        {
            return;
        }
        *wait = Some(list.wait(mark, bit, Some(id), None));
    }
}

crate::engine::engine_local! {
    /// The flag sources of every handle, a chain per handle: bound or not,
    /// whatever node it is bound to.
    [Shared("registered with by live mounts only: a cold build's mounts register nowhere")]
    static HANDLE_WAITERS: Rc<Waiters> = Rc::default();
}

/// Wake the flag sources of handle `id` waiting for any of `mask`.
pub(crate) fn wake_handle(id: u64, mask: u8) {
    if mask == 0 || cold() {
        return;
    }
    // The thread may be ending (its engine state with it).
    HANDLE_WAITERS.try_with(|w| w.wake_chain(id, mask)).ok();
}

// ── one signal ────────────────────────────────────────────────────────

/// The claim of the one mount reading a signal alone: which claim it is,
/// and the mark that wakes its node (to move into the hub, once another
/// mount reads the signal).
struct Claim {
    id: u64,
    mark: Mark,
}

/// What the mounts of one signal share: the claim of the one reading it
/// alone, under a tracker of its own, or the hub all of them share.
#[derive(Default)]
struct SignalReaders {
    solo: Option<Claim>,
    hub: Weak<SignalHub>,
}

/// The readers of each signal a source reads, and the last claim made.
#[derive(Default)]
struct Readers {
    last: u64,
    by: HashMap<(usize, u32), SignalReaders>,
}

crate::engine::engine_local! {
    /// The readers of each signal a source reads, by the signal's slot and
    /// generation: gone with the last.
    [Shared("made and registered with by live mounts only: a cold build reads its signals directly")]
    static HUBS: RefCell<Readers> = RefCell::default();
}

/// A signal's hub, and the signal it stands for.
pub(crate) struct SignalHub {
    key: (usize, u32),
    /// Its entry, forgotten first as it goes.
    _entry: crate::engine::Ends<HubEntry>,
    hub: Hub,
}

/// Drop the entry of `key` when nothing reads it any more.
fn forget(h: &mut Readers, key: (usize, u32)) {
    if h.by
        .get(&key)
        .is_some_and(|r| r.solo.is_none() && r.hub.strong_count() == 0)
    {
        h.by.remove(&key);
    }
}

/// Run `f` on the claim `(key, id)`, if it still stands (no hub took it).
fn claim<R>(key: (usize, u32), id: u64, f: impl FnOnce(&mut Claim) -> R) -> Option<R> {
    HUBS.try_with(|h| {
        let mut h = h.try_borrow_mut().ok()?;
        h.by.get_mut(&key)?.solo.as_mut().filter(|c| c.id == id).map(f)
    })
    .ok()
    .flatten()
}

/// The claim `(key, id)` went with its mount.
fn solo_gone(key: (usize, u32), id: u64) {
    // The thread may be ending (its engine state with it).
    HUBS.try_with(|h| {
        if let Ok(mut h) = h.try_borrow_mut() {
            if let Some(r) = h.by.get_mut(&key)
                && r.solo.as_ref().is_some_and(|c| c.id == id)
            {
                r.solo = None;
            }
            forget(&mut h, key);
        }
    })
    .ok();
}

/// A signal hub's entry among the readers ([`HUBS`]): forgotten as its hub
/// goes, in the world that made it (ADR-0035).
struct HubEntry((usize, u32));

impl crate::engine::Ending for HubEntry {
    fn end(self) {
        HUBS.try_with(|h| {
            if let Ok(mut h) = h.try_borrow_mut() {
                forget(&mut h, self.0);
            }
        })
        .ok();
    }
}

/// How a mount reading `key` through `mark` joins its readers: alone (a
/// claim), when no other mount reads it; else through the hub. The hub is
/// made by the second mount, and the first, reading alone, is woken to
/// move into it: N mounts of a signal hold one tracker.
fn join(key: (usize, u32), mark: &Mark) -> Result<Rc<SignalHub>, u64> {
    let (hub, moved) = HUBS.with(|h| {
        let mut h = h.borrow_mut();
        h.last = h.last.wrapping_add(1);
        let id = h.last;
        let r = h.by.entry(key).or_default();
        if let Some(hub) = r.hub.upgrade() {
            return (Ok(hub), None);
        }
        let Some(first) = r.solo.take() else {
            r.solo = Some(Claim {
                id,
                mark: Rc::clone(mark),
            });
            return (Err(id), None);
        };
        let hub = Rc::new(SignalHub {
            key,
            _entry: crate::engine::Ends::new(HubEntry(key)),
            hub: Hub::new(),
        });
        r.hub = Rc::downgrade(&hub);
        (Ok(hub), Some(first.mark))
    });
    // The first moves in when its node resolves next.
    if let Some(m) = moved {
        m();
    }
    hub
}

/// `f` of what a [`Signal`] holds, as a setter's value (`signal.map(f)`).
/// Its one mount reads the signal under a tracker of its own, as a closure
/// does; mounted more than once, all of them register with the signal's
/// hub, one tracker for all of them. `f` reads only what it is given.
pub struct Mapped<S: 'static, F> {
    signal: Signal<S>,
    f: F,
    at: &'static Location<'static>,
}

impl<S: 'static, F: Clone> Clone for Mapped<S, F> {
    fn clone(&self) -> Self {
        Self {
            signal: self.signal,
            f: self.f.clone(),
            at: self.at,
        }
    }
}

impl<S: 'static> Signal<S> {
    /// `f` of the value, as a setter's value: `.opacity(open.map(|o| if *o { 1.0 } else { 0.0 }))`.
    /// A setter given the signal itself reads its value so. `f` reads only
    /// what it is given (a debug build reports a signal it reads).
    #[must_use]
    #[track_caller]
    pub fn map<T, F: Fn(&S) -> T + 'static>(self, f: F) -> Mapped<S, F> {
        Mapped {
            signal: self,
            f,
            at: Location::caller(),
        }
    }
}

impl<S: 'static, T: 'static, F: Fn(&S) -> T + 'static> Read for Mapped<S, F> {
    type Out = T;
    const TRACKED: bool = false;

    fn get(&self) -> T {
        self.signal.with(&self.f)
    }

    fn peek(&self, solo: bool) -> T {
        let f = |s: &S| pure(self.at, || (self.f)(s));
        if solo {
            self.signal.with(f)
        } else {
            self.signal.with_untracked(f)
        }
    }

    fn watch(&self, mark: &Mark, wait: &mut Option<Wait>) {
        let signal = self.signal;
        let key = signal.key();
        if let Some(w) = wait.as_ref()
            && let Some((k, _)) = w.solo
            && k == key
            && w.is_solo()
        {
            // Read alone still, under its mount's tracker.
            return;
        }
        // Its hub, held already, or joined now (a claim taken by the hub
        // joins it).
        let held = wait
            .as_ref()
            .and_then(|w| w.keep.clone())
            .and_then(|k| k.downcast::<SignalHub>().ok())
            .filter(|h| h.key == key);
        let hub = match held.map_or_else(|| join(key, mark), Ok) {
            Ok(hub) => hub,
            Err(id) => {
                *wait = Some(Wait::new(Waiting {
                    on: None,
                    key: 0,
                    keep: None,
                    solo: Some((key, id)),
                }));
                return;
            }
        };
        hub.hub.arm(|| {
            if signal.is_alive() {
                signal.with(|_| ());
            }
        });
        let list = Rc::clone(&hub.hub.waiters);
        let hold: Rc<dyn Any> = hub;
        keep(wait, &list, mark, EVERY, || Some(hold));
    }
}

// ── a group ───────────────────────────────────────────────────────────

/// A value read from signals once for all its readers: one tracker for
/// the group, and a list its mounts register with. Made where the value
/// is provided (shadcn's theme makes one per aspect: its colours, its
/// radius, its shadows), mapped where it is read: `.background(colors.map(move |c| c.get(role)))`.
///
/// Its value is read again when a signal it read changes, once, by the
/// first mount to run after; each mount then runs its map over it. A
/// closure may read the group too ([`Self::with`]): it reads `read`
/// tracked, as any closure reads signals.
pub struct Group<T: 'static>(Rc<GroupCell<T>>);

struct GroupCell<T> {
    read: Box<dyn Fn() -> T>,
    hub: Hub,
    /// The world's that made the group: a cold build reads the value anew
    /// and keeps none of it (ADR-0034, S7).
    value: crate::engine::WorldRefCell<Option<T>>,
}

impl<T: 'static> Clone for Group<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: 'static> std::fmt::Debug for Group<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Group")
    }
}

impl<T: 'static> Group<T> {
    /// The group of what `read` gives.
    #[must_use]
    pub fn new(read: impl Fn() -> T + 'static) -> Self {
        Self(Rc::new(GroupCell {
            read: Box::new(read),
            hub: Hub::new(),
            value: crate::engine::WorldRefCell::new("a group's value (`Group`)", None),
        }))
    }

    /// `f` of its value as `read` gives it now, subscribing the running
    /// reader to what `read` reads.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&(self.0.read)())
    }

    /// `f` of its value, as a setter's value. `f` reads only what it is
    /// given (a debug build reports a signal it reads).
    #[must_use]
    #[track_caller]
    pub fn map<U, F: Fn(&T) -> U + 'static>(&self, f: F) -> Grouped<T, F> {
        Grouped {
            group: self.clone(),
            f,
            at: Location::caller(),
        }
    }

    /// `f` of the value its mounts read: kept until a signal `read` read
    /// changes, untracked; a cold build reads it anew.
    fn current<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let c = &*self.0;
        if cold() {
            return f(&crate::reactive::untracked(|| (c.read)()));
        }
        c.hub.arm(|| {
            let v = (c.read)();
            c.value.with_mut(|value| *value = Some(v));
        });
        c.value.with(|value| match value {
            Some(v) => f(v),
            None => f(&crate::reactive::untracked(|| (c.read)())),
        })
    }
}

/// `f` of a [`Group`]'s value, as a setter's value (`group.map(f)`).
pub struct Grouped<T: 'static, F> {
    group: Group<T>,
    f: F,
    at: &'static Location<'static>,
}

impl<T: 'static, F: Clone> Clone for Grouped<T, F> {
    fn clone(&self) -> Self {
        Self {
            group: self.group.clone(),
            f: self.f.clone(),
            at: self.at,
        }
    }
}

impl<G: 'static, T: 'static, F: Fn(&G) -> T + 'static> Read for Grouped<G, F> {
    type Out = T;
    const TRACKED: bool = false;

    fn get(&self) -> T {
        self.group.with(&self.f)
    }

    fn peek(&self, _: bool) -> T {
        self.group.current(|g| pure(self.at, || (self.f)(g)))
    }

    fn watch(&self, mark: &Mark, wait: &mut Option<Wait>) {
        keep(wait, &self.group.0.hub.waiters, mark, EVERY, || None);
    }
}

/// Whether a mount of a source must run: it never ran, or a cold build
/// mounted it (registered nowhere), or its source woke it.
pub(crate) fn due(tracked: bool, wait: Option<&Wait>) -> bool {
    !tracked && wait.is_none_or(|w| w.woken())
}

/// A mount of a source is about to run: registered with what it reads
/// (not in a cold build), and its wake taken.
pub(crate) fn watch<R: Read>(read: &R, mark: &Mark, wait: &mut Option<Wait>) {
    if R::TRACKED {
        return;
    }
    if cold() {
        *wait = None;
        return;
    }
    read.watch(mark, wait);
    if let Some(w) = wait {
        w.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::{EVERY, Group, Waiters};
    use crate::reactive::{Mark, Signal};

    fn mark() -> (Mark, Rc<Cell<u32>>) {
        let n = Rc::new(Cell::new(0_u32));
        let c = Rc::clone(&n);
        (Rc::new(move || c.set(c.get().saturating_add(1))), n)
    }

    /// Opus r19's nit (ADR-0035, acceptance 6): a registration a check made
    /// on its own list, escaped into app heap and dropped live, ends
    /// nowhere: its world expired with the check. No panic, nothing live
    /// touched.
    #[test]
    #[cfg(debug_assertions)]
    fn a_checks_wait_dropped_live_neither_panics_nor_touches_live() {
        let live = Rc::new(Waiters::default());
        let (m, _) = mark();
        let kept = live.wait(&m, EVERY, None, None);
        let escaped = crate::engine::in_cold_frame(|| {
            let own = Rc::new(Waiters::default());
            own.wait(&m, EVERY, None, None)
        });
        let before = live.sizes();
        drop(escaped);
        assert_eq!(live.sizes(), before, "nothing live touched");
        drop(kept);
        assert_eq!(live.sizes().0, 0);
    }

    /// A wake visits the mounts that wait now: grown to many and dropped
    /// to one, it visits one; slots freed are reused, not grown.
    #[test]
    fn a_wake_visits_the_live_mounts_alone() {
        let w = Rc::new(Waiters::default());
        let (m, woke) = mark();
        let mut waits: Vec<_> = (0..64).map(|_| w.wait(&m, EVERY, None, None)).collect();
        let last = waits.pop();
        drop(waits);
        w.visits.set(0);
        w.wake(EVERY);
        assert_eq!(w.visits.get(), 1, "one waiter visited");
        assert_eq!(woke.get(), 1, "and woken");
        assert!(
            last.as_ref().is_some_and(|w| w.woken()),
            "the one left is the one woken"
        );
        // Mounted and unmounted again and again: the same places.
        for _ in 0..10 {
            let again: Vec<_> = (0..63).map(|_| w.wait(&m, EVERY, None, None)).collect();
            drop(again);
        }
        assert_eq!(w.sizes(), (1, 64), "one waits; no place was added");
    }

    /// Chains in one list, their waiters interleaved and some gone: a wake
    /// of a chain visits the waiters of that chain left, and wakes them
    /// alone; a chain emptied goes.
    #[test]
    fn a_chain_wakes_its_own_waiters_alone() {
        let w = Rc::new(Waiters::default());
        let (a, woke_a) = mark();
        let (b, woke_b) = mark();
        let mut waits: Vec<_> = (0..12)
            .map(|k| {
                if k % 2 == 0 {
                    w.wait(&a, EVERY, Some(1), None)
                } else {
                    w.wait(&b, EVERY, Some(2), None)
                }
            })
            .collect();
        // Out of the middle, the head and the tail of each chain.
        for k in [11, 10, 5, 4, 1, 0] {
            drop(waits.remove(k));
        }
        w.visits.set(0);
        w.wake_chain(1, EVERY);
        assert_eq!(
            (w.visits.get(), woke_a.get(), woke_b.get()),
            (3, 3, 0),
            "chain 1's three left"
        );
        w.wake_chain(2, 1);
        assert_eq!(woke_b.get(), 3, "chain 2's three");
        drop(waits);
        assert!(w.list.with(|l| l.heads.is_empty()), "emptied, the chains go");
    }

    /// A group mounted on many nodes, then on one: a change visits the one
    /// mount left. Mounted and unmounted again and again, its list reuses
    /// the places it has.
    #[test]
    fn a_group_wakes_the_mounts_it_has_now() {
        use crate::element::{IntoElement, div};
        use crate::styled::Styled;
        let a = Signal::new(0.5_f32);
        let g = Group::new(move || a.get());
        let tree = |n: usize| {
            div()
                .children((0..n).map(|_| div().size(10.0).opacity(g.map(|v| *v))))
                .into_element()
        };
        let (many, one) = (tree(64), tree(1));
        let mut t = crate::layout::LayoutTree::new();
        let mut lay_out = |root: &crate::Element| {
            let mut ts = crate::text::TextSystem::new();
            t.layout(
                root,
                400.0,
                300.0,
                &crate::layout::UiState::default(),
                &mut crate::Motion::default(),
                &mut ts,
            );
        };
        lay_out(&many);
        lay_out(&one);
        let waiters = &g.0.hub.waiters;
        assert_eq!(waiters.sizes(), (1, 64), "one mount waits");
        waiters.visits.set(0);
        a.set(0.25);
        assert_eq!(waiters.visits.get(), 1, "the change visits it alone");
        lay_out(&one);
        for _ in 0..10 {
            lay_out(&many);
            lay_out(&one);
        }
        assert_eq!(waiters.sizes(), (1, 64), "no place was added");
    }

    /// A group whose read panicked once reads again next time: it never
    /// serves the value from before the change that made it panic.
    #[test]
    fn a_group_read_that_panicked_reads_again() {
        let s = Signal::new(1_u32);
        let boom = Rc::new(Cell::new(false));
        let b = Rc::clone(&boom);
        let g = Group::new(move || {
            let v = s.get();
            assert!(!b.get(), "a read panicked");
            v
        });
        assert_eq!(g.current(|v| *v), 1);
        s.set(2);
        boom.set(true);
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| g.current(|v| *v)));
        assert!(unwound.is_err());
        boom.set(false);
        assert_eq!(g.current(|v| *v), 2, "the new value, not the cached one");
    }
}
