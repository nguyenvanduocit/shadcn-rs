//! The reactive graph: a thread-local arena of nodes and links, and the
//! alien-signals algorithm over it as Vue 3.6 adopted it
//! (`references/alien-signals/src/system.ts`, `index.ts`;
//! `references/vue-minor/packages/reactivity/src/system.ts`). ADR-0012.
//!
//! Every node and link lives in one `RefCell<Runtime>`; no borrow of it is
//! held while user code runs (a computed's getter, an effect, a cleanup, a
//! value's `Drop`). The algorithms that touch only the graph (`link`,
//! `unlink`, `propagate`, `shallow_propagate`) run inside one borrow;
//! `check_dirty`, which recomputes computeds on its way, borrows per step.
//! Propagation and the dirty check are loops over an explicit stack kept in
//! the runtime, so they allocate nothing once it has grown and never recurse.

use std::any::Any;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::panic::Location;
use std::rc::Rc;

use super::trace::{ReactiveTrace, SignalRef};

/// No node or link.
pub(crate) const NONE: usize = usize::MAX;

/// The bit of a node's index that puts it in a cold frame's own region
/// (`Runtime::fork_nodes`); [`NONE`] is in no region's range.
const FORK_NODE: usize = 1 << (usize::BITS - 2);

/// The bit of a link's index that puts it in a cold frame's own region
/// (`Runtime::fork_links`), as [`FORK_NODE`] does for nodes: the live
/// links, their free list and their numbering are never changed by a
/// frame, only written in place under its log. Debug only, as forks are
/// until F2 (ADR-0028).
#[cfg(debug_assertions)]
const FORK_LINK: usize = 1 << (usize::BITS - 2);

/// alien-signals' `ReactiveFlags`, plus `HAS_CHILD`, which marks an owner
/// (effect, computed, scope) whose deps include a child effect or scope.
pub(crate) mod flag {
    pub(crate) const NONE: u8 = 0;
    pub(crate) const MUTABLE: u8 = 1;
    pub(crate) const WATCHING: u8 = 2;
    pub(crate) const RECURSED_CHECK: u8 = 4;
    pub(crate) const RECURSED: u8 = 8;
    pub(crate) const DIRTY: u8 = 16;
    pub(crate) const PENDING: u8 = 32;
    pub(crate) const HAS_CHILD: u8 = 64;
}

/// How many times one effect may run in one flush before it is taken for a
/// cycle and stopped (Vue's `RECURSION_LIMIT`).
const RECURSION_LIMIT: u32 = 100;

/// A node's slot and the generation it had when the handle was made: a slot
/// reused for another node has a new generation, so a stale handle is told
/// apart from a live one.
///
/// It names a node of this thread's arena, so it is neither `Send` nor
/// `Sync`, nor is any handle holding it (`Signal`, `Computed`, `Effect`,
/// `Scope`): moved to another thread, its slot and generation would name
/// that thread's node, a fork's dead handle aliasing a live one there
/// (Codex 1, F1 review 7), as a `SlabId` would ([`crate::engine::SlabId`]).
/// The type says it, so no access pays a thread check (compile-fail tests
/// on [`super::Signal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodeId {
    // (Slots with `FORK_NODE` set are a cold frame's region.)
    pub(crate) index: usize,
    pub(crate) generation: u32,
    thread: PhantomData<*const ()>,
}

/// A node's identity as plain data, for what crosses threads (the render
/// thread carries the instance that made a node, to route input back to
/// it): its slot and generation, and the thread whose arena they are in.
/// It is turned back into a node only on that thread ([`Self::node`]), so
/// on another it names nothing rather than that thread's node of the same
/// slot (Codex 1, F1 review 7). The arena is named by its thread's serial
/// ([`crate::engine::arena`]: an id, never handed out twice in a process),
/// read from the thread's own slots, with no `thread::current()` handle
/// cloned per call (Opus nit 6, F1 review 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct OwnerId {
    index: usize,
    generation: u32,
    arena: u64,
}

crate::engine::parts_whole!(OwnerId);

impl OwnerId {
    /// The node it names, on the thread whose arena it is in; else none.
    pub(crate) fn node(self) -> Option<NodeId> {
        (self.arena == crate::engine::arena()).then(|| NodeId::new(self.index, self.generation))
    }
}

impl NodeId {
    pub(crate) const fn new(index: usize, generation: u32) -> Self {
        Self {
            index,
            generation,
            thread: PhantomData,
        }
    }

    /// Its identity as plain data that may cross threads.
    pub(crate) fn owner_id(self) -> OwnerId {
        OwnerId {
            index: self.index,
            generation: self.generation,
            arena: crate::engine::arena(),
        }
    }

    /// One number for the devtools: the generation above the slot.
    pub(crate) fn as_u64(self) -> u64 {
        let index = u64::try_from(self.index).unwrap_or(u64::MAX) & 0xFFFF_FFFF;
        u64::from(self.generation).wrapping_shl(32) | index
    }

    /// Whether a fork made it (its generation is the forks' space, its
    /// slot in the forks' region, `FORK_NODE`).
    pub(crate) fn made_in_fork(self) -> bool {
        self.generation & crate::engine::FORK_GENERATION != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Signal,
    Computed,
    Effect,
    Scope,
    /// A component's render (ADR-0013): notified like an effect, but a
    /// change only marks it (its `run`), and the tracked run happens when the
    /// frame resolves the instance.
    Render,
}

/// A computed's recomputation: runs the getter, stores the value, says
/// whether it changed.
pub(crate) type Update = Rc<dyn Fn() -> bool>;
/// An effect's body.
pub(crate) type Run = Rc<dyn Fn()>;
/// A signal's check, made for its type by the first `set`: whether its value
/// differs from the one it had before it was written, forgetting that one.
pub(crate) type Check = fn(&dyn Any) -> bool;
type Cleanup = Box<dyn FnOnce()>;

pub(crate) struct Node {
    pub(crate) generation: u32,
    pub(crate) alive: bool,
    pub(crate) kind: Kind,
    pub(crate) flags: u8,
    pub(crate) deps: usize,
    pub(crate) deps_tail: usize,
    pub(crate) subs: usize,
    pub(crate) subs_tail: usize,
    /// A signal's or computed's value, as `Rc<RefCell<T>>`.
    pub(crate) value: Option<Rc<dyn Any>>,
    pub(crate) update: Option<Update>,
    pub(crate) check: Option<Check>,
    pub(crate) run: Option<Run>,
    /// What `on_cleanup` registered during the last run (effect) or the
    /// scope's life (scope).
    pub(crate) cleanups: Vec<Cleanup>,
    /// Signals, computeds and collections made while this node ran or was the
    /// owner; disposed with it, and before an effect or computed runs again.
    pub(crate) owned: Vec<NodeId>,
    pub(crate) created_at: &'static Location<'static>,
    pub(crate) label: Option<&'static str>,
    flush_epoch: u32,
    flush_runs: u32,
    /// The signal whose write last notified it (an effect or render: why
    /// it runs; devtools' "why did this render").
    woken_by: usize,
    /// The frame ([`Runtime::frame`]) its last tracked read was in, and the
    /// phases that read it in that frame (bits of [`Phase`]): the late-write
    /// check (ADR-0013).
    read_frame: u32,
    read_phases: u8,
    /// The render ([`Runtime::rendering`]) it was made in, `0` for none: a
    /// render may write what it made itself, and nothing else.
    #[cfg(debug_assertions)]
    born_in: u32,
}

impl Node {
    fn new(kind: Kind, flags: u8, created_at: &'static Location<'static>) -> Self {
        Self {
            generation: 0,
            alive: true,
            kind,
            flags,
            deps: NONE,
            deps_tail: NONE,
            subs: NONE,
            subs_tail: NONE,
            value: None,
            update: None,
            check: None,
            run: None,
            cleanups: Vec::new(),
            owned: Vec::new(),
            created_at,
            label: None,
            flush_epoch: 0,
            flush_runs: 0,
            woken_by: NONE,
            read_frame: 0,
            read_phases: 0,
            #[cfg(debug_assertions)]
            born_in: 0,
        }
    }

    /// Whether a cold frame's version of a node, replaced at the switch,
    /// holds nothing whose drop is user code (ADR-0034, S1): no cleanup,
    /// and, if the frame owned its value (`owned`: it gave the node one of
    /// its own, or made it), no value or closure; one it only read holds
    /// handles shared with the live node put back.
    #[cfg(debug_assertions)]
    fn is_shell(&self, owned: bool) -> bool {
        self.cleanups.is_empty() && (!owned || (self.value.is_none() && self.update.is_none() && self.run.is_none()))
    }

    /// This node as a cold frame writes it, the live one kept in the undo
    /// log ([`ColdLog`]): the same, its value shared (never written in
    /// place: [`cold_owns`]); no cleanup of the live one's (the frame runs
    /// none, and the live ones go back with the node).
    #[cfg(debug_assertions)]
    fn cold_copy(&self) -> Self {
        Self {
            generation: self.generation,
            alive: self.alive,
            kind: self.kind,
            flags: self.flags,
            deps: self.deps,
            deps_tail: self.deps_tail,
            subs: self.subs,
            subs_tail: self.subs_tail,
            value: self.value.clone(),
            update: self.update.clone(),
            check: self.check,
            run: self.run.clone(),
            cleanups: Vec::new(),
            owned: self.owned.clone(),
            created_at: self.created_at,
            label: self.label,
            flush_epoch: self.flush_epoch,
            flush_runs: self.flush_runs,
            woken_by: self.woken_by,
            read_frame: self.read_frame,
            read_phases: self.read_phases,
            born_in: self.born_in,
        }
    }
}

/// The phase of a frame the UI thread runs now (ADR-0013's phases, Chrome's
/// document lifecycle): a signal read in one phase and written in a later
/// one of the same frame left what the earlier phase made stale.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Between frames: input, timers, effects. Writes here start the next frame.
    #[default]
    Idle,
    /// The effects a render or setup made, run once it returns: what they
    /// read is no render's (their writes are theirs to make).
    Effect,
    /// Component renders.
    Render,
    /// Layout: patching the retained tree, bindings, placing nodes.
    Layout,
    /// Recording paint chunks.
    Paint,
    /// Building what a screen reader is sent.
    Access,
}

impl Phase {
    fn bit(self) -> u8 {
        match self {
            Self::Idle | Self::Effect => 0,
            Self::Render => 1,
            Self::Layout => 2,
            Self::Paint => 4,
            Self::Access => 8,
        }
    }

    /// The phases before this one.
    fn earlier(self) -> u8 {
        self.bit().wrapping_sub(1) & !Self::Idle.bit()
    }

    fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Effect => "effect",
            Self::Render => "render",
            Self::Layout => "layout",
            Self::Paint => "paint",
            Self::Access => "accessibility",
        }
    }

    fn of_bits(bits: u8) -> Self {
        [Self::Render, Self::Layout, Self::Paint, Self::Access]
            .into_iter()
            .find(|p| bits & p.bit() != 0)
            .unwrap_or(Self::Idle)
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A signal written in a phase after an earlier phase of the same frame
/// read it (Chrome's lifecycle asserts): what the earlier phase made from
/// it is stale on screen until another frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LateWrite {
    pub signal: SignalRef,
    /// Where the signal was made.
    pub created_at: &'static Location<'static>,
    /// The earliest phase that read it this frame.
    pub read_in: Phase,
    pub written_in: Phase,
}

