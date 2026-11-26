//! Fine-grained reactive state (ADR-0012): signals, computeds, effects and
//! the scopes that own them, on the alien-signals algorithm as Vue 3.6
//! adopted it.
//!
//! Handles are `Copy` and name a node in a thread-local arena. A write marks
//! what depends on it at once (push); a computed recomputes only when read
//! and only if something it read changed (pull); an equal value stops the
//! update at a signal and at a computed. Effects run after the write that
//! woke them, an owner before what it owns, once per update however many of
//! their sources changed, and never inside one another's flush.

mod collection;
mod computed;
mod effect;
mod runtime;
/// A node that owns what is made while it runs (a scope, effect or computed).
pub(crate) use runtime::OwnerId;
mod scope;
mod signal;
mod trace;

#[cfg(test)]
mod tests;

pub use collection::{Change, Changes, Collection, Snapshot, apply_changes};
pub use runtime::{LateWrite, Phase};

/// The late writes reported since the last call (ADR-0013's debug check, on
/// in debug builds): signals written in a phase of a frame after an earlier
/// phase of the same frame read them.
#[must_use]
pub fn take_late_writes() -> Vec<LateWrite> {
    runtime::take_late_writes()
}

/// Start a frame of the late-write check: what is read from here on is this frame's.
pub(crate) fn begin_frame() {
    runtime::begin_frame();
}

/// The phase of the frame running now.
pub(crate) fn phase() -> Phase {
    runtime::with(|rt| rt.phase)
}

/// Run `f` as `phase` of the running frame (the late-write check); `settles`
/// for a layout pass that another follows when it dirties what was read.
/// Run `f` as the render (or setup) of component `name`: it may write no
/// live signal but those it made (checked in a debug build).
pub(crate) fn rendering<R>(name: &'static str, f: impl FnOnce() -> R) -> R {
    runtime::rendering(name, f)
}

pub(crate) fn in_phase<R>(phase: Phase, settles: bool, f: impl FnOnce() -> R) -> R {
    runtime::in_phase(phase, settles, f)
}
pub use computed::Computed;
pub(crate) use effect::on_release;
pub use effect::{Effect, on_cleanup, watch};
#[cfg(all(debug_assertions, test))]
pub(crate) use runtime::arena;
#[cfg(all(debug_assertions, test))]
pub(crate) use runtime::cold_steps;
#[cfg(debug_assertions)]
pub(crate) use runtime::cold_wrote;
pub(crate) use runtime::flush_unbatched;
pub(crate) use runtime::is_cold;
pub(crate) use runtime::making_shared;
pub use scope::Scope;
pub use signal::Signal;
pub use trace::{
    Graph, ReactiveTrace, SignalRef, SignalValue, cleanups, graph, held_bytes, probes, start_trace, stop_trace,
    take_trace, write_probe,
};

/// Run `f` as one update: writes inside it mark what depends on them, and
/// effects run once when it returns (or unwinds). Reads inside it are fresh.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    let _guard = runtime::BatchGuard::new();
    f()
}

/// Run `f` without subscribing the running reader to what `f` reads. What
/// `f` makes still belongs to the running owner.
pub fn untracked<R>(f: impl FnOnce() -> R) -> R {
    let _guard = runtime::OwnerGuard::enter(None, false);
    f()
}

/// Run `f`, then run what depends on each signal and computed it read, as if
/// each had changed (alien-signals' `trigger`): for a value changed in place
/// behind the graph's back.
#[track_caller]
pub fn trigger(f: impl FnOnce()) {
    runtime::trigger(f, std::panic::Location::caller());
}

/// Run `f` with no owner and no reader: what it makes lives until disposed
/// by hand, whatever runs now. For state that outlives any component (a
/// thread's focus, a global setting) made lazily, from wherever it is first
/// touched.
pub fn detached<R>(f: impl FnOnce() -> R) -> R {
    let _guard = runtime::OwnerGuard::enter(None, true);
    f()
}

/// Whether something owns what is made now (a scope, an effect, a computed).
/// What owns what is made now, if it is alive.
pub(crate) fn owner() -> Option<OwnerId> {
    runtime::with(|rt| rt.active_owner.filter(|&o| rt.live(o).is_some())).map(runtime::NodeId::owner_id)
}

