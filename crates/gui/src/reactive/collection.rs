//! `Collection<T>`: a list that is one source in the graph, not one signal
//! per item (ADR-0012). It holds an immutable snapshot that shares structure
//! (a [`SumTree`], O(1) to clone, O(log n) to edit) and a log of the changes
//! made to it, tagged with versions. A reader keeps the version it last saw
//! and asks for the changes since, so a list of a million rows redoes only
//! what changed; a reader too far behind is told to start again (`Reset`).

use std::collections::VecDeque;
use std::fmt;
use std::panic::Location;

use super::signal::Signal;
use crate::sum_tree::{self, SumTree};

/// How many changes the log keeps; a reader further behind gets `Reset`.
const LOG_KEEP: usize = 4096;

/// One change to a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// Items `at..at + removed` were replaced by `inserted` new ones.
    Splice { at: usize, removed: usize, inserted: usize },
    /// Item `at` changed in place.
    Update { at: usize },
    /// Items `from..from + len` moved to start at `to` (counted after they left).
    Move { from: usize, len: usize, to: usize },
    /// Everything changed: read the snapshot anew.
    Reset,
}

/// What changed since a version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Changes {
    /// These, in order.
    Log(Vec<Change>),
    /// More than the log keeps, or a version it never had: start again.
    Reset,
}

/// An item as the tree stores it: no summary of its own.
#[derive(Clone, Debug, PartialEq)]
struct Plain<T>(T);

impl<T: Clone> sum_tree::Item for Plain<T> {
    type Summary = ();
    fn summary(&self) {}
}

/// A collection's contents at one version: cheap to clone, never changes,
/// sendable to a worker when `T` is.
pub struct Snapshot<T: Clone> {
    items: SumTree<Plain<T>>,
    version: u64,
}

impl<T: Clone> Clone for Snapshot<T> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            version: self.version,
        }
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for Snapshot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("version", &self.version)
            .field("len", &self.len())
            .finish()
    }
}

impl<T: Clone> Snapshot<T> {
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The version this snapshot is of.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Item `i`, in O(log n).
    #[must_use]
    pub fn get(&self, i: usize) -> Option<&T> {
        self.items.get(i).map(|p| &p.0)
    }

    /// The items in order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.items.iter().map(|p| &p.0)
    }

    /// The items from `i` on (a visible window), reaching `i` in O(log n).
    pub fn iter_from(&self, i: usize) -> impl Iterator<Item = &T> {
        self.items.iter_from(i).map(|p| &p.0)
    }

    /// Whether `other` is this very snapshot.
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        self.items.ptr_eq(&other.items)
    }
}

struct State<T: Clone> {
    items: SumTree<Plain<T>>,
    version: u64,
    /// `(version after the change, change)`, oldest first.
    log: VecDeque<(u64, Change)>,
}

impl<T: Clone> State<T> {
    fn record(&mut self, change: Change) {
        self.version = self.version.wrapping_add(1);
        if self.log.len() >= LOG_KEEP {
            self.log.pop_front();
        }
        self.log.push_back((self.version, change));
    }
}

/// A list read and written as one reactive source. A `Copy` handle, owned by
/// its scope like a signal.
pub struct Collection<T: Clone + 'static> {
    state: Signal<State<T>>,
}

impl<T: Clone + 'static> Clone for Collection<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static> Copy for Collection<T> {}

impl<T: Clone + 'static> PartialEq for Collection<T> {
    fn eq(&self, other: &Self) -> bool {
        self.state == other.state
    }
}

impl<T: Clone + 'static> fmt::Debug for Collection<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Collection(made at {})", self.state.created_at())
    }
}