impl std::fmt::Display for LateWrite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (made at {}) was written during {} after {} read it in the same frame",
            self.signal, self.created_at, self.written_in, self.read_in
        )
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Link {
    version: u32,
    dep: usize,
    sub: usize,
    prev_sub: usize,
    next_sub: usize,
    prev_dep: usize,
    next_dep: usize,
}

const FREE_LINK: Link = Link {
    version: 0,
    dep: NONE,
    sub: NONE,
    prev_sub: NONE,
    next_sub: NONE,
    prev_dep: NONE,
    next_dep: NONE,
};

/// What leaves the graph and must run or drop after the borrow ends: user
/// cleanups, and values and closures whose `Drop` may be user code.
#[derive(Default)]
pub(crate) struct Settle {
    cleanups: Vec<Cleanup>,
    garbage: Vec<Garbage>,
}

/// A value or closure let go by a disposed node, held as it was (no box of
/// its own: a render that ends thousands of signals allocates nothing for it).
enum Garbage {
    Value(#[expect(dead_code, reason = "held only to be dropped after the borrow ends")] Rc<dyn Any>),
    Update(#[expect(dead_code, reason = "held only to be dropped after the borrow ends")] Update),
    Run(#[expect(dead_code, reason = "held only to be dropped after the borrow ends")] Run),
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Counts {
    pub(crate) signals: usize,
    pub(crate) computeds: usize,
    pub(crate) effects: usize,
    pub(crate) scopes: usize,
    pub(crate) renders: usize,
    pub(crate) links: usize,
}

impl Counts {
    /// One more node of `kind`.
    fn count(&mut self, kind: Kind) {
        let n = match kind {
            Kind::Signal => &mut self.signals,
            Kind::Computed => &mut self.computeds,
            Kind::Effect => &mut self.effects,
            Kind::Scope => &mut self.scopes,
            Kind::Render => &mut self.renders,
        };
        *n = n.saturating_add(1);
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    nodes: Vec<Node>,
    /// Each slot's next generations, live and a fork's, by the one slot
    /// rule of every arena ([`crate::engine::Generations`], ADR-0028 rule
    /// 3): each only grows, a cold frame's rollback included, which neither
    /// logs nor truncates them. So no handle is named twice, live or a
    /// fork's (Codex 3, Opus 1, F1 review 4), and the live ids are as
    /// without the frame (F1 review 5).
    generations: crate::engine::Generations,
    /// The nodes a cold frame makes, in a region of their own (indices
    /// `FORK_NODE | k`): the live slots, the free ones and their numbering
    /// are as without the frame, so what live code makes after it (a
    /// Shared piece's init, run once the fork rolled back) takes the very
    /// id a run with no check gives. Emptied by each rollback; their next
    /// generations (the forks' space) never put back.
    #[cfg(debug_assertions)]
    fork_nodes: Vec<Node>,
    #[cfg(debug_assertions)]
    fork_generations: crate::engine::Generations,
    free_nodes: Vec<usize>,
    links: Vec<Link>,
    free_links: Vec<usize>,
    /// The links a cold frame makes (indices `FORK_LINK | k`), and those of
    /// them it freed; emptied by each rollback.
    #[cfg(debug_assertions)]
    fork_links: Vec<Link>,
    #[cfg(debug_assertions)]
    fork_free_links: Vec<usize>,
    pub(crate) cycle: u32,
    pub(crate) run_depth: u32,
    pub(crate) batch_depth: u32,
    pub(crate) active_sub: usize,
    /// The node that owns what is made now: an effect or computed running, or
    /// a scope inside `Scope::run`.
    pub(crate) active_owner: Option<NodeId>,
    queued: Vec<usize>,
    notify_index: usize,
    flushing: bool,
    epoch: u32,
    /// The explicit stack `propagate` and `check_dirty` use; nested calls
    /// work above the length they found.
    stack: Vec<usize>,
    unwatched_stack: Vec<usize>,
    unwatching: bool,
    pub(crate) settle: Settle,
    pub(crate) counts: Counts,
    pub(crate) trace: Option<ReactiveTrace>,
    /// The signal being written while its subscribers are notified.
    writer: usize,
    /// While a render's mark runs: the signal whose write woke it.
    pub(crate) waking: Option<SignalRef>,
    /// The frame running now and its phase (the late-write check). In the
    /// layout phase, `settles` says another pass follows a write that
    /// dirties what was read (the frame's layout loop): such a write is not late.
    pub(crate) frame: u32,
    pub(crate) phase: Phase,
    pub(crate) settles: bool,
    pub(crate) late: Vec<LateWrite>,
    /// While a cold frame runs: its undo log ([`ColdLog`]).
    #[cfg(debug_assertions)]
    cold: Option<Box<ColdLog>>,
    /// While a component's render or setup runs ([`rendering`]), or a
    /// computed's getter: its number (from 1, one per run) and what runs. A
    /// render is pure and a getter too, so neither writes a live signal but
    /// those it made itself.
    #[cfg(debug_assertions)]
    rendering: Option<(u32, Pure)>,
    #[cfg(debug_assertions)]
    renders_run: u32,
    /// Renders and setups running ([`rendering`]): an effect made in one
    /// waits in `deferred` for it to return (Vue's pre-flush watchers).
    in_render: u32,
    deferred: Vec<NodeId>,
    /// How deep Shared inits run ([`making_shared`]), and what they made
    /// so far: kept only when the outermost returns.
    making: u32,
    made: Vec<NodeId>,
}

/// What runs that must write nothing live: a component's render or setup
/// (its name), or a computed's getter.
#[cfg(debug_assertions)]
#[derive(Clone, Copy)]
enum Pure {
    Render(&'static str),
    Getter,
}

crate::engine::engine_local! {
    /// The reactive arena. A cold frame writes it in place, each slot's
    /// live state kept on its first write and put back at the end.
    [Fork(cold_enter, cold_drain, cold_exit)] static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime {
        active_sub: NONE,
        writer: NONE,
        frame: 1,
        ..Runtime::default()
    });
}

impl Runtime {
    /// The bytes the arena holds: its nodes, links, free lists, queue and
    /// stacks at their capacity (what signals' values own aside).
    /// The cleanups the live nodes hold, run as they are disposed.
    pub(crate) fn cleanup_count(&self) -> usize {
        self.nodes.iter().filter(|n| n.alive).map(|n| n.cleanups.len()).sum()
    }

    pub(crate) fn held_bytes(&self) -> u64 {
        fn of<T>(v: &Vec<T>) -> u64 {
            u64::try_from(v.capacity().saturating_mul(std::mem::size_of::<T>())).unwrap_or(u64::MAX)
        }
        [
            of(&self.nodes),
            self.generations.held_bytes(),
            of(&self.free_nodes),
            of(&self.links),
            of(&self.free_links),
            of(&self.queued),
            of(&self.stack),
            of(&self.unwatched_stack),
        ]
        .iter()
        .fold(0, |n, b| n.saturating_add(*b))
    }
}

/// Run `f` on this thread's runtime: every access to it goes through here
/// or [`try_with`], so R4 has its one gate here (ADR-0028, F1 review 12): a
/// Shared init reads nothing of the runtime (a node, its liveness or value,
/// a count, the graph, a trace), as a fork changes all of it. The engine's
/// own bookkeeping making a handle passes ([`crate::engine::allocating`]),
/// and gives the init nothing but the handle.
pub(crate) fn with<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    #[cfg(debug_assertions)]
    crate::engine::no_forked_read("the reactive runtime (a node, a count, the graph)");
    RUNTIME.with_arena(|rt| f(&mut rt.borrow_mut()))
}

/// Run `f` on this thread's runtime unless the thread is ending.
pub(crate) fn try_with<R>(f: impl FnOnce(&mut Runtime) -> R) -> Option<R> {
    #[cfg(debug_assertions)]
    crate::engine::no_forked_read("the reactive runtime (a node, a count, the graph)");
    RUNTIME
        .try_with_arena(|rt| rt.try_borrow_mut().ok().map(|mut rt| f(&mut rt)))
        .ok()
        .flatten()
}

/// Run what left the graph: cleanups in the order they were queued (each may
/// queue more), then drop what was let go.
pub(crate) fn settle() {
    // Every round runs, though a cleanup of one panics: what a cleanup
    // queued is let go too, and the first panic goes on once all have.
    // Inside a turn, each to its arbiter as it is caught.
    let mut all = crate::engine::Gather::new();
    // Live, what a fork left to run live runs here, as cleanups do
    // ([`crate::engine::run_releases`]); none while a fork runs.
    if let Some(p) = crate::engine::released() {
        all.add(p);
    }
    loop {
        let Some(Settle { cleanups, garbage }) = try_with(|rt| std::mem::take(&mut rt.settle)) else {
            break;
        };
        if cleanups.is_empty() && garbage.is_empty() {
            break;
        }
        // A cold frame runs no cleanup: what it registered is dropped.
        #[cfg(debug_assertions)]
        if crate::engine::is_cold() {
            drop(cleanups);
            drop(garbage);
            continue;
        }
        // Every one runs, though one panics (taken out, a cleanup a panic
        // skipped would never run). A user's goes through the gate in its
        // window, in a turn (`on_cleanup`); the engine's own runs as is.
        for cleanup in cleanups {
            all.run(|| untracked_call(cleanup));
        }
        drop(garbage);
    }
    all.finish();
}

/// Run `f` with no subscriber, so what it reads tracks nothing.
fn untracked_call(f: impl FnOnce()) {
    let _guard = OwnerGuard::enter(None, false);
    f();
}

/// Clears the running subscriber, and sets the owner if asked, until dropped
/// (on return or unwind).
pub(crate) struct OwnerGuard {
    prev_sub: usize,
    prev_owner: Option<Option<NodeId>>,
}

/// Its steps are the engine's own scoping, which a Shared init may take
/// (`detached`, `untracked`): what they read goes back into the runtime,
/// never to the caller ([`crate::engine::allocating`]).
impl OwnerGuard {
    pub(crate) fn enter(owner: Option<NodeId>, set_owner: bool) -> Self {
        crate::engine::allocating(|| {
            with(|rt| Self {
                prev_sub: std::mem::replace(&mut rt.active_sub, NONE),
                prev_owner: set_owner.then(|| std::mem::replace(&mut rt.active_owner, owner)),
            })
        })
    }
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        let (sub, owner) = (self.prev_sub, self.prev_owner);
        crate::engine::allocating(|| {
            try_with(|rt| {
                rt.active_sub = sub;
                if let Some(owner) = owner {
                    rt.active_owner = owner;
                }
            });
        });
    }
}

/// A misuse of the API that cannot be recovered from (reading a value while
/// it is being changed); the message says what happened and where.
#[expect(clippy::panic, reason = "misuse is a bug; the report names it")]
pub(crate) fn misuse(message: &str) -> ! {
    panic!("{message}")
}

/// Using a handle whose node was disposed: a programming error, reported with
/// where the handle was made.
#[expect(
    clippy::panic,
    reason = "a disposed handle is a bug; the report names where it was made"
)]
pub(crate) fn disposed(what: &str, created_at: &'static Location<'static>) -> ! {
    panic!("this {what} was disposed (its owner scope ended); it was created at {created_at}")
}

impl Runtime {
    // ── Arena ────────────────────────────────────────────────────────────

