//! The compositor's scroll on its way to the UI (ADR-0033, which supersedes
//! ADR-0032's I1 and I2): what the render thread moved each scroll
//! container by and has not handed the UI yet, and what it handed the UI
//! that no committed base holds yet. The render thread alone owns it.
//!
//! Chromium's compositor keeps a scroll offset as a synced property: the
//! main thread's value is a base, the compositor's delta rides on top, the
//! compositor thread pulls the delta as it sends the main thread a frame,
//! and a commit brings back the new base, the delta it sent dropped and the
//! rest kept on top (`cc/trees/property_tree.cc`, `PullDeltaForMainThread`,
//! `PushScrollUpdatesFromMainThread`). Every input reaches the UI through
//! the render thread here, so it pulls not only at a main frame but as it
//! hands the UI any input ([`Outbox::pull`]): the UI takes in, before it
//! handles an input, what every wheel the render thread applied before it
//! moved, and nothing a later one moves, by construction.
//!
//! **Storage.** One sum a container for what was moved since the last pull,
//! and one sum a container for each pull the UI has not committed yet (a
//! few containers each: those the reader scrolls). The pulls held are the
//! inputs and main frames on their way to the UI or not yet in a commit,
//! which the UI's queue holds anyway; never one entry a wheel event.

use std::collections::VecDeque;

use foldhash::HashMap;

use super::motion::Exact;
use super::{NodeKey, ScrollDelta};
use crate::reactive::OwnerId;

/// A scroll container's identity on the render thread
/// ([`super::ScrollNode::instance`]).
pub(super) type ScrollerId = u64;

/// What the compositor moved one scroll container by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Moved {
    pub(super) key: NodeKey,
    pub(super) owner: Option<OwnerId>,
    /// Of the reader's input: a notched turn, a gesture's samples, the
    /// platform's momentum. Never dropped (ADR-0032, I4). Exactly
    /// ([`super::motion`]), as `synthetic`.
    pub(super) real: Exact,
    /// The compositor's own motion: its fling where the platform has
    /// none. A jump stops it, and what of it the UI has not taken goes.
    pub(super) synthetic: Exact,
    /// Wheel steps it took, moved or held against its edge.
    pub(super) steps: u32,
    /// The container's generation when it was last moved
    /// ([`super::ScrollNode::generation`]): a commit with a jump zeroes
    /// the synthetic motion moved before it, so what is left is of this one.
    pub(super) generation: u64,
}

impl Moved {
    pub(super) fn total(&self) -> Exact {
        super::motion::add2(self.real, self.synthetic)
    }
}

/// Each container's sums, by the container's identity: a lookup is one
/// probe however many containers it holds, so nothing done here is
/// quadratic in them (Codex r8 6).
#[derive(Clone, Debug, Default)]
pub(super) struct Sums(pub(super) HashMap<ScrollerId, Moved>);

impl Sums {
    /// `id`'s entry, made empty if it has none.
    pub(super) fn entry(&mut self, id: ScrollerId, key: NodeKey, owner: Option<OwnerId>) -> &mut Moved {
        super::scroll::visit();
        self.0.entry(id).or_insert(Moved {
            key,
            owner,
            real: (0, 0),
            synthetic: (0, 0),
            steps: 0,
            generation: 0,
        })
    }
}

/// What a pull handed the UI ([`Outbox::pull`]): what the compositor moved
/// each container by since the last pull, in one order whatever the map's,
/// and the pull's number, which a commit names to say its bases hold it
/// and every pull before ([`super::Commit::taken`]). Attached to the input
/// or main frame it rides with; empty when nothing moved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pulled {
    /// The number of the last pull made when this was: what the UI has
    /// taken in once it took this in.
    pub(crate) take: u64,
    pub(crate) deltas: Vec<ScrollDelta>,
}

impl Pulled {
    /// Whether it moves nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.deltas.is_empty()
    }
}

