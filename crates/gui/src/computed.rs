//! `Computed<T>`: lazy, cached, dependency-tracked derived value.
//!
//! # Design: lazy pull via bridge `EffectInner`
//!
//! Each `Computed<T>` allocates one `NodeId` and registers a "bridge"
//! [`EffectInner`] in `REGISTRY` under that id.  Signals see the bridge as an
//! ordinary subscriber.  When a signal fires:
//!
//! 1. `Signal::notify()` marks the bridge dirty and pushes it to `PENDING`.
//! 2. `flush()` calls the bridge's closure (not the user's computation).
//! 3. The bridge sets `ComputedInner::dirty = true` and walks
//!    `ComputedInner::subs`, marking every downstream subscriber (Effect or
//!    another Computed's bridge) dirty and pushing its id to `PENDING`.
//! 4. The Computed itself never enters `PENDING`.  It recomputes lazily in
//!    `get()` the next time a caller invokes it.
//!
//! This avoids the double-run bug that arises when `get()` itself triggers
//! subscriber notifications: all scheduling happens during the signal-write
//! phase, before any effect runs.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use crate::reactive::{
    ACTIVE, COMPUTEDS, EffectInner, NodeId, PENDING, REGISTRY, Source, alloc_id, count_down, count_up, drop_subs,
    track, untrack,
};

// ---------------------------------------------------------------------------
// ComputedInner
// ---------------------------------------------------------------------------

struct ComputedInner<T: Clone + PartialEq> {
    /// Shared `NodeId` used by both this inner struct and the bridge registered
    /// in `REGISTRY`.  Signals subscribe `self.id` (the bridge's id) so that
    /// `Signal::notify()` can reach the bridge through the existing path.
    id: NodeId,
    /// The pure computation; must not perform I/O or write to signals.
    f: Box<dyn Fn() -> T>,
    /// Cached result; `None` before the first `get()`.
    cached: RefCell<Option<T>>,
    /// `true` when a dependency has changed since the last recomputation.
    /// Starts `true` to force recomputation on the first `get()`.
    dirty: Cell<bool>,
    /// `NodeId`s of downstream subscribers: Effects and bridges of other
    /// `Computed`s that called `get()` while they were the active node.
    subs: RefCell<HashSet<NodeId>>,
    /// Keeps the bridge `Rc<EffectInner>` alive so `REGISTRY` holds a live
    /// `Weak`; it holds what the last computation read.  Dropped together
    /// with the last `Computed<T>` handle, which unregisters it.
    bridge: Rc<EffectInner>,
}

// ---------------------------------------------------------------------------
// Computed<T>
// ---------------------------------------------------------------------------

/// A lazy, cached derived value that recomputes only when dirty and queried.
///
/// `Computed<T>` is both a **subscriber** (it subscribes to every signal or
/// other `Computed` it reads) and a **source** (effects or other `Computed`s
/// that read it subscribe to it).
///
/// Dirtiness is propagated **eagerly** through a bridge `EffectInner` on each
/// signal write; recomputation is **deferred** until `get()` is called.
///
/// Cloning is cheap (`Rc` clone).  The bridge is unregistered from `REGISTRY`
/// when the last handle is dropped.
pub struct Computed<T: Clone + PartialEq>(Rc<ComputedInner<T>>);