    /// Slot `i`, in the live region or a cold frame's.
    fn slot(&self, i: usize) -> Option<&Node> {
        if i & FORK_NODE != 0 {
            #[cfg(debug_assertions)]
            return self.fork_nodes.get(i & !FORK_NODE);
            #[cfg(not(debug_assertions))]
            return None;
        }
        self.nodes.get(i)
    }

    fn slot_mut(&mut self, i: usize) -> Option<&mut Node> {
        if i & FORK_NODE != 0 {
            #[cfg(debug_assertions)]
            return self.fork_nodes.get_mut(i & !FORK_NODE);
            #[cfg(not(debug_assertions))]
            return None;
        }
        self.nodes.get_mut(i)
    }

    pub(crate) fn node(&self, i: usize) -> Option<&Node> {
        self.slot(i)
    }

    pub(crate) fn node_mut(&mut self, i: usize) -> Option<&mut Node> {
        self.log_node(i);
        self.slot_mut(i)
    }

    /// The live node `id` names, if it was not disposed.
    pub(crate) fn live(&self, id: NodeId) -> Option<&Node> {
        self.slot(id.index).filter(|n| n.alive && n.generation == id.generation)
    }

    pub(crate) fn live_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        if self.live(id).is_some() {
            self.log_node(id.index);
        }
        self.slot_mut(id.index)
            .filter(|n| n.alive && n.generation == id.generation)
    }

    pub(crate) fn id_of(&self, i: usize) -> Option<NodeId> {
        self.slot(i).filter(|n| n.alive).map(|n| NodeId::new(i, n.generation))
    }

    /// How many subscribers node `i` has.
    pub(crate) fn sub_count(&self, i: usize) -> usize {
        let mut n: usize = 0;
        let mut link = self.node(i).map_or(NONE, |n| n.subs);
        while link != NONE {
            n = n.saturating_add(1);
            link = self.l(link).next_sub;
        }
        n
    }

    fn flags(&self, i: usize) -> u8 {
        self.slot(i).map_or(flag::NONE, |n| n.flags)
    }

    fn set_flags(&mut self, i: usize, flags: u8) {
        if let Some(n) = self.node_mut(i) {
            n.flags = flags;
        }
    }

    /// A new node owned by the active owner.
    pub(crate) fn alloc(&mut self, kind: Kind, flags: u8, created_at: &'static Location<'static>) -> NodeId {
        let node = Node::new(kind, flags, created_at);
        let id = self.alloc_slot(node);
        #[cfg(debug_assertions)]
        if let Some((run, _)) = self.rendering
            && let Some(n) = self.node_mut(id.index)
        {
            n.born_in = run;
        }
        self.counts.count(kind);
        if self.making > 0 {
            self.made.push(id);
        }
        id
    }

    /// Undo `id`, made by a Shared init that failed: disposed.
    fn unmake(&mut self, id: NodeId) {
        self.dispose(id);
    }

    /// Whether slot `index` may hold another node (its generations did not
    /// run out).
    fn reusable(&self, index: usize) -> bool {
        self.generations.reusable(index)
    }

    /// The index the next pushed live node takes, with its generation:
    /// retired slots past the end are pushed dead first.
    fn push_index(&mut self, created_at: &'static Location<'static>) -> (usize, u32) {
        loop {
            let index = self.nodes.len();
            if let Some(generation) = self.generations.take(index, false) {
                return (index, generation);
            }
            let mut retired = Node::new(Kind::Signal, flag::NONE, created_at);
            retired.alive = false;
            self.nodes.push(retired);
        }
    }

    /// Whether a cold frame runs: what is done now is the frame's. The
    /// engine's answer, which a fork never leaves until it ends: it runs
    /// no live code (ADR-0028). Read as `is_cold`, not the public
    /// `engine::forking`: a Shared init allocates here, and may not ask
    /// whether a fork runs ([`crate::engine::shared_init`]).
    #[cfg(debug_assertions)]
    fn forking(&self) -> bool {
        self.cold.is_some() && crate::engine::is_cold()
    }

    fn alloc_slot(&mut self, node: Node) -> NodeId {
        // A cold frame's (the one answer, at the moment it is made): in its
        // own region, under the forks' generations, which a rollback never
        // puts back (ADR-0028, rule 3), so a handle a frame hands out
        // names nothing after it, in a live run or another frame.
        #[cfg(debug_assertions)]
        if self.forking() {
            loop {
                let k = self.fork_nodes.len();
                if let Some(generation) = self.fork_generations.take(k, true) {
                    self.fork_nodes.push(Node { generation, ..node });
                    return NodeId::new(k | FORK_NODE, generation);
                }
                let mut retired = Node::new(Kind::Signal, flag::NONE, node.created_at);
                retired.alive = false;
                self.fork_nodes.push(retired);
            }
        }
        // Live: exactly as without any frame, its slot and generation the
        // very ones a run with no check takes.
        while let Some(index) = self.free_nodes.pop() {
            let Some(generation) = self.generations.take(index, false) else {
                continue;
            };
            if let Some(slot) = self.nodes.get_mut(index) {
                // A freed slot holds no lists (`dispose_index` took them).
                *slot = Node { generation, ..node };
                return NodeId::new(index, generation);
            }
        }
        let (index, generation) = self.push_index(node.created_at);
        self.nodes.push(Node { generation, ..node });
        NodeId::new(index, generation)
    }

    /// The room the free slots' `owned` lists hold (none: a list goes with
    /// its node).
    #[cfg(test)]
    pub(crate) fn free_owned_capacity(&self) -> usize {
        self.free_nodes
            .iter()
            .filter_map(|&i| self.nodes.get(i))
            .map(|n| n.owned.capacity())
            .sum()
    }

    /// Record `id` as owned by the active owner: effects and scopes are linked
    /// as its deps (so the owner runs first and disposes them when it runs
    /// again), everything else joins its `owned` list.
    pub(crate) fn adopt(&mut self, id: NodeId) {
        let Some(owner) = self.active_owner.filter(|o| self.live(*o).is_some()) else {
            return;
        };
        let kind = self.live(id).map(|n| n.kind);
        match kind {
            Some(Kind::Effect | Kind::Scope) => {
                self.link(id.index, owner.index, 0);
                if let Some(o) = self.node_mut(owner.index) {
                    o.flags |= flag::HAS_CHILD;
                }
            }
            Some(_) => {
                if let Some(o) = self.node_mut(owner.index) {
                    o.owned.push(id);
                }
            }
            None => {}
        }
    }

    fn alloc_link(&mut self, link: Link) -> usize {
        self.counts.links = self.counts.links.saturating_add(1);
        // A cold frame's in its own region (as its nodes are): the live
        // free list and numbering are as without it.
        #[cfg(debug_assertions)]
        if self.forking() {
            if let Some(k) = self.fork_free_links.pop()
                && let Some(slot) = self.fork_links.get_mut(k)
            {
                *slot = link;
                return k | FORK_LINK;
            }
            self.fork_links.push(link);
            return self.fork_links.len().saturating_sub(1) | FORK_LINK;
        }
        if let Some(i) = self.free_links.pop()
            && let Some(slot) = self.links.get_mut(i)
        {
            *slot = link;
            return i;
        }
        self.links.push(link);
        self.links.len().saturating_sub(1)
    }

    fn free_link(&mut self, i: usize) {
        #[cfg(debug_assertions)]
        if i != NONE && i & FORK_LINK != 0 {
            let k = i & !FORK_LINK;
            if let Some(slot) = self.fork_links.get_mut(k) {
                *slot = FREE_LINK;
                self.fork_free_links.push(k);
                self.counts.links = self.counts.links.saturating_sub(1);
            }
            return;
        }
        self.log_link(i);
        // A live link a running frame frees is not handed to the live free
        // list (the log puts it back).
        #[cfg(debug_assertions)]
        let freed = !self.forking();
        #[cfg(not(debug_assertions))]
        let freed = true;
        if let Some(slot) = self.links.get_mut(i) {
            *slot = FREE_LINK;
            if freed {
                self.free_links.push(i);
            }
            self.counts.links = self.counts.links.saturating_sub(1);
        }
    }

    fn l(&self, i: usize) -> Link {
        #[cfg(debug_assertions)]
        if i != NONE && i & FORK_LINK != 0 {
            return self.fork_links.get(i & !FORK_LINK).copied().unwrap_or(FREE_LINK);
        }
        self.links.get(i).copied().unwrap_or(FREE_LINK)
    }

    fn l_mut(&mut self, i: usize) -> Option<&mut Link> {
        #[cfg(debug_assertions)]
        if i != NONE && i & FORK_LINK != 0 {
            return self.fork_links.get_mut(i & !FORK_LINK);
        }
        self.log_link(i);
        self.links.get_mut(i)
    }

    // ── alien-signals: system.ts ─────────────────────────────────────────

    pub(crate) fn link(&mut self, dep: usize, sub: usize, version: u32) {
        let Some(s) = self.node(sub) else { return };
        let prev_dep = s.deps_tail;
        let sub_deps = s.deps;
        if prev_dep != NONE && self.l(prev_dep).dep == dep {
            return;
        }
        let next_dep = if prev_dep == NONE {
            sub_deps
        } else {
            self.l(prev_dep).next_dep
        };
        if next_dep != NONE && self.l(next_dep).dep == dep {
            if let Some(l) = self.l_mut(next_dep) {
                l.version = version;
            }
            if let Some(s) = self.node_mut(sub) {
                s.deps_tail = next_dep;
            }
            return;
        }
        let Some(d) = self.node(dep) else { return };
        let prev_sub = d.subs_tail;
        if prev_sub != NONE {
            let p = self.l(prev_sub);
            if p.version == version && p.sub == sub {
                return;
            }
        }
        let new = self.alloc_link(Link {
            version,
            dep,
            sub,
            prev_dep,
            next_dep,
            prev_sub,
            next_sub: NONE,
        });
        if let Some(s) = self.node_mut(sub) {
            s.deps_tail = new;
        }
        if let Some(d) = self.node_mut(dep) {
            d.subs_tail = new;
        }
        if next_dep != NONE
            && let Some(l) = self.l_mut(next_dep)
        {
            l.prev_dep = new;
        }
        if prev_dep != NONE {
            if let Some(l) = self.l_mut(prev_dep) {
                l.next_dep = new;
            }
        } else if let Some(s) = self.node_mut(sub) {
            s.deps = new;
        }
        if prev_sub != NONE {
            if let Some(l) = self.l_mut(prev_sub) {
                l.next_sub = new;
            }
        } else if let Some(d) = self.node_mut(dep) {
            d.subs = new;
        }
    }

