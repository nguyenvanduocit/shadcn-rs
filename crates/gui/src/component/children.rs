//! An instance's children, in the order they were placed (ADR-0026,
//! "Teardown has one order"): a map by slot that also keeps an order, so
//! nothing walks them in a hash map's order, which changes from run to run.

use std::collections::HashMap;

use super::{Id, InstanceKey};

/// Children by slot, in order. Removal leaves a hole that the next
/// compaction closes, so a removal is O(1) and the order stays.
#[derive(Clone, Default)]
pub(super) struct Children {
    order: Vec<Option<(InstanceKey, Id)>>,
    index: HashMap<InstanceKey, usize>,
}

impl Children {
    pub(super) fn with_capacity(n: usize) -> Self {
        Self {
            order: Vec::with_capacity(n),
            index: HashMap::with_capacity(n),
        }
    }

    pub(super) fn len(&self) -> usize {
        self.index.len()
    }

    pub(super) fn get(&self, slot: &InstanceKey) -> Option<&Id> {
        let at = *self.index.get(slot)?;
        self.order.get(at)?.as_ref().map(|(_, id)| id)
    }

    pub(super) fn contains_key(&self, slot: &InstanceKey) -> bool {
        self.index.contains_key(slot)
    }

    /// Put `id` at `slot`: in place if the slot is there, else last.
    pub(super) fn insert(&mut self, slot: InstanceKey, id: Id) {
        if let Some(&at) = self.index.get(&slot)
            && let Some(entry) = self.order.get_mut(at)
        {
            *entry = Some((slot, id));
            return;
        }
        self.index.insert(slot, self.order.len());
        self.order.push(Some((slot, id)));
    }

    pub(super) fn remove(&mut self, slot: &InstanceKey) -> Option<Id> {
        let at = self.index.remove(slot)?;
        let gone = self.order.get_mut(at).and_then(Option::take).map(|(_, id)| id);
        // Holes are closed once they are half.
        if self.order.len() > 8 && self.index.len() < self.order.len() / 2 {
            self.compact();
        }
        gone
    }

    fn compact(&mut self) {
        self.order.retain(Option::is_some);
        for (at, entry) in self.order.iter().enumerate() {
            if let Some((slot, _)) = entry {
                self.index.insert(*slot, at);
            }
        }
    }

    /// The children, in order.
    pub(super) fn iter(&self) -> impl DoubleEndedIterator<Item = (&InstanceKey, &Id)> {
        self.order.iter().flatten().map(|(slot, id)| (slot, id))
    }

    /// The children's ids, in order.
    pub(super) fn values(&self) -> impl DoubleEndedIterator<Item = &Id> {
        self.iter().map(|(_, id)| id)
    }

    /// Put the children in the order of `ids` (each one of them, a control
    /// node's rows after a commit); one not named keeps its place after.
    pub(super) fn reorder(&mut self, ids: &[Id]) {
        let mut by_id: HashMap<Id, InstanceKey> = HashMap::with_capacity(self.index.len());
        for (slot, id) in self.iter() {
            by_id.insert(*id, *slot);
        }
        let mut order = Vec::with_capacity(self.index.len());
        for id in ids {
            if let Some(slot) = by_id.remove(id) {
                order.push(Some((slot, *id)));
            }
        }
        for entry in self.order.drain(..).flatten() {
            if by_id.contains_key(&entry.1) {
                order.push(Some(entry));
            }
        }
        self.order = order;
        self.compact();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(n: u64) -> InstanceKey {
        InstanceKey {
            path: n,
            type_id: std::any::TypeId::of::<()>(),
        }
    }

    fn id(n: usize) -> Id {
        Id { slot: n, generation: 0 }
    }

    #[test]
    fn children_keep_the_order_they_were_placed_in() {
        let mut c = Children::default();
        for n in 0..20 {
            c.insert(slot(n), id(usize::try_from(n).unwrap_or(0)));
        }
        for n in (0..20).step_by(2) {
            assert!(c.remove(&slot(n)).is_some());
        }
        let left: Vec<usize> = c.values().map(|i| i.slot).collect();
        assert_eq!(left, (1..20).step_by(2).collect::<Vec<_>>());
        assert_eq!(c.get(&slot(5)), Some(&id(5)));
        c.reorder(&[id(19), id(1)]);
        let now: Vec<usize> = c.values().map(|i| i.slot).collect();
        assert_eq!(now.first(), Some(&19));
        assert_eq!(now.get(1), Some(&1));
        assert_eq!(now.len(), 10);
    }
}