impl<T: Clone + 'static> Collection<T> {
    #[track_caller]
    pub fn new(items: impl IntoIterator<Item = T>) -> Self {
        let state = State {
            items: items.into_iter().map(Plain).collect(),
            version: 0,
            log: VecDeque::new(),
        };
        Self {
            state: Signal::new(state),
        }
    }

    /// Where it was made.
    #[must_use]
    pub fn created_at(&self) -> &'static Location<'static> {
        self.state.created_at()
    }

    /// The contents now, subscribing the running reader. O(1).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<T> {
        self.state.with(|s| Snapshot {
            items: s.items.clone(),
            version: s.version,
        })
    }

    /// How many items, subscribing the running reader.
    #[must_use]
    pub fn len(&self) -> usize {
        self.state.with(|s| s.items.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The current version, subscribing the running reader.
    #[must_use]
    pub fn version(&self) -> u64 {
        self.state.with(|s| s.version)
    }

    /// What changed after `version`, subscribing the running reader.
    #[must_use]
    pub fn changes_since(&self, version: u64) -> Changes {
        self.state.with(|s| {
            if version == s.version {
                return Changes::Log(Vec::new());
            }
            // The log holds versions `oldest..=s.version`, each the version
            // after its change; a reader at `version` needs those above it.
            let oldest = s.log.front().map_or(s.version, |(v, _)| v.wrapping_sub(1));
            if version < oldest || version > s.version {
                return Changes::Reset;
            }
            let log: Vec<Change> = s.log.iter().filter(|(v, _)| *v > version).map(|(_, c)| *c).collect();
            if log.contains(&Change::Reset) {
                Changes::Reset
            } else {
                Changes::Log(log)
            }
        })
    }

    fn edit(&self, f: impl FnOnce(&mut State<T>)) {
        self.state.update(f);
    }

    /// Replace items `at..at + removed` with `inserted`.
    pub fn splice(&self, at: usize, removed: usize, inserted: impl IntoIterator<Item = T>) {
        let inserted: Vec<Plain<T>> = inserted.into_iter().map(Plain).collect();
        self.edit(|s| {
            let at = at.min(s.items.len());
            let removed = removed.min(s.items.len().saturating_sub(at));
            let count = inserted.len();
            s.items.splice(at, removed, inserted);
            s.record(Change::Splice {
                at,
                removed,
                inserted: count,
            });
        });
    }

    pub fn push(&self, item: T) {
        let at = self.state.with_untracked(|s| s.items.len());
        self.splice(at, 0, [item]);
    }

    pub fn insert(&self, at: usize, item: T) {
        self.splice(at, 0, [item]);
    }

    pub fn remove(&self, at: usize) {
        self.splice(at, 1, []);
    }

    /// Change item `at` in place.
    pub fn update(&self, at: usize, f: impl FnOnce(&mut T)) {
        self.edit(|s| {
            if s.items.update(at, |p| f(&mut p.0)).is_some() {
                s.record(Change::Update { at });
            }
        });
    }

    /// Move items `from..from + len` to start at `to`, counted after they left.
    pub fn move_range(&self, from: usize, len: usize, to: usize) {
        self.edit(|s| {
            let from = from.min(s.items.len());
            let len = len.min(s.items.len().saturating_sub(from));
            let moved: Vec<Plain<T>> = s.items.iter_from(from).take(len).cloned().collect();
            s.items.splice(from, len, []);
            let to = to.min(s.items.len());
            s.items.splice(to, 0, moved);
            s.record(Change::Move { from, len, to });
        });
    }

    /// Replace everything.
    pub fn reset(&self, items: impl IntoIterator<Item = T>) {
        let items: SumTree<Plain<T>> = items.into_iter().map(Plain).collect();
        self.edit(|s| {
            s.items = items;
            s.record(Change::Reset);
        });
    }

    /// Take it out of the graph now, before its scope ends.
    pub fn dispose(self) {
        self.state.dispose();
    }
}

/// Apply `changes` to `mirror`, a reader's copy, taking new items from
/// `now`: what a list does with the log to redo only what changed. Returns
/// false on `Reset` (the reader starts again from `now`).
pub fn apply_changes<T: Clone>(mirror: &mut Vec<T>, changes: &Changes, now: &Snapshot<T>) -> bool {
    let Changes::Log(log) = changes else {
        *mirror = now.iter().cloned().collect();
        return false;
    };
    // Changes are replayed in order; an inserted or updated item is read from
    // `now` at its final place only when no later change moved it, so items
    // are taken from `now` in one pass after the structure is replayed.
    let mut fresh: Vec<Option<T>> = mirror.drain(..).map(Some).collect();
    for change in log {
        match *change {
            Change::Splice { at, removed, inserted } => {
                let at = at.min(fresh.len());
                let end = at.saturating_add(removed).min(fresh.len());
                fresh.splice(at..end, std::iter::repeat_n(None, inserted));
            }
            Change::Update { at } => {
                if let Some(slot) = fresh.get_mut(at) {
                    *slot = None;
                }
            }
            Change::Move { from, len, to } => {
                let end = from.saturating_add(len).min(fresh.len());
                let moved: Vec<Option<T>> = fresh.drain(from.min(end)..end).collect();
                let to = to.min(fresh.len());
                fresh.splice(to..to, moved);
            }
            Change::Reset => {
                *mirror = now.iter().cloned().collect();
                return false;
            }
        }
    }
    *mirror = fresh
        .into_iter()
        .zip(now.iter())
        .map(|(kept, new)| kept.unwrap_or_else(|| new.clone()))
        .collect();
    true
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::reactive::{Effect, graph};

    #[test]
    fn a_million_rows_are_one_node_and_an_edit_wakes_its_reader_once() {
        let rows = Collection::new(0..1_000_000_u32);
        let runs = Rc::new(Cell::new(0_u32));
        let r = Rc::clone(&runs);
        Effect::new(move || {
            std::hint::black_box(rows.len());
            r.set(r.get().saturating_add(1));
        });
        assert_eq!(graph().signals, 1);
        assert_eq!(graph().subscriptions, 1);
        rows.update(500_000, |v| *v = 0);
        rows.splice(10, 5, [7, 8]);
        assert_eq!(runs.get(), 3);
        assert_eq!(rows.snapshot().get(10), Some(&7));
        assert_eq!(rows.len(), 999_997);
    }

    #[test]
    fn a_snapshot_does_not_see_later_edits() {
        let rows = Collection::new(0..10_u32);
        let before = rows.snapshot();
        rows.remove(0);
        assert_eq!(before.get(0), Some(&0));
        assert_eq!(rows.snapshot().get(0), Some(&1));
        assert_eq!(before.version().wrapping_add(1), rows.version());
    }

    #[test]
    fn changes_since_lists_what_a_reader_missed_and_resets_when_too_far_behind() {
        let rows = Collection::new(0..10_u32);
        let v0 = rows.version();
        rows.push(10);
        rows.update(3, |v| *v = 33);
        rows.move_range(0, 2, 5);
        assert_eq!(
            rows.changes_since(v0),
            Changes::Log(vec![
                Change::Splice {
                    at: 10,
                    removed: 0,
                    inserted: 1
                },
                Change::Update { at: 3 },
                Change::Move { from: 0, len: 2, to: 5 }
            ])
        );
        assert_eq!(rows.changes_since(rows.version()), Changes::Log(Vec::new()));
        for _ in 0..LOG_KEEP {
            rows.update(0, |v| *v = v.wrapping_add(1));
        }
        assert_eq!(rows.changes_since(v0), Changes::Reset);
        rows.reset([1, 2, 3]);
        assert_eq!(rows.changes_since(rows.version().wrapping_sub(1)), Changes::Reset);
    }

    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            usize::try_from(self.0.checked_rem(u64::try_from(n.max(1)).unwrap()).unwrap()).unwrap()
        }
    }

    /// The plan's R4 check: a reader applying the log ends where a rebuild
    /// does, over random edits.
    #[test]
    fn applying_the_log_equals_rebuilding() {
        for seed in 1..=200_u64 {
            let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
            let rows = Collection::new(0..u32::try_from(rng.below(50)).unwrap());
            let mut mirror: Vec<u32> = rows.snapshot().iter().copied().collect();
            let mut seen = rows.version();
            let mut next = 1_000_u32;
            for step in 0..80 {
                for _ in 0..=rng.below(4) {
                    let len = rows.snapshot().len();
                    match rng.below(6) {
                        0 => rows.insert(rng.below(len.saturating_add(1)), next),
                        1 if len > 0 => rows.remove(rng.below(len)),
                        2 if len > 0 => rows.update(rng.below(len), |v| *v = v.wrapping_add(100_000)),
                        3 => {
                            let at = rng.below(len.saturating_add(1));
                            let removed = rng.below(len.saturating_sub(at).saturating_add(1));
                            let k = rng.below(6);
                            rows.splice(
                                at,
                                removed,
                                (0..k).map(|i| next.wrapping_add(u32::try_from(i).unwrap())),
                            );
                        }
                        4 if len > 0 => {
                            let from = rng.below(len);
                            let n = rng.below(len.saturating_sub(from)).saturating_add(1);
                            let to = rng.below(len.saturating_sub(n).saturating_add(1));
                            rows.move_range(from, n, to);
                        }
                        5 if rng.below(20) == 0 => rows.reset((0..rng.below(30)).map(|i| u32::try_from(i).unwrap())),
                        _ => rows.push(next),
                    }
                    next = next.wrapping_add(10);
                }
                let now = rows.snapshot();
                apply_changes(&mut mirror, &rows.changes_since(seen), &now);
                seen = now.version();
                assert!(mirror.iter().eq(now.iter()), "seed {seed} step {step}");
            }
        }
    }
}