    /// Remove `link` from both lists; its next dep. A dep left with no
    /// subscriber is unwatched.
    pub(crate) fn unlink(&mut self, link: usize) -> usize {
        let Link {
            dep,
            sub,
            prev_dep,
            next_dep,
            prev_sub,
            next_sub,
            ..
        } = self.l(link);
        if next_dep != NONE {
            if let Some(l) = self.l_mut(next_dep) {
                l.prev_dep = prev_dep;
            }
        } else if let Some(s) = self.node_mut(sub) {
            s.deps_tail = prev_dep;
        }
        if prev_dep != NONE {
            if let Some(l) = self.l_mut(prev_dep) {
                l.next_dep = next_dep;
            }
        } else if let Some(s) = self.node_mut(sub) {
            s.deps = next_dep;
        }
        if next_sub != NONE {
            if let Some(l) = self.l_mut(next_sub) {
                l.prev_sub = prev_sub;
            }
        } else if let Some(d) = self.node_mut(dep) {
            d.subs_tail = prev_sub;
        }
        let mut unwatched = false;
        if prev_sub != NONE {
            if let Some(l) = self.l_mut(prev_sub) {
                l.next_sub = next_sub;
            }
        } else if let Some(d) = self.node_mut(dep) {
            d.subs = next_sub;
            unwatched = next_sub == NONE;
        }
        self.free_link(link);
        if unwatched {
            self.unwatched(dep);
        }
        next_dep
    }

    /// `dep` lost its last subscriber. A computed lets go of its deps (it is
    /// lazy again, recomputed when next read); a chain of computeds unwinds
    /// through a stack, not recursion. An effect or scope was a child of the
    /// owner that let go of it, and is disposed.
    fn unwatched(&mut self, dep: usize) {
        let Some(node) = self.node(dep) else { return };
        match node.kind {
            Kind::Computed => {
                self.unwatched_stack.push(dep);
                if self.unwatching {
                    return;
                }
                self.unwatching = true;
                while let Some(c) = self.unwatched_stack.pop() {
                    if self.node(c).is_some_and(|n| n.alive && n.deps_tail != NONE) {
                        self.set_flags(c, flag::MUTABLE | flag::DIRTY);
                        self.dispose_deps_in_reverse(c);
                    }
                }
                self.unwatching = false;
            }
            Kind::Effect | Kind::Scope => self.dispose_index(dep),
            Kind::Signal | Kind::Render => {}
        }
    }

    pub(crate) fn propagate(&mut self, start: usize, inner_write: bool) {
        let base = self.stack.len();
        let mut link = start;
        let mut next = self.l(link).next_sub;
        'top: loop {
            let sub = self.l(link).sub;
            let mut flags = self.flags(sub);
            if flags & (flag::RECURSED_CHECK | flag::RECURSED | flag::DIRTY | flag::PENDING) == 0 {
                let mut f = flags | flag::PENDING;
                if inner_write {
                    f |= flag::RECURSED;
                }
                self.set_flags(sub, f);
            } else if flags & (flag::RECURSED_CHECK | flag::RECURSED) == 0 {
                flags = flag::NONE;
            } else if flags & flag::RECURSED_CHECK == 0 {
                self.set_flags(sub, (flags & !flag::RECURSED) | flag::PENDING);
            } else if flags & (flag::DIRTY | flag::PENDING) == 0 && self.is_valid_link(link, sub) {
                self.set_flags(sub, flags | flag::RECURSED | flag::PENDING);
                flags &= flag::MUTABLE;
            } else {
                flags = flag::NONE;
            }

            if flags & flag::WATCHING != 0 {
                self.notify(sub);
            }

            if flags & flag::MUTABLE != 0 {
                let sub_subs = self.node(sub).map_or(NONE, |n| n.subs);
                if sub_subs != NONE {
                    link = sub_subs;
                    let next_sub = self.l(link).next_sub;
                    if next_sub != NONE {
                        self.stack.push(next);
                        next = next_sub;
                    }
                    continue;
                }
            }

            link = next;
            if link != NONE {
                next = self.l(link).next_sub;
                continue;
            }

            while self.stack.len() > base {
                link = self.stack.pop().unwrap_or(NONE);
                if link != NONE {
                    next = self.l(link).next_sub;
                    continue 'top;
                }
            }
            break;
        }
        self.stack.truncate(base);
    }

    pub(crate) fn shallow_propagate(&mut self, start: usize) {
        let mut link = start;
        while link != NONE {
            let sub = self.l(link).sub;
            let flags = self.flags(sub);
            if flags & (flag::PENDING | flag::DIRTY) == flag::PENDING {
                self.set_flags(sub, flags | flag::DIRTY);
                if flags & (flag::WATCHING | flag::RECURSED_CHECK) == flag::WATCHING {
                    self.notify(sub);
                }
            }
            link = self.l(link).next_sub;
        }
    }

    fn is_valid_link(&self, check: usize, sub: usize) -> bool {
        let mut link = self.node(sub).map_or(NONE, |n| n.deps_tail);
        while link != NONE {
            if link == check {
                return true;
            }
            link = self.l(link).prev_dep;
        }
        false
    }

    // ── alien-signals: index.ts ──────────────────────────────────────────

    /// Queue `effect`, and every watching effect that owns it, owners first.
    fn notify(&mut self, effect: usize) {
        let writer = self.writer;
        if let Some(n) = self.node_mut(effect) {
            n.woken_by = writer;
        }
        let first = self.queued.len();
        let mut e = effect;
        loop {
            self.queued.push(e);
            let f = self.flags(e);
            self.set_flags(e, f & !flag::WATCHING);
            let subs = self.node(e).map_or(NONE, |n| n.subs);
            if subs == NONE {
                break;
            }
            e = self.l(subs).sub;
            if self.node(e).is_none_or(|n| n.kind != Kind::Effect) || self.flags(e) & flag::WATCHING == 0 {
                break;
            }
        }
        if let Some(inserted) = self.queued.get_mut(first..) {
            inserted.reverse();
        }
    }

    fn dispose_deps_in_reverse(&mut self, sub: usize) {
        let mut link = self.node(sub).map_or(NONE, |n| n.deps_tail);
        while link != NONE {
            let prev = self.l(link).prev_dep;
            self.unlink(link);
            link = prev;
        }
    }

    /// Unlink the deps of `sub` that are child effects or scopes, last made
    /// first, disposing them, before `sub` runs again.
    fn dispose_children(&mut self, sub: usize) {
        if self.flags(sub) & flag::HAS_CHILD == 0 {
            return;
        }
        let mut link = self.node(sub).map_or(NONE, |n| n.deps_tail);
        while link != NONE {
            let Link { prev_dep, dep, .. } = self.l(link);
            if self
                .node(dep)
                .is_some_and(|n| matches!(n.kind, Kind::Effect | Kind::Scope))
            {
                self.unlink(link);
            }
            link = prev_dep;
        }
    }

    /// Before `sub` runs again: its child effects and scopes, then the nodes
    /// its last run made, then its own cleanups.
    pub(crate) fn before_rerun(&mut self, sub: usize) {
        self.dispose_children(sub);
        self.dispose_owned(sub);
        if let Some(n) = self.node_mut(sub) {
            let cleanups = std::mem::take(&mut n.cleanups);
            self.settle.cleanups.extend(cleanups);
        }
    }

    fn dispose_owned(&mut self, sub: usize) {
        let mut owned = self
            .node_mut(sub)
            .map(|n| std::mem::take(&mut n.owned))
            .unwrap_or_default();
        for id in owned.iter().rev() {
            if self.live(*id).is_some() {
                self.dispose_index(id.index);
            }
        }
        // Give the list back, emptied, so its capacity is reused.
        owned.clear();
        if let Some(n) = self.node_mut(sub)
            && n.owned.is_empty()
        {
            n.owned = owned;
        }
    }

    pub(crate) fn purge_deps(&mut self, sub: usize) {
        let Some(n) = self.node(sub) else { return };
        let mut dep = if n.deps_tail == NONE {
            n.deps
        } else {
            self.l(n.deps_tail).next_dep
        };
        while dep != NONE {
            dep = self.unlink(dep);
        }
    }

    /// Dispose `id`'s node if it is alive.
    pub(crate) fn dispose(&mut self, id: NodeId) {
        if self.live(id).is_some() {
            self.dispose_index(id.index);
        }
    }

    /// Take node `i` out of the graph: its children and owned nodes first
    /// (last made first), then its subscriptions and its own cleanups; the
    /// slot is freed with a new generation.
    pub(crate) fn dispose_index(&mut self, i: usize) {
        if !self.node(i).is_some_and(|n| n.alive) {
            return;
        }
        self.set_flags(i, flag::NONE);
        // Mark it gone first so an unlink that reaches it again stops here.
        if let Some(n) = self.node_mut(i) {
            n.alive = false;
        }
        self.dispose_deps_in_reverse(i);
        let owned = self
            .node_mut(i)
            .map(|n| std::mem::take(&mut n.owned))
            .unwrap_or_default();
        for id in owned.iter().rev() {
            if self.live(*id).is_some() {
                self.dispose_index(id.index);
            }
        }
        // Whoever read it (or owns it) forgets it.
        let mut link = self.node(i).map_or(NONE, |n| n.subs);
        while link != NONE {
            let next = self.l(link).next_sub;
            self.unlink(link);
            link = next;
        }
        let Some(n) = self.node_mut(i) else { return };
        let kind = n.kind;
        let cleanups = std::mem::take(&mut n.cleanups);
        let value = n.value.take();
        let update = n.update.take();
        let run = n.run.take();
        n.check = None;
        n.label = None;
        n.deps = NONE;
        n.deps_tail = NONE;
        n.subs = NONE;
        n.subs_tail = NONE;
        // A free slot holds no list: the one it had goes with the node. Kept
        // for whatever is made in the slot next (most often a signal, which
        // owns nothing), lists would spread over the free slots, a render
        // scope's table's worth of entries among them, and stay there.
        drop(owned);
        self.settle.cleanups.extend(cleanups);
        if let Some(v) = value {
            self.settle.garbage.push(Garbage::Value(v));
        }
        if let Some(u) = update {
            self.settle.garbage.push(Garbage::Update(u));
        }
        if let Some(r) = run {
            self.settle.garbage.push(Garbage::Run(r));
        }
        // Its handles are dead (`alive` is false); the slot's next node
        // takes a generation none had before. A slot whose generations ran
        // out is retired. A frame's own slots are never freed (its region
        // goes whole), nor a live one a running frame disposes (the log
        // puts it back): the live free slots are as without the frame.
        #[cfg(debug_assertions)]
        let freed = i & FORK_NODE == 0 && !self.forking();
        #[cfg(not(debug_assertions))]
        let freed = true;
        if freed && self.reusable(i) {
            self.free_nodes.push(i);
        }
        match kind {
            Kind::Signal => self.counts.signals = self.counts.signals.saturating_sub(1),
            Kind::Computed => self.counts.computeds = self.counts.computeds.saturating_sub(1),
            Kind::Effect => self.counts.effects = self.counts.effects.saturating_sub(1),
            Kind::Scope => self.counts.scopes = self.counts.scopes.saturating_sub(1),
            Kind::Render => self.counts.renders = self.counts.renders.saturating_sub(1),
        }
    }

    /// Link the dep `dep` to the running subscriber, if any.
    pub(crate) fn track(&mut self, dep: usize) {
        let sub = self.active_sub;
        if sub != NONE {
            self.link(dep, sub, self.cycle);
            if cfg!(debug_assertions) && self.phase != Phase::Idle {
                let (frame, bit) = (self.frame, self.phase.bit());
                if let Some(n) = self.node_mut(dep) {
                    if n.read_frame != frame {
                        n.read_frame = frame;
                        n.read_phases = 0;
                    }
                    n.read_phases |= bit;
                }
            }
        }
    }

    /// A render wrote signal `i`: a bug unless the render made it (a render
    /// is pure, as in Compose and Flutter; its writes belong in an effect,
    /// a layout hook or a handler). Allocates nothing unless it fails.
    #[cfg(debug_assertions)]
    fn check_render_write(&self, i: usize) {
        let Some((run, pure)) = self.rendering else {
            return;
        };
        let Some(n) = self.node(i) else { return };
        let own = n.born_in == run;
        match pure {
            Pure::Render(component) => assert!(
                own,
                "a render wrote live state, which a render must not: {} (made at {}) written by the render of \
                 `{component}`; write it in an effect, a layout hook or a handler",
                self.signal_ref(i),
                n.created_at,
            ),
            Pure::Getter => assert!(
                own,
                "a computed's getter wrote live state, which a getter must not: {} (made at {}); a getter \
                 only reads",
                self.signal_ref(i),
                n.created_at,
            ),
        }
    }

    /// Whether writing signal `i` now is late (ADR-0013's debug check): an
    /// earlier phase of this frame read it, and no pass of that phase
    /// follows in this frame.
    fn check_late(&mut self, i: usize) {
        let phase = self.phase;
        if phase <= Phase::Render {
            return;
        }
        let Some(n) = self.node(i) else { return };
        if n.read_frame != self.frame {
            return;
        }
        let mut earlier = n.read_phases & phase.earlier();
        if phase == Phase::Layout && self.settles {
            // Another render and layout pass follows (as Chrome's
            // ResizeObserver loop): what they read is read again.
            earlier &= !(Phase::Render.bit() | Phase::Layout.bit());
        } else if phase == Phase::Layout {
            earlier |= n.read_phases & Phase::Layout.bit();
        }
        if earlier == 0 {
            return;
        }
        let late = LateWrite {
            signal: self.signal_ref(i),
            created_at: n.created_at,
            read_in: Phase::of_bits(earlier),
            written_in: phase,
        };
        if !self.late.contains(&late) {
            eprintln!("gui: {late}");
            self.late.push(late);
        }
    }

    /// A signal's value changed: mark it and its subscribers; whether a
    /// flush is due now.
    pub(crate) fn wrote_signal(&mut self, i: usize) -> bool {
        if cfg!(debug_assertions) {
            self.check_late(i);
        }
        self.set_flags(i, flag::MUTABLE | flag::DIRTY);
        let subs = self.node(i).map_or(NONE, |n| n.subs);
        if subs != NONE {
            let inner = self.run_depth > 0;
            let outer = std::mem::replace(&mut self.writer, i);
            self.propagate(subs, inner);
            self.writer = outer;
            return self.batch_depth == 0;
        }
        false
    }

    /// Record a read for the trace.
    fn signal_ref(&self, i: usize) -> SignalRef {
        SignalRef {
            id: self.id_of(i).map_or(u64::MAX, NodeId::as_u64),
            label: self.slot(i).and_then(|n| n.label),
        }
    }

    pub(crate) fn record_read(&mut self, i: usize) {
        if self.trace.is_none() {
            return;
        }
        let me = self.signal_ref(i);
        if let Some(trace) = &mut self.trace
            && !trace.reads.contains(&me)
        {
            trace.reads.push(me);
        }
    }

    pub(crate) fn record_write(&mut self, i: usize) {
        if self.trace.is_none() {
            return;
        }
        let me = self.signal_ref(i);
        if let Some(trace) = &mut self.trace {
            trace.writes.push(me);
        }
    }

    /// The flush counter for effect `i`: false once it ran too often in this
    /// flush (a cycle), reported once.
    fn may_run(&mut self, i: usize) -> bool {
        let epoch = self.epoch;
        let Some(n) = self.node_mut(i) else { return false };
        if n.flush_epoch == epoch {
            n.flush_runs = n.flush_runs.saturating_add(1);
        } else {
            n.flush_epoch = epoch;
            n.flush_runs = 1;
        }
        if n.flush_runs == RECURSION_LIMIT.saturating_add(1) {
            eprintln!(
                "gui: an effect created at {} ran {RECURSION_LIMIT} times in one update; it writes what it reads \
                 through other effects. It is stopped for this update.",
                n.created_at
            );
        }
        n.flush_runs <= RECURSION_LIMIT
    }
}

