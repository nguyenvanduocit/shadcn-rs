//! `Scope`: owns what is made inside it, and disposes it all at once.

use std::panic::Location;

use super::runtime::{self, Kind, NodeId, flag};

/// An owner. Signals, computeds, effects and scopes made inside
/// [`Scope::run`] belong to it, and [`Scope::dispose`] disposes them all,
/// last made first, after the scopes and effects it owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    id: NodeId,
    created_at: &'static Location<'static>,
}

impl Scope {
    /// A scope owned by what runs now (an effect, a computed or a scope), or
    /// a root scope if nothing does.
    #[track_caller]
    #[expect(clippy::new_without_default, reason = "a scope records where it was made")]
    pub fn new() -> Self {
        let created_at = Location::caller();
        let id = crate::engine::allocating(|| {
            runtime::with(|rt| {
                let id = rt.alloc(Kind::Scope, flag::NONE, created_at);
                rt.adopt(id);
                id
            })
        });
        Self { id, created_at }
    }

    /// A root scope, owned by nothing, whatever runs now.
    #[track_caller]
    pub fn detached() -> Self {
        let created_at = Location::caller();
        let id = crate::engine::allocating(|| runtime::with(|rt| rt.alloc(Kind::Scope, flag::NONE, created_at)));
        Self { id, created_at }
    }

    /// Run `f` with this scope as the owner of what it makes. Reads in `f`
    /// subscribe nothing.
    pub fn run<R>(&self, f: impl FnOnce() -> R) -> R {
        let alive = runtime::with(|rt| rt.live(self.id).is_some());
        if !alive {
            runtime::disposed("scope", self.created_at);
        }
        let _guard = runtime::OwnerGuard::enter(Some(self.id), true);
        f()
    }

    /// Its node in the runtime: what owns what it makes.
    pub(crate) fn id(&self) -> NodeId {
        self.id
    }

    /// Whether it was not disposed.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        runtime::with(|rt| rt.live(self.id).is_some())
    }

    /// Dispose everything it owns, then itself.
    pub fn dispose(self) {
        runtime::with(|rt| rt.dispose(self.id));
        runtime::settle();
    }

    /// [`Self::dispose`], as the engine's re-run retirement of this scope
    /// (ADR-0035, D3): a live instance's old render scope, a failed
    /// render's tree. It permits this scope's disposal alone, in a check;
    /// what its settle drops (cleanups, captured app values) is not.
    pub(crate) fn retire(self) {
        runtime::with(|rt| rt.retire(self.id));
        runtime::settle();
    }
}