impl<T: Clone + PartialEq + 'static> Computed<T> {
    /// Create a new `Computed<T>` from a pure function.
    ///
    /// `f` is **not** called here; the first `get()` triggers the initial
    /// computation and establishes all subscriptions.
    pub fn new(f: impl Fn() -> T + 'static) -> Self {
        let id = alloc_id();

        // `Rc::new_cyclic` lets us capture `Weak<ComputedInner<T>>` in the
        // bridge closure before the `Rc` is fully constructed.  During
        // construction `Weak::upgrade()` returns `None`; at runtime (called
        // from `flush()`) it always returns `Some`.
        let inner: Rc<ComputedInner<T>> = Rc::new_cyclic(|weak: &Weak<ComputedInner<T>>| {
            let weak_inner = Weak::clone(weak);

            // The bridge closure:
            //   • sets `ComputedInner::dirty = true`,
            //   • walks `ComputedInner::subs`, marks each downstream node
            //     dirty, and extends `PENDING`.
            //
            // No `start_batch`/`end_batch` is called here because the bridge
            // already runs inside `flush()`, which drains `PENDING` in a loop;
            // entries added in this closure are picked up in the next iteration.
            let bridge_f: Box<dyn Fn()> = Box::new(move || {
                let Some(computed) = weak_inner.upgrade() else {
                    return;
                };

                computed.dirty.set(true);

                let downstream: Vec<NodeId> = computed.subs.borrow().iter().copied().collect();

                for sub_id in &downstream {
                    if let Some(eff) = REGISTRY.with(|r| r.borrow().get(sub_id).and_then(|w| w.upgrade())) {
                        eff.dirty.set(true);
                    }
                }

                // Extend PENDING without triggering a new flush; the outer
                // flush loop picks these ids up in its next iteration.
                PENDING.with(|p| p.borrow_mut().extend(downstream));
            });

            // Registered so `Signal::notify()` can find the bridge by `id`.
            let bridge = EffectInner::register(id, bridge_f);
            count_up(&COMPUTEDS, 1);

            ComputedInner {
                id,
                f: Box::new(f),
                cached: RefCell::new(None),
                dirty: Cell::new(true),
                subs: RefCell::new(HashSet::new()),
                bridge,
            }
        });

        Self(inner)
    }

    /// Return the current value, tracking any active Effect or `Computed` as a
    /// downstream subscriber.
    ///
    /// Steps:
    /// 1. Register `ACTIVE` in `self.subs` (subscribe the caller to this node).
    /// 2. If dirty or never computed: re-run `f` with `ACTIVE = self.id` so
    ///    that signals inside `f` subscribe the bridge; update the cache.
    /// 3. Return the cached value (cloned).
    pub fn get(&self) -> T {
        // Step 1: subscribe the active caller (Effect or bridge) to this node.
        track(&self.0.subs, || -> Weak<dyn Source> {
            Rc::<ComputedInner<T>>::downgrade(&self.0)
        });

        // Step 2: recompute if dirty or on the very first call.
        if self.0.dirty.get() || self.0.cached.borrow().is_none() {
            // Run the computation with ACTIVE = self.id (the bridge id) so
            // that any Signal or Computed read inside f() subscribes the bridge,
            // wiring it into the dependency graph for future notifications.
            self.0.bridge.forget_sources();
            let prev = ACTIVE.with(|a| *a.borrow());
            ACTIVE.with(|a| *a.borrow_mut() = Some(self.0.id));
            let new_val = (self.0.f)();
            ACTIVE.with(|a| *a.borrow_mut() = prev);

            self.0.dirty.set(false);
            *self.0.cached.borrow_mut() = Some(new_val.clone());
            return new_val;
        }

        // Step 3: cache is populated and clean; clone and return.
        self.0
            .cached
            .borrow()
            .clone()
            .unwrap_or_else(|| unreachable!("cache is Some: dirty=false and !is_none()"))
    }
}

impl<T: Clone + PartialEq> Clone for Computed<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: Clone + PartialEq> Source for ComputedInner<T> {
    fn unsubscribe(&self, id: NodeId) {
        untrack(&self.subs, id);
    }
}

impl<T: Clone + PartialEq> Drop for ComputedInner<T> {
    fn drop(&mut self) {
        count_down(&COMPUTEDS, 1);
        drop_subs(&self.subs);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::{Effect, Signal};
    use std::cell::Cell;
    use std::rc::Rc;

    /// A signal change propagates through a computed and updates its value.
    #[test]
    fn computed_tracks_signal_lazily() {
        let s = Signal::new(1_i32);
        let s2 = s.clone();
        let c = Computed::new(move || s2.get().saturating_mul(2));

        assert_eq!(c.get(), 2);
        s.set(5);
        assert_eq!(c.get(), 10);
        s.set(0);
        assert_eq!(c.get(), 0);
    }

    /// The computation function is not called again when the signal is unchanged.
    #[test]
    fn computed_caches_when_signal_unchanged() {
        let s = Signal::new(3_i32);
        let call_count = Rc::new(Cell::new(0_u32));

        let s2 = s.clone();
        let cc = Rc::clone(&call_count);
        let c = Computed::new(move || {
            cc.set(cc.get().saturating_add(1));
            s2.get().saturating_add(1)
        });

        assert_eq!(c.get(), 4);
        assert_eq!(call_count.get(), 1, "first get must call fn once");

        // Second read with no intervening signal write must hit the cache.
        assert_eq!(c.get(), 4);
        assert_eq!(call_count.get(), 1, "fn must not be called again when clean");

        // A read after a signal change must recompute exactly once.
        s.set(10);
        assert_eq!(c.get(), 11);
        assert_eq!(call_count.get(), 2, "fn must be called once after signal change");

        // Immediately cached again.
        assert_eq!(c.get(), 11);
        assert_eq!(call_count.get(), 2, "fn must not be called again after recompute");
    }

    /// An effect that reads a computed re-runs when the computed's signal changes.
    #[test]
    fn computed_propagates_to_effect() {
        let s = Signal::new(0_i32);
        let s2 = s.clone();
        let c = Computed::new(move || s2.get().saturating_add(100));

        let seen = Rc::new(Cell::new(0_i32));
        let seen2 = Rc::clone(&seen);
        let _e = Effect::new(move || {
            seen2.set(c.get());
        });

        // Effect runs immediately on creation.
        assert_eq!(seen.get(), 100);

        s.set(5);
        assert_eq!(seen.get(), 105);

        s.set(10);
        assert_eq!(seen.get(), 110);
    }

    /// Chained computeds: c2 reads c1; changing the signal propagates through both.
    #[test]
    fn computed_chain_propagates() {
        let s = Signal::new(1_i32);
        let s2 = s.clone();
        let c1 = Computed::new(move || s2.get().saturating_mul(10));
        let c2 = Computed::new(move || c1.get().saturating_add(1));

        assert_eq!(c2.get(), 11);

        s.set(3);
        assert_eq!(c2.get(), 31);
    }
}