// ── Running user code ────────────────────────────────────────────────────

/// Restores the running subscriber and owner, and ends a run of `node`, when
/// dropped: on return or while a panic unwinds.
struct RunGuard {
    node: usize,
    prev_sub: usize,
    prev_owner: Option<NodeId>,
    effect: bool,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        try_with(|rt| {
            rt.active_sub = self.prev_sub;
            rt.active_owner = self.prev_owner;
            if self.effect {
                rt.run_depth = rt.run_depth.saturating_sub(1);
            }
            let f = rt.flags(self.node);
            rt.set_flags(self.node, f & !flag::RECURSED_CHECK);
            if rt.node(self.node).is_some_and(|n| n.alive) {
                rt.purge_deps(self.node);
            }
        });
        // While a panic unwinds no user code runs (a second panic would
        // abort); what is queued runs at the next settle.
        if !std::thread::panicking() {
            settle();
        }
    }
}

/// Recompute computed `i`; whether its value changed.
pub(crate) fn update_computed(i: usize) -> bool {
    let Some((update, guard)) = with(|rt| {
        rt.before_rerun(i);
        let id = rt.id_of(i)?;
        let update = rt.node(i)?.update.clone()?;
        if let Some(n) = rt.node_mut(i) {
            n.deps_tail = NONE;
            n.flags = flag::MUTABLE | flag::RECURSED_CHECK;
        }
        let guard = RunGuard {
            node: i,
            prev_sub: std::mem::replace(&mut rt.active_sub, i),
            prev_owner: rt.active_owner.replace(id),
            effect: false,
        };
        rt.cycle = rt.cycle.wrapping_add(1);
        Some((update, guard))
    }) else {
        return false;
    };
    let _guard = guard;
    let _pure = Marked::enter(
        #[cfg(debug_assertions)]
        Some(Pure::Getter),
    );
    // Cleanups of the last run go before the new one starts.
    settle();
    update()
}

/// Run effect `i`'s body, tracking what it reads.
fn run_effect_body(i: usize) {
    let Some((run, guard)) = with(|rt| {
        let id = rt.id_of(i)?;
        let run = rt.node(i)?.run.clone()?;
        if let Some(n) = rt.node_mut(i) {
            n.deps_tail = NONE;
            n.flags = flag::WATCHING | flag::RECURSED_CHECK | (n.flags & flag::HAS_CHILD);
        }
        let guard = RunGuard {
            node: i,
            prev_sub: std::mem::replace(&mut rt.active_sub, i),
            prev_owner: rt.active_owner.replace(id),
            effect: true,
        };
        rt.cycle = rt.cycle.wrapping_add(1);
        rt.run_depth = rt.run_depth.saturating_add(1);
        if let Some(trace) = &mut rt.trace {
            trace.effect_runs = trace.effect_runs.saturating_add(1);
        }
        Some((run, guard))
    }) else {
        return;
    };
    let _guard = guard;
    // User code: its run goes through the gate in its window, in a turn
    // (`Effect::new`'s `Here::run`), so a turn goes on past its panic by
    // every path that runs it (ADR-0015, the turn rule).
    run();
}

/// What must write nothing live (`Some`), or nothing (`None`: the effects
/// a render queued, flushed), set while a guard lives. Nothing in a
/// release build.
struct Marked(#[cfg(debug_assertions)] Option<(u32, Pure)>);

impl Marked {
    fn enter(#[cfg(debug_assertions)] pure: Option<Pure>) -> Self {
        #[cfg(debug_assertions)]
        return Self(with(|rt| {
            let next = pure.map(|p| {
                rt.renders_run = rt.renders_run.wrapping_add(1).max(1);
                (rt.renders_run, p)
            });
            std::mem::replace(&mut rt.rendering, next)
        }));
        #[cfg(not(debug_assertions))]
        Self()
    }
}

#[cfg(debug_assertions)]
impl Drop for Marked {
    fn drop(&mut self) {
        let outer = self.0;
        try_with(|rt| rt.rendering = outer);
    }
}

/// Before signal `i` is written: a render or getter writing what it did not
/// make panics (a debug build), and leaves the signal as it was.
pub(crate) fn check_write(i: usize) {
    #[cfg(debug_assertions)]
    with(|rt| rt.check_render_write(i));
    #[cfg(not(debug_assertions))]
    let _ = i;
}

/// The first run of new effect `id`: now, or once the render or setup making
/// it returns (it is not the render's; Vue's pre-flush, Solid's
/// `createEffect`).
pub(crate) fn first_run_or_defer(id: NodeId) {
    // An effect made anywhere in a cold frame (a render, a binding, paint)
    // never runs: the frame is thrown away. The engine's own scheduling,
    // not a read of a Shared init: the effect's run reads what it reads,
    // each read checked ([`crate::engine::no_forked_read`]).
    #[cfg(debug_assertions)]
    if crate::engine::is_cold() {
        return;
    }
    let deferred = with(|rt| {
        let defer = rt.in_render > 0;
        if defer {
            rt.deferred.push(id);
        }
        defer
    });
    if !deferred {
        first_run(id.index);
    }
}

/// The first run of a new effect.
pub(crate) fn first_run(i: usize) {
    run_effect_body(i);
}

/// `run(e)` of alien-signals: run the effect when one of its deps changed.
fn run(i: usize) {
    let flags = with(|rt| rt.flags(i));
    let dirty = flags & flag::DIRTY != 0 || (flags & flag::PENDING != 0 && check_dirty_of(i));
    let render = with(|rt| rt.node(i).is_some_and(|n| n.kind == Kind::Render));
    if dirty && render {
        // Marked, not run: it stays dirty (and is not notified again) until
        // the frame renders it through `track_render`.
        let mark = with(|rt| {
            rt.set_flags(i, flag::DIRTY);
            let by = rt.node(i).map_or(NONE, |n| n.woken_by);
            rt.waking = (by != NONE).then(|| rt.signal_ref(by));
            rt.node(i).and_then(|n| n.run.clone())
        });
        if let Some(mark) = mark {
            untracked_call(|| mark());
        }
        with(|rt| rt.waking = None);
        return;
    }
    if dirty {
        let go = with(|rt| {
            if !rt.may_run(i) {
                let f = rt.flags(i);
                rt.set_flags(i, (f & !(flag::DIRTY | flag::PENDING)) | flag::WATCHING);
                return false;
            }
            rt.before_rerun(i);
            true
        });
        settle();
        // A cleanup may have disposed the effect.
        if go && with(|rt| rt.node(i).is_some_and(|n| n.alive && n.flags != flag::NONE)) {
            run_effect_body(i);
        }
    } else {
        with(|rt| {
            if rt.node(i).is_some_and(|n| n.deps != NONE) {
                let f = rt.flags(i);
                rt.set_flags(i, flag::WATCHING | (f & flag::HAS_CHILD));
            }
        });
    }
}

/// Whether a dep of pending node `i` changed, recomputing computeds on the way.
pub(crate) fn check_dirty_of(i: usize) -> bool {
    let deps = with(|rt| rt.node(i).map_or(NONE, |n| n.deps));
    if deps == NONE {
        return false;
    }
    check_dirty(deps, i)
}

/// `checkDirty` of alien-signals, borrowing the runtime per step so a
/// computed's getter can run between steps.
fn check_dirty(start: usize, start_sub: usize) -> bool {
    let base = with(|rt| rt.stack.len());
    let mut link = start;
    let mut sub = start_sub;
    let mut depth: u32 = 0;
    let mut dirty = false;
    'top: loop {
        enum Step {
            Dirty,
            Update(usize),
            Descend(usize),
            Next,
        }
        let step = with(|rt| {
            let dep = rt.l(link).dep;
            let flags = rt.flags(dep);
            if rt.flags(sub) & flag::DIRTY != 0 {
                Step::Dirty
            } else if flags & (flag::MUTABLE | flag::DIRTY) == flag::MUTABLE | flag::DIRTY {
                Step::Update(dep)
            } else if flags & (flag::MUTABLE | flag::PENDING) == flag::MUTABLE | flag::PENDING {
                rt.stack.push(link);
                Step::Descend(dep)
            } else {
                Step::Next
            }
        });
        match step {
            Step::Dirty => dirty = true,
            Step::Update(dep) => {
                if update(dep) {
                    with(|rt| {
                        let subs = rt.node(dep).map_or(NONE, |n| n.subs);
                        if subs != NONE && rt.l(subs).next_sub != NONE {
                            rt.shallow_propagate(subs);
                        }
                    });
                    dirty = true;
                }
            }
            Step::Descend(dep) => {
                link = with(|rt| rt.node(dep).map_or(NONE, |n| n.deps));
                sub = dep;
                depth = depth.saturating_add(1);
                if link == NONE {
                    // A pending computed with no deps cannot have changed.
                    link = with(|rt| rt.stack.pop().unwrap_or(NONE));
                    sub = with(|rt| rt.l(link).sub);
                    depth = depth.saturating_sub(1);
                } else {
                    continue;
                }
            }
            Step::Next => {}
        }

        if !dirty {
            let next_dep = with(|rt| rt.l(link).next_dep);
            if next_dep != NONE {
                link = next_dep;
                continue;
            }
        }

        while depth > 0 {
            depth = depth.saturating_sub(1);
            link = with(|rt| rt.stack.pop().unwrap_or(NONE));
            if dirty {
                if update(sub) {
                    with(|rt| {
                        let subs = rt.node(sub).map_or(NONE, |n| n.subs);
                        if subs != NONE && rt.l(subs).next_sub != NONE {
                            rt.shallow_propagate(subs);
                        }
                    });
                    sub = with(|rt| rt.l(link).sub);
                    continue;
                }
                dirty = false;
            } else {
                with(|rt| {
                    let f = rt.flags(sub);
                    rt.set_flags(sub, f & !flag::PENDING);
                });
            }
            sub = with(|rt| rt.l(link).sub);
            let next_dep = with(|rt| rt.l(link).next_dep);
            if next_dep != NONE {
                link = next_dep;
                continue 'top;
            }
        }

        with(|rt| rt.stack.truncate(base));
        return dirty && with(|rt| rt.flags(sub)) != flag::NONE;
    }
}