/// Run `f` with `owner` (if still alive, and of this thread) owning what
/// it makes, tracking nothing.
pub(crate) fn run_owned<R>(owner: Option<OwnerId>, f: impl FnOnce() -> R) -> R {
    let owner = owner
        .and_then(OwnerId::node)
        .filter(|&o| runtime::with(|rt| rt.live(o).is_some()));
    let _guard = runtime::OwnerGuard::enter(owner, true);
    f()
}

/// Whether `owner` (as [`owner`] gave it) was not disposed; never, named
/// on another thread.
pub(crate) fn is_live(owner: OwnerId) -> bool {
    owner
        .node()
        .and_then(|o| runtime::try_with(|rt| rt.live(o).is_some()))
        .unwrap_or(false)
}

pub(crate) fn has_owner() -> bool {
    runtime::with(|rt| rt.active_owner.is_some_and(|o| rt.live(o).is_some()))
}

/// A component's render node (ADR-0013): `mark` runs (untracked) when
/// something its last tracked run read changed; [`RenderNode::track`] is
/// that run. Owned by the scope it is made in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderNode(runtime::NodeId);

/// While a render's mark runs: the signal whose write woke it.
pub(crate) fn waking() -> Option<SignalRef> {
    runtime::with(|rt| rt.waking)
}

/// What a [`RenderNode`] runs when something it read changed; one may be
/// shared by several ([`RenderNode::marking`]).
pub(crate) type Mark = runtime::Run;

impl RenderNode {
    #[track_caller]
    pub(crate) fn new(mark: impl Fn() + 'static) -> Self {
        let mark: Mark = std::rc::Rc::new(mark);
        Self::marking(&mark)
    }

    /// A render node running `mark`, shared with whoever else holds it.
    #[track_caller]
    pub(crate) fn marking(mark: &Mark) -> Self {
        let at = std::panic::Location::caller();
        let run = std::rc::Rc::clone(mark);
        let id = crate::engine::allocating(|| {
            runtime::with(|rt| {
                let id = rt.alloc(runtime::Kind::Render, runtime::flag::WATCHING, at);
                if let Some(n) = rt.live_mut(id) {
                    n.run = Some(run);
                }
                rt.adopt(id);
                id
            })
        });
        Self(id)
    }

    /// What its last run read.
    pub(crate) fn deps(self) -> Vec<SignalRef> {
        runtime::deps_of(self.0.index)
    }

    /// Whether something its last run read changed since (its mark ran):
    /// it stays so until [`Self::track`] runs it again.
    pub(crate) fn is_marked(self) -> bool {
        runtime::with(|rt| rt.live(self.0).is_some_and(|n| n.flags & runtime::flag::DIRTY != 0))
    }

    /// Whether its last run read anything.
    pub(crate) fn has_deps(self) -> bool {
        runtime::with(|rt| rt.live(self.0).is_some()) && runtime::has_deps(self.0.index)
    }

    /// Take over what `from` read in its last run: a write to any of it
    /// marks this node, not `from`.
    pub(crate) fn take_deps(self, from: Self) {
        let live = runtime::with(|rt| rt.live(self.0).is_some() && rt.live(from.0).is_some());
        if live {
            runtime::transfer_deps(from.0.index, self.0.index);
        }
    }

    /// Take it out of the graph now (one made detached, owned by no scope).
    /// Safe from a `Drop` while the thread unwinds: the runtime is left as
    /// it is when it cannot be reached, and what leaving runs (cleanups,
    /// drops) waits for the next settle, so a panic in it cannot abort.
    pub(crate) fn dispose(self) {
        if runtime::try_with(|rt| rt.dispose(self.0)).is_some() && !std::thread::panicking() {
            runtime::settle();
        }
    }

    /// Run `f`, its reads becoming this render's dependencies.
    pub(crate) fn track<R>(self, f: impl FnOnce() -> R) -> R {
        if runtime::with(|rt| rt.live(self.0).is_none()) {
            // Untracked, still user code: counted as the tracked run is.
            return crate::engine::lend(f);
        }
        runtime::track_render(self.0.index, f)
    }
}