/// The compositor's scroll on its way to the UI, owned by the render
/// thread: moved and not pulled, then pulled and not committed.
#[derive(Debug, Default)]
pub struct Outbox {
    /// Moved since the last pull.
    pub(super) pending: Sums,
    /// Pulls made so far.
    pub(super) takes: u64,
    /// Pulled and not yet in a committed base: (the pull's number, its
    /// sums), oldest first.
    pub(super) sent: VecDeque<(u64, Sums)>,
    /// Sums emptied, kept for the next pull: scroll going round allocates
    /// nothing once warm.
    spare: Vec<Sums>,
}

impl Outbox {
    /// `sums`, emptied, kept for another pull.
    pub(super) fn recycle(&mut self, mut sums: Sums) {
        sums.0.clear();
        self.spare.push(sums);
    }

    /// Hand the UI, into `into`, what was moved since the last pull, each
    /// container's sum once, and keep it as sent until a commit's bases
    /// hold it. The pull's number, the last one's when nothing moved.
    pub(crate) fn pull(&mut self, into: &mut Vec<ScrollDelta>) -> u64 {
        into.clear();
        if self.pending.0.is_empty() {
            return self.takes;
        }
        self.takes = self.takes.saturating_add(1);
        into.extend(self.pending.0.iter().map(|(&id, m)| ScrollDelta {
            key: m.key,
            owner: m.owner,
            instance: id,
            delta: m.total(),
            synthetic: m.synthetic,
            generation: m.generation,
            reach: (0.0, 0.0),
            steps: m.steps,
        }));
        // In one order whatever the map's: a replay takes them alike.
        into.sort_unstable_by_key(|d| (d.key, d.instance));
        let fresh = self.spare.pop().unwrap_or_default();
        let sums = std::mem::replace(&mut self.pending, fresh);
        self.sent.push_back((self.takes, sums));
        self.takes
    }

    /// Whether the render thread moved anything the UI has not taken: it
    /// is owed a main frame.
    pub(crate) fn untaken(&self) -> bool {
        !self.pending.0.is_empty()
    }

    /// The entries it holds, pending and sent alike (a memory test).
    #[cfg(test)]
    pub(crate) fn entries(&self) -> usize {
        self.sent
            .iter()
            .fold(self.pending.0.len(), |n, (_, s)| n.saturating_add(s.0.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push(o: &mut Outbox, id: u64, dy: f32) {
        let real = &mut o.pending.entry(id, NodeKey(id), None).real.1;
        *real = real.saturating_add(super::super::motion::units(dy));
    }

    fn pulled(o: &mut Outbox) -> Vec<(NodeKey, f32)> {
        let mut into = Vec::new();
        o.pull(&mut into);
        into.iter()
            .map(|d| (d.key, super::super::motion::px(d.delta.1)))
            .collect()
    }

    /// Moved, pulled for an input, moved again: the input's pull has what
    /// came before it alone, the next pull the rest; nothing is left.
    #[test]
    fn a_pull_has_what_was_moved_since_the_last() {
        let mut o = Outbox::default();
        push(&mut o, 1, 10.0);
        push(&mut o, 1, 20.0);
        assert_eq!(pulled(&mut o), [(NodeKey(1), 30.0)]);
        push(&mut o, 1, 40.0);
        assert_eq!(pulled(&mut o), [(NodeKey(1), 40.0)]);
        assert!(pulled(&mut o).is_empty(), "nothing moved since");
        assert!(!o.untaken());
        assert_eq!(o.takes, 2, "an empty pull is no take");
    }

    /// A UI stalled while 10,000 wheels are applied with no input between
    /// them keeps one entry for each container: storage is the
    /// containers', not the events'.
    #[test]
    fn untaken_scroll_is_one_sum_a_container_however_many_wheels() {
        let mut o = Outbox::default();
        for k in 0..10_000_u32 {
            push(&mut o, 1 + u64::from(k % 2), 1.0);
        }
        assert_eq!(o.entries(), 2);
        assert_eq!(pulled(&mut o), [(NodeKey(1), 5_000.0), (NodeKey(2), 5_000.0)]);
    }
}