/// `update` of alien-signals: bring dep `i` up to date; whether it changed.
fn update(i: usize) -> bool {
    let kind = with(|rt| rt.node(i).map(|n| n.kind));
    match kind {
        Some(Kind::Computed) => update_computed(i),
        Some(Kind::Signal) => update_signal(i),
        Some(Kind::Scope | Kind::Effect | Kind::Render) | None => {
            with(|rt| rt.set_flags(i, flag::MUTABLE));
            true
        }
    }
}

/// `updateSignal` of alien-signals: signal `i` was written since it was
/// last checked; whether its value now differs from the one before (a value
/// written and written back within a batch did not change).
fn update_signal(i: usize) -> bool {
    let found = with(|rt| {
        let n = rt.node(i)?;
        let value = n.value.clone()?;
        // A cold frame's copy of a live signal: its check would write the
        // live value's record of what it was; changed, as far as it knows.
        #[cfg(debug_assertions)]
        let check = if rt.inherited(i) { None } else { n.check };
        #[cfg(not(debug_assertions))]
        let check = n.check;
        rt.set_flags(i, flag::MUTABLE);
        Some((value, check))
    });
    match found {
        Some((value, Some(check))) => check(&*value),
        Some((_, None)) => true,
        None => false,
    }
}

/// Before signal `i` is read: if it was written since it was last checked,
/// check it, and if it changed, its pending readers are dirty.
pub(crate) fn settle_signal(i: usize) {
    if with(|rt| rt.flags(i) & flag::DIRTY == 0) {
        return;
    }
    if update_signal(i) {
        with(|rt| {
            let subs = rt.node(i).map_or(NONE, |n| n.subs);
            if subs != NONE {
                rt.shallow_propagate(subs);
            }
        });
    }
}

/// Whether signal `i` was written since it was last checked.
pub(crate) fn is_dirty(rt: &Runtime, i: usize) -> bool {
    rt.flags(i) & flag::DIRTY != 0
}

/// Bring computed `i` up to date before it is read (`computedOper`).
pub(crate) fn refresh_computed(i: usize) {
    #[cfg(debug_assertions)]
    with(|rt| rt.distrust(i));
    let flags = with(|rt| rt.flags(i));
    if flags & flag::DIRTY != 0
        || (flags & flag::PENDING != 0 && {
            let dirty = check_dirty_of(i);
            if !dirty {
                with(|rt| {
                    let f = rt.flags(i);
                    rt.set_flags(i, f & !flag::PENDING);
                });
            }
            dirty
        })
    {
        if update_computed(i) {
            with(|rt| {
                let subs = rt.node(i).map_or(NONE, |n| n.subs);
                if subs != NONE {
                    rt.shallow_propagate(subs);
                }
            });
        }
    } else if flags == flag::NONE {
        // First read: compute without comparing.
        update_computed(i);
    }
}

/// Run the queued effects, unless a flush is running already (it will reach
/// them): no flush nests in another.
pub(crate) fn flush() {
    // The effects run in a turn (ADR-0015, no bare mode): the one under
    // way, or, a write made with none (a test's top level), one of the
    // current window's, so their panics have one arbiter.
    if !crate::engine::dispatching()
        && !crate::engine::forking()
        && try_with(|rt| rt.queued.len() > rt.notify_index && !rt.flushing).unwrap_or(false)
    {
        crate::engine::in_turn(|_| flush());
        return;
    }
    let start = with(|rt| {
        // A cold frame runs no effect: what it queued is dropped.
        #[cfg(debug_assertions)]
        if rt.cold.is_some() {
            rt.queued.clear();
            rt.notify_index = 0;
            return false;
        }
        if rt.flushing {
            return false;
        }
        rt.flushing = true;
        rt.epoch = rt.epoch.wrapping_add(1);
        true
    });
    if !start {
        return;
    }
    let guard = FlushGuard;
    // A panicking effect does not stop the others: the flush goes on and the
    // first panic is raised again at its end (Vue's `callWithErrorHandling`).
    // Inside a dispatch, each goes to the dispatch's one arbiter as it is
    // caught: a later effect may dispatch again (input to another window)
    // and panic there, and the first in time must stay first.
    let mut all = crate::engine::Gather::new();
    loop {
        let next = with(|rt| {
            let e = rt.queued.get(rt.notify_index).copied()?;
            rt.notify_index = rt.notify_index.saturating_add(1);
            Some(e)
        });
        let Some(e) = next else { break };
        // An engine commit step publishes all its writes at once: an effect
        // run inside one would read it half done (`engine::commit`).
        debug_assert!(
            !crate::engine::in_commit(),
            "an effect ran inside an engine commit step: its writes are not one batch"
        );
        all.run(|| run(e));
    }
    drop(guard);
    all.finish();
}

/// Run what is queued when no batch is open: the effects a batch left as a
/// panic unwound it ([`BatchGuard`]), run by the dispatch that caught the
/// panic ([`crate::engine::call_out`]) before it goes on.
pub(crate) fn flush_unbatched() {
    if try_with(|rt| rt.batch_depth == 0 && rt.queued.len() > rt.notify_index).unwrap_or(false) {
        flush();
    }
}

/// Ends a flush: on return or while a panic unwinds, effects still queued
/// watch again (they run on the next write) and the queue empties.
struct FlushGuard;

impl Drop for FlushGuard {
    fn drop(&mut self) {
        try_with(|rt| {
            while let Some(&e) = rt.queued.get(rt.notify_index) {
                rt.notify_index = rt.notify_index.saturating_add(1);
                let f = rt.flags(e);
                rt.set_flags(e, f | flag::WATCHING | flag::RECURSED);
            }
            rt.queued.clear();
            rt.notify_index = 0;
            rt.flushing = false;
        });
    }
}

pub(crate) fn start_batch() {
    with(|rt| rt.batch_depth = rt.batch_depth.saturating_add(1));
}

pub(crate) fn end_batch() {
    let flush_now = with(|rt| {
        rt.batch_depth = rt.batch_depth.saturating_sub(1);
        rt.batch_depth == 0
    });
    if flush_now {
        flush();
    }
}

/// Ends a batch when dropped, on return or unwind.
pub(crate) struct BatchGuard;

impl BatchGuard {
    pub(crate) fn new() -> Self {
        start_batch();
        Self
    }
}

impl Drop for BatchGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            // The effects queued stay queued, and run at the next flush.
            try_with(|rt| rt.batch_depth = rt.batch_depth.saturating_sub(1));
        } else if try_with(|_| ()).is_some() {
            end_batch();
        }
    }
}

/// `trigger` of alien-signals: run `f` reading through a temporary
/// subscriber, then wake what depends on each thing it read, as if each had
/// changed.
pub(crate) fn trigger(f: impl FnOnce(), created_at: &'static Location<'static>) {
    let temp = with(|rt| rt.alloc(Kind::Scope, flag::WATCHING | flag::RECURSED_CHECK, created_at));
    // One batch around it all: a write inside `f` and the wake after it run
    // what depends on them once.
    let _batch = BatchGuard::new();
    {
        let prev = with(|rt| std::mem::replace(&mut rt.active_sub, temp.index));
        let _restore = RestoreSub(prev);
        f();
    }
    with(|rt| {
        rt.set_flags(temp.index, flag::NONE);
        let mut link = rt.node(temp.index).map_or(NONE, |n| n.deps);
        while link != NONE {
            let dep = rt.l(link).dep;
            link = rt.unlink(link);
            let subs = rt.node(dep).map_or(NONE, |n| n.subs);
            if subs != NONE {
                let inner = rt.run_depth > 0;
                rt.propagate(subs, inner);
                rt.shallow_propagate(subs);
            }
        }
        rt.dispose(temp);
    });
    settle();
}

struct RestoreSub(usize);

impl Drop for RestoreSub {
    fn drop(&mut self) {
        let prev = self.0;
        try_with(|rt| rt.active_sub = prev);
    }
}

