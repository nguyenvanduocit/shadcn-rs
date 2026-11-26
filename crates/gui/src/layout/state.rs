use std::collections::{HashMap, HashSet};

use crate::geometry::NodeId;

/// A scroll container without a handle, by identity: its [`NodeId`]
/// (ADR-0026, "As built (K4e)"). A component that another replaces at its
/// key is new nodes, with ids of their own, so the new one's list starts at
/// the top; a container hidden and shown again keeps its id and its offset.
pub(crate) type ScrollId = NodeId;

/// Values by scroll container identity ([`ScrollId`]). Each id written is
/// logged, once however often, until the next layout pass takes the log
/// ([`LayoutTree::layout`](super::LayoutTree::layout)): a pass costs what
/// changed, not what is kept, and the log holds at most the ids kept.
#[derive(Clone, Debug)]
pub(crate) struct ByScroller<V> {
    map: HashMap<NodeId, V>,
    /// The ids written since the last layout pass took them (a pass reads
    /// the state shared, and takes them as it places them). The world's
    /// that made the state: a check's copy is its own (review r18,
    /// decision 2).
    pub(super) changed: crate::engine::WorldRefCell<HashSet<NodeId>>,
    /// Entries visited by writes (tests: a write is O(1)).
    #[cfg(test)]
    pub(crate) visited: u64,
}

impl<V> Default for ByScroller<V> {
    fn default() -> Self {
        Self {
            map: HashMap::default(),
            changed: crate::engine::WorldRefCell::new(
                "the scroll containers written (`ByScroller::changed`)",
                HashSet::default(),
            ),
            #[cfg(test)]
            visited: 0,
        }
    }
}

impl<V> ByScroller<V> {
    /// The value container `id` holds.
    pub(crate) fn get(&self, id: &ScrollId) -> Option<&V> {
        self.map.get(id)
    }

    /// Container `id` holds `v`.
    pub(crate) fn insert(&mut self, id: ScrollId, v: V) {
        #[cfg(test)]
        {
            self.visited = self.visited.saturating_add(1);
        }
        let changed = self.changed.get_mut();
        if !changed.contains(&id) {
            changed.insert(id);
        }
        self.map.insert(id, v);
    }

    /// Forget what containers that are gone held (their ids freed): `keep`
    /// says which are still in a tree.
    pub(crate) fn retain(&mut self, keep: impl Fn(NodeId) -> bool) {
        self.map.retain(|&id, _| keep(id));
    }

    /// The ids written since the last call, the log emptied.
    pub(super) fn take_written(&self) -> HashSet<NodeId> {
        self.changed.take()
    }

    /// How many ids the log holds (tests).
    #[cfg(test)]
    pub(crate) fn logged(&self) -> usize {
        self.changed.with(|c| c.len())
    }

    /// Forget every value (tests).
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        let gone: Vec<NodeId> = self.map.drain().map(|(p, _)| p).collect();
        self.changed.get_mut().extend(gone);
    }

    /// The ids holding a value.
    #[cfg(test)]
    pub(super) fn ids(&self) -> impl Iterator<Item = &NodeId> {
        self.map.keys()
    }
}
