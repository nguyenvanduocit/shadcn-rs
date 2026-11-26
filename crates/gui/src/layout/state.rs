use std::collections::{HashMap, HashSet};

use crate::geometry::NodePath;

/// A scroll container without a handle, by identity, as every target is
/// named: its path and the component instance whose output it is (the
/// nearest above it, itself included; else its owner). A component that
/// another replaces at its key leaves its offset behind: the new one's
/// list starts at the top.
pub(crate) type ScrollId = (NodePath, Option<crate::reactive::OwnerId>);

/// Values by scroll container identity ([`ScrollId`]): one entry a path,
/// held for its owner. Found by path and owner at once; another owner's
/// write at a path replaces what the last left there (no scan). Each
/// path written is logged, once however often, until the next layout pass
/// takes the log ([`LayoutTree::layout`](super::LayoutTree::layout)): a pass costs what changed, not
/// what is kept, and the log holds at most the paths kept.
#[derive(Clone, Debug)]
pub(crate) struct ByScroller<V> {
    map: HashMap<NodePath, (Option<crate::reactive::OwnerId>, V)>,
    /// The paths written since the last layout pass took them (a pass
    /// reads the state shared, and takes them as it places them). The
    /// world's that made the state: a check's copy is its own (review r18,
    /// decision 2).
    pub(super) changed: crate::engine::WorldRefCell<HashSet<NodePath>>,
    /// Entries visited by writes (tests: a write is O(1)).
    #[cfg(test)]
    pub(crate) visited: u64,
}

impl<V> Default for ByScroller<V> {
    fn default() -> Self {
        Self {
            map: HashMap::default(),
            changed: crate::engine::WorldRefCell::new(
                "the scroll paths written (`ByScroller::changed`)",
                HashSet::default(),
            ),
            #[cfg(test)]
            visited: 0,
        }
    }
}

impl<V> ByScroller<V> {
    /// The value container `id` holds: its path's, if its owner wrote it.
    pub(crate) fn get(&self, id: &ScrollId) -> Option<&V> {
        self.map.get(&id.0).filter(|(owner, _)| *owner == id.1).map(|(_, v)| v)
    }

    /// Container `id` holds `v`, in place of what was at its path.
    pub(crate) fn insert(&mut self, id: ScrollId, v: V) {
        #[cfg(test)]
        {
            self.visited = self.visited.saturating_add(1);
        }
        let changed = self.changed.get_mut();
        if !changed.contains(&id.0) {
            changed.insert(id.0.clone());
        }
        self.map.insert(id.0, (id.1, v));
    }

    /// The paths written since the last call, the log emptied.
    pub(super) fn take_written(&self) -> HashSet<NodePath> {
        self.changed.take()
    }

    /// How many paths the log holds (tests).
    #[cfg(test)]
    pub(crate) fn logged(&self) -> usize {
        self.changed.borrow().len()
    }

    /// Forget every value (tests).
    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        let gone: Vec<NodePath> = self.map.drain().map(|(p, _)| p).collect();
        self.changed.get_mut().extend(gone);
    }

    /// The paths holding a value.
    #[cfg(test)]
    pub(super) fn paths(&self) -> impl Iterator<Item = &NodePath> {
        self.map.keys()
    }
}