/// Run `f` as render node `i`'s tracked run: what it reads is what the
/// render depends on from now on. The owner is left as it is (a render
/// creates no state of its own; ADR-0013).
pub(crate) fn track_render<R>(i: usize, f: impl FnOnce() -> R) -> R {
    let guard = with(|rt| {
        if let Some(n) = rt.node_mut(i) {
            n.deps_tail = NONE;
            n.flags = flag::WATCHING | flag::RECURSED_CHECK;
        }
        rt.cycle = rt.cycle.wrapping_add(1);
        rt.run_depth = rt.run_depth.saturating_add(1);
        RunGuard {
            node: i,
            prev_sub: std::mem::replace(&mut rt.active_sub, i),
            prev_owner: rt.active_owner,
            effect: true,
        }
    });
    let _guard = guard;
    f()
}

/// Whether node `i` read anything in its last run.
pub(crate) fn has_deps(i: usize) -> bool {
    with(|rt| rt.node(i).is_some_and(|n| n.deps != NONE))
}

/// Move what node `from` read in its last run to node `to` (which has read
/// nothing): `to` is woken by those from now on, `from` by none of them. A
/// tracker made only once its first run is known to read something
/// (ADR-0013's per-node phase subscribers).
pub(crate) fn transfer_deps(from: usize, to: usize) {
    with(|rt| {
        rt.cycle = rt.cycle.wrapping_add(1);
        let cycle = rt.cycle;
        let mut link = rt.node(from).map_or(NONE, |n| n.deps);
        while link != NONE {
            let dep = rt.l(link).dep;
            rt.link(dep, to, cycle);
            link = rt.l(link).next_dep;
        }
        if let Some(n) = rt.node_mut(from) {
            n.deps_tail = NONE;
        }
        rt.purge_deps(from);
    });
    settle();
}

/// Start frame `frame`'s late-write record: reads from here on are this frame's.
pub(crate) fn begin_frame() {
    with(|rt| rt.frame = rt.frame.wrapping_add(1).max(1));
}

/// Run `f` as phase `phase` of the running frame; `settles` for a layout
/// pass another pass follows when it dirties what was read.
pub(crate) fn in_phase<R>(phase: Phase, settles: bool, f: impl FnOnce() -> R) -> R {
    struct Restore(Phase, bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let (phase, settles) = (self.0, self.1);
            try_with(|rt| {
                rt.phase = phase;
                rt.settles = settles;
            });
        }
    }
    let _restore = with(|rt| {
        let r = Restore(rt.phase, rt.settles);
        rt.phase = phase;
        rt.settles = settles;
        r
    });
    f()
}

/// Run `f`, the render (or setup) of component `name`: a live signal it
/// writes, one it did not make, panics in a debug build. The effects it
/// makes run once it returns, before its children resolve and before
/// layout; in a cold re-render they never run (they go with its scope).
#[cfg_attr(not(debug_assertions), expect(unused_variables, reason = "the check is debug only"))]
pub(crate) fn rendering<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    struct Leave(usize);
    impl Drop for Leave {
        fn drop(&mut self) {
            let mark = self.0;
            try_with(|rt| {
                rt.in_render = rt.in_render.saturating_sub(1);
                if std::thread::panicking() {
                    rt.deferred.truncate(mark);
                }
            });
        }
    }
    let leave = with(|rt| {
        rt.in_render = rt.in_render.saturating_add(1);
        Leave(rt.deferred.len())
    });
    // Marked for the render only (it has a `Drop` in debug builds only).
    let out = {
        let _marked = Marked::enter(
            #[cfg(debug_assertions)]
            Some(Pure::Render(name)),
        );
        f()
    };
    let mark = leave.0;
    drop(leave);
    flush_deferred(mark);
    out
}

/// Run the effects queued from `mark` on, as effects (no render's), unless
/// a cold re-render made them.
fn flush_deferred(mark: usize) {
    let (queued, cold) = with(|rt| {
        let at = mark.min(rt.deferred.len());
        (rt.deferred.split_off(at), is_cold_rt(rt))
    });
    if cold || queued.is_empty() {
        return;
    }
    struct Outer(u32);
    impl Drop for Outer {
        fn drop(&mut self) {
            let depth = self.0;
            try_with(|rt| rt.in_render = depth);
        }
    }
    let _outer = with(|rt| Outer(std::mem::take(&mut rt.in_render)));
    let _effects = Marked::enter(
        #[cfg(debug_assertions)]
        None,
    );
    let settles = with(|rt| rt.settles);
    // Every one runs, though one panics: the first panic goes on once all
    // have (a panic leaves nothing queued behind it).
    let mut all = crate::engine::Gather::new();
    in_phase(Phase::Effect, settles, || {
        for id in queued {
            if with(|rt| rt.live(id).is_some()) {
                all.run(|| first_run(id.index));
            }
        }
    });
    all.finish();
}

/// Whether `rt` is a cold frame's.
#[cfg(debug_assertions)]
fn is_cold_rt(rt: &Runtime) -> bool {
    rt.cold.is_some()
}

#[cfg(not(debug_assertions))]
fn is_cold_rt(_: &Runtime) -> bool {
    false
}

/// Whether a cold frame runs ([`crate::engine::ColdFrame`]). Checked where
/// it is read (ADR-0028, R4, F1 review 11): a Shared init may not ask, as
/// its answer differs in a fork and live. The engine's own scheduling and
/// allocation read `crate::engine::is_cold`, unchecked.
#[cfg(debug_assertions)]
#[must_use]
pub(crate) fn is_cold() -> bool {
    crate::engine::no_forked_read("whether a fork runs");
    crate::engine::is_cold()
}

/// Never in a release build. This and the other release halves of the
/// cold check are `const fn`, which cannot reach a thread local: the
/// compiler proves the release read and write paths look nothing up for
/// the check, and the assertion below pins it.
#[cfg(not(debug_assertions))]
#[must_use]
pub(crate) const fn is_cold() -> bool {
    false
}

#[cfg(not(debug_assertions))]
const _: () = {
    let id = NodeId::new(0, 0);
    assert!(!is_cold() && !cold_refuses(id) && !cold_owns(id));
};

/// One step a cold frame took on the arena, undone at its end, last first.
#[cfg(debug_assertions)]
enum Undo {
    /// A slot that was there before the frame, as it was then.
    Node(usize, Box<Node>),
    /// A live link, as it was then.
    Link(usize, Link),
}

/// What a cold frame did to the arena, to undo (decision B): each slot
/// that was there before it, kept as it was on its first write, and the
/// free lists' steps; the rest of the runtime set aside whole. A check
/// costs what the frame writes, not what the thread holds.
#[cfg(debug_assertions)]
pub(crate) struct ColdLog {
    /// The live signals the frame tried to write (never: a render, a
    /// getter or a binding is pure).
    wrote: Vec<String>,
    undo: Vec<Undo>,
    /// Each live node the frame wrote, and where its record is in `undo`.
    logged_nodes: std::collections::HashMap<usize, usize>,
    /// Each live link the frame wrote, and where its record is in `undo`.
    logged_links: std::collections::HashMap<usize, usize>,
    /// Live nodes whose value the frame replaced with its own.
    owned: std::collections::HashSet<usize>,
    /// Live computeds the frame computed again (it trusts no live cache).
    distrusted: std::collections::HashSet<usize>,
    /// The live queues and counters while the frame runs.
    aside: Aside,
}

#[cfg(debug_assertions)]
impl ColdLog {
    /// Slot `i` holds a live node the running frame forks (its own region
    /// is never logged).
    fn live(i: usize) -> bool {
        i & FORK_NODE == 0
    }
}

/// The runtime's queues and counters, set aside while a cold frame runs.
#[cfg(debug_assertions)]
struct Aside {
    queued: Vec<usize>,
    notify_index: usize,
    flushing: bool,
    epoch: u32,
    cycle: u32,
    run_depth: u32,
    batch_depth: u32,
    active_sub: usize,
    active_owner: Option<NodeId>,
    stack: Vec<usize>,
    unwatched_stack: Vec<usize>,
    unwatching: bool,
    settle: Settle,
    counts: Counts,
    trace: Option<ReactiveTrace>,
    writer: usize,
    waking: Option<SignalRef>,
    frame: u32,
    phase: Phase,
    settles: bool,
    late: Vec<LateWrite>,
    rendering: Option<(u32, Pure)>,
    renders_run: u32,
    in_render: u32,
    deferred: Vec<NodeId>,
}

impl Runtime {
    #[cfg(debug_assertions)]
    #[inline]
    fn log_node(&mut self, i: usize) {
        let Some(log) = self.cold.as_deref_mut() else { return };
        if i & FORK_NODE != 0 {
            return;
        }
        if !log.logged_nodes.contains_key(&i)
            && let Some(n) = self.nodes.get_mut(i)
        {
            let copy = n.cold_copy();
            // The copy, and each node of the owned list it copies.
            crate::engine::fork_work(u64::try_from(copy.owned.len()).unwrap_or(u64::MAX).saturating_add(1));
            log.logged_nodes.insert(i, log.undo.len());
            log.undo.push(Undo::Node(i, Box::new(std::mem::replace(n, copy))));
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn log_node(&mut self, _: usize) {}

    #[cfg(debug_assertions)]
    #[inline]
    fn log_link(&mut self, i: usize) {
        if let Some(log) = self.cold.as_deref_mut()
            && i & FORK_LINK == 0
            && !log.logged_links.contains_key(&i)
            && let Some(&l) = self.links.get(i)
        {
            log.logged_links.insert(i, log.undo.len());
            log.undo.push(Undo::Link(i, l));
            crate::engine::fork_work(1);
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn log_link(&mut self, _: usize) {}

    /// In a cold frame, node `i` is a live one whose value the frame shares
    /// and has not made its own.
    #[cfg(debug_assertions)]
    fn inherited(&self, i: usize) -> bool {
        self.cold
            .as_deref()
            .is_some_and(|log| ColdLog::live(i) && !log.owned.contains(&i))
    }

    /// In a cold frame, live computed `i` read the first time: marked dirty,
    /// so the frame computes it again rather than trust the live cache.
    #[cfg(debug_assertions)]
    fn distrust(&mut self, i: usize) {
        let first = self
            .cold
            .as_deref_mut()
            .is_some_and(|log| ColdLog::live(i) && log.distrusted.insert(i));
        if first && self.node(i).is_some_and(|n| n.alive && n.kind == Kind::Computed) {
            let f = self.flags(i);
            self.set_flags(i, f | flag::DIRTY);
        }
    }

    /// What the cold frame made, taken out while it still runs: the
    /// cleanups, values and closures of what it wrote or made, and its
    /// queues; dropped still cold, one box per element (ADR-0034, S9), so a
    /// drop that panics is a fault and the rest are still dropped, each on
    /// its own, never in its unwind (S2). A live node it only read keeps its fields: they are handles
    /// to the live node's own (`Node::cold_copy`), and what reads them while
    /// the frame disposes finds them.
    #[cfg(debug_assertions)]
    fn drain_cold(&mut self) -> Vec<Box<dyn Any>> {
        fn each<T: 'static>(made: &mut Vec<Box<dyn Any>>, items: impl IntoIterator<Item = T>) {
            made.extend(items.into_iter().map(|t| -> Box<dyn Any> { Box::new(t) }));
        }
        let mut made: Vec<Box<dyn Any>> = Vec::new();
        let Some(log) = self.cold.as_deref() else { return made };
        let own: Vec<(usize, bool)> = log
            .logged_nodes
            .keys()
            .map(|&i| (i, log.owned.contains(&i)))
            .chain((0..self.fork_nodes.len()).map(|k| (k | FORK_NODE, true)))
            .collect();
        for (i, whole) in own {
            let Some(n) = self.slot_mut(i) else { continue };
            each(&mut made, std::mem::take(&mut n.cleanups));
            if whole {
                each(&mut made, n.value.take());
                each(&mut made, n.update.take());
                each(&mut made, n.run.take());
                each(&mut made, n.check.take());
            }
        }
        let Settle { cleanups, garbage } = std::mem::take(&mut self.settle);
        each(&mut made, cleanups);
        each(&mut made, garbage);
        made.push(Box::new(std::mem::take(&mut self.late)));
        made.push(Box::new(std::mem::take(&mut self.deferred)));
        made
    }

    /// Put back what the cold frame changed, by moves (ADR-0034, S1). What
    /// is replaced is the frame's shells, which the drain emptied of every
    /// value and closure of its own: a node it only read holds handles to
    /// the live node's (`Node::cold_copy`), whose drop runs no code while
    /// the live node, just put back, holds them too. A shell holding more
    /// is quarantined ([`Node::is_shell`]).
    #[cfg(debug_assertions)]
    fn rollback(&mut self) {
        fn put_by(made: Vec<Box<dyn Any>>) {
            if !made.is_empty() {
                crate::engine::quarantined("RUNTIME", Box::new(made));
            }
        }
        let mut made: Vec<Box<dyn Any>> = Vec::new();
        let Some(log) = self.cold.take() else { return };
        let ColdLog {
            wrote,
            undo,
            aside,
            owned,
            ..
        } = *log;
        drop(wrote);
        for step in undo.into_iter().rev() {
            match step {
                Undo::Node(i, old) => {
                    if let Some(n) = self.nodes.get_mut(i) {
                        let shell = std::mem::replace(n, *old);
                        if !shell.is_shell(owned.contains(&i)) {
                            made.push(Box::new(shell));
                        }
                    }
                }
                Undo::Link(i, old) => {
                    if let Some(l) = self.links.get_mut(i) {
                        *l = old;
                    }
                }
            }
        }
        // The frame's own regions go; the live ones were only written
        // where the log puts them back, their free slots never touched.
        for shell in self.fork_nodes.drain(..) {
            if !shell.is_shell(true) {
                made.push(Box::new(shell));
            }
        }
        self.fork_links.clear();
        self.fork_free_links.clear();
        let Aside {
            queued,
            notify_index,
            flushing,
            epoch,
            cycle,
            run_depth,
            batch_depth,
            active_sub,
            active_owner,
            stack,
            unwatched_stack,
            unwatching,
            settle,
            counts,
            trace,
            writer,
            waking,
            frame,
            phase,
            settles,
            late,
            rendering,
            renders_run,
            in_render,
            deferred,
        } = aside;
        // The frame's queue of cleanups and garbage was drained; the rest
        // are ids and records, data.
        let left = std::mem::replace(&mut self.settle, settle);
        if !left.cleanups.is_empty() || !left.garbage.is_empty() {
            made.push(Box::new(left));
        }
        self.queued = queued;
        self.stack = stack;
        self.unwatched_stack = unwatched_stack;
        self.late = late;
        self.deferred = deferred;
        self.trace = trace;
        self.notify_index = notify_index;
        self.flushing = flushing;
        self.epoch = epoch;
        self.cycle = cycle;
        self.run_depth = run_depth;
        self.batch_depth = batch_depth;
        self.active_sub = active_sub;
        self.active_owner = active_owner;
        self.unwatching = unwatching;
        self.counts = counts;
        self.writer = writer;
        self.waking = waking;
        self.frame = frame;
        self.phase = phase;
        self.settles = settles;
        self.rendering = rendering;
        self.renders_run = renders_run;
        self.in_render = in_render;
        put_by(made);
    }
}

/// The arena's cold policy ([`RUNTIME`], Fork by undo log): the log starts.
#[cfg(debug_assertions)]
fn cold_enter() -> Box<dyn Any> {
    with(|rt| {
        assert!(rt.cold.is_none(), "a cold frame inside a cold frame");
        let aside = Aside {
            queued: std::mem::take(&mut rt.queued),
            notify_index: std::mem::take(&mut rt.notify_index),
            flushing: std::mem::take(&mut rt.flushing),
            epoch: rt.epoch,
            cycle: rt.cycle,
            run_depth: rt.run_depth,
            batch_depth: rt.batch_depth,
            active_sub: rt.active_sub,
            active_owner: rt.active_owner,
            stack: std::mem::take(&mut rt.stack),
            unwatched_stack: std::mem::take(&mut rt.unwatched_stack),
            unwatching: std::mem::take(&mut rt.unwatching),
            settle: std::mem::take(&mut rt.settle),
            counts: rt.counts,
            trace: rt.trace.take(),
            writer: std::mem::replace(&mut rt.writer, NONE),
            waking: rt.waking.take(),
            frame: rt.frame,
            phase: rt.phase,
            settles: rt.settles,
            late: std::mem::take(&mut rt.late),
            rendering: rt.rendering,
            renders_run: rt.renders_run,
            in_render: rt.in_render,
            deferred: std::mem::take(&mut rt.deferred),
        };
        rt.cold = Some(Box::new(ColdLog {
            wrote: Vec::new(),
            undo: Vec::new(),
            logged_nodes: std::collections::HashMap::new(),
            logged_links: std::collections::HashMap::new(),
            owned: std::collections::HashSet::new(),
            distrusted: std::collections::HashSet::new(),
            aside,
        }));
    });
    Box::new(())
}

/// What the cold frame made, taken out of the arena while it still runs:
/// the values and closures of the nodes it made or made its own, the
/// cleanups it queued, its queues.
#[cfg(debug_assertions)]
fn cold_drain(_: &dyn Any) -> Vec<Box<dyn Any>> {
    try_with(Runtime::drain_cold).unwrap_or_default()
}

/// The arena's slots, and how many hold a live node (the fork tests).
#[cfg(all(debug_assertions, test))]
pub(crate) fn arena() -> (usize, usize) {
    with(|rt| (rt.nodes.len(), rt.nodes.iter().filter(|n| n.alive).count()))
}

/// Run `f`, a Shared piece's init, as a step of its own: what it allocates
/// is kept once it returns, and is then no longer the record of an init
/// it ran within (a piece made is made, whatever becomes of the init that
/// first asked for it); if it panics, what it allocated itself is undone
/// ([`Runtime::unmake`]), so a failed init retried leaves nothing behind.
pub(crate) fn making_shared<R>(f: impl FnOnce() -> R) -> R {
    /// Ends the step however `f` ends.
    struct Step {
        mark: usize,
        returned: bool,
    }
    impl Drop for Step {
        fn drop(&mut self) {
            let (mark, returned) = (self.mark, self.returned);
            let undone = try_with(|rt| {
                rt.making = rt.making.saturating_sub(1);
                // Its own record, above the mark: kept, or undone.
                if returned {
                    rt.made.truncate(mark);
                    Vec::new()
                } else {
                    rt.made.split_off(mark.min(rt.made.len()))
                }
            })
            .unwrap_or_default();
            if undone.is_empty() {
                return;
            }
            // Last made first, as a scope disposes what it owns: the
            // arena's own bookkeeping ([`crate::engine::allocating`]).
            crate::engine::allocating(|| try_with(|rt| undone.into_iter().rev().for_each(|id| rt.unmake(id))));
            // While unwinding, what disposing runs waits for the next settle.
            if !std::thread::panicking() {
                settle();
            }
        }
    }
    let mark = with(|rt| {
        rt.making = rt.making.saturating_add(1);
        rt.made.len()
    });
    let mut step = Step { mark, returned: false };
    let out = f();
    step.returned = true;
    out
}

/// The arena put back as it was, by moves (ADR-0034, S1).
#[cfg(debug_assertions)]
fn cold_exit(_: Box<dyn Any>) {
    RUNTIME.switch_with(|rt| {
        if let Ok(mut rt) = rt.try_borrow_mut() {
            rt.rollback();
        }
    });
}

/// How many steps the running cold frame's undo log holds (its cost).
#[cfg(all(debug_assertions, test))]
pub(crate) fn cold_steps() -> usize {
    try_with(|rt| rt.cold.as_ref().map_or(0, |l| l.undo.len())).unwrap_or(0)
}

/// The live signals the running cold frame tried to write.
#[cfg(debug_assertions)]
pub(crate) fn cold_wrote() -> Vec<String> {
    try_with(|rt| rt.cold.as_ref().map(|l| l.wrote.clone()).unwrap_or_default()).unwrap_or_default()
}

/// Before live state is written in place: in a cold frame, a value it
/// shares with the live runtime is not, and the write is recorded (a
/// render, a getter or a binding never writes live state). Whether the
/// write is refused. Compiled away in a release build.
#[cfg(debug_assertions)]
#[inline]
pub(crate) fn cold_refuses(id: NodeId) -> bool {
    with(|rt| {
        if rt.live(id).is_none() || !rt.inherited(id.index) {
            return false;
        }
        let made_at = rt.live(id).map(|n| n.created_at);
        let what = format!(
            "{} (made at {})",
            rt.signal_ref(id.index),
            made_at.map_or_else(String::new, ToString::to_string)
        );
        if let Some(log) = rt.cold.as_deref_mut() {
            log.wrote.push(what);
        }
        true
    })
}

#[cfg(not(debug_assertions))]
#[inline]
pub(crate) const fn cold_refuses(_: NodeId) -> bool {
    false
}

/// In a cold frame, node `id`'s value made the frame's own: whether it was
/// the live one's until now (the caller then gives it `value`, a new one,
/// copied on write). Outside one, `false`.
#[cfg(debug_assertions)]
pub(crate) fn cold_owns(id: NodeId) -> bool {
    with(|rt| {
        if rt.live(id).is_none() || !rt.inherited(id.index) {
            return false;
        }
        rt.log_node(id.index);
        if let Some(log) = rt.cold.as_deref_mut() {
            log.owned.insert(id.index);
        }
        true
    })
}

#[cfg(not(debug_assertions))]
#[inline]
pub(crate) const fn cold_owns(_: NodeId) -> bool {
    false
}

/// Give node `id` `value`, the one it now holds (copied on write in a cold
/// frame, [`cold_owns`]).
pub(crate) fn replace_value(id: NodeId, value: Rc<dyn Any>) {
    let old = with(|rt| rt.live_mut(id).and_then(|n| n.value.replace(value)));
    drop(old);
}

/// The late writes reported since the last call.
pub(crate) fn take_late_writes() -> Vec<LateWrite> {
    with(|rt| std::mem::take(&mut rt.late))
}

/// What node `i` read in its last run (signals and computeds), for the devtools.
pub(crate) fn deps_of(i: usize) -> Vec<SignalRef> {
    with(|rt| {
        let mut out = Vec::new();
        let mut link = rt.node(i).map_or(NONE, |n| n.deps);
        while link != NONE {
            let dep = rt.l(link).dep;
            out.push(rt.signal_ref(dep));
            link = rt.l(link).next_dep;
        }
        out
    })
}
