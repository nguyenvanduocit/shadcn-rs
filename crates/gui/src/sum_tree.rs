//! A persistent B+ tree whose nodes carry the summary of what is below them
//! (as Zed's `sum_tree`, `references/zed/crates/sum_tree`): a clone is O(1)
//! and shares every node; an edit copies only the path it walks (O(log n)).
//! Every node keeps how many items are below it, so indexing is O(log n);
//! an item's own summary (a row's height) adds a second dimension to seek by
//! (offset to row in O(log n), ADR-0014).
//!
//! Nodes are behind `Arc`, so a snapshot of a million rows goes to a worker
//! (sort, filter) without a copy. The B-tree keeps its invariants on every
//! edit: all leaves at one depth, and every node but the root at least half
//! full, so the height stays `log₁₆ n` (5 for a million rows).

use std::fmt;
use std::sync::Arc;

#[cfg(test)]
crate::engine::engine_local! {
    /// Entries (items, children) the tree looked at on this thread: what a
    /// test reads to tell O(log n) from a pass over every item, allocating
    /// or not. Only in tests.
    [Shared("a test's count of work done, read as a difference; no frame reads it")]
    static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Note `n` entries looked at ([`visits`]); nothing outside tests.
#[inline]
fn visit(n: usize) {
    #[cfg(test)]
    VISITS.with(|v| v.set(v.get().saturating_add(u64::try_from(n).unwrap_or(u64::MAX))));
    let _ = n;
}

/// Entries the trees looked at on this thread so far.
#[cfg(test)]
pub(crate) fn visits() -> u64 {
    VISITS.with(std::cell::Cell::get)
}

/// Most items in a leaf, children in an inner node.
const MAX: usize = 32;
/// Fewest, for every node but the root.
const MIN: usize = MAX / 2;

/// What a run of items adds up to (a total height, a count of a kind).
pub trait Summary: Clone + Default {
    fn add(&mut self, other: &Self);
}

impl Summary for () {
    fn add(&mut self, _: &Self) {}
}

/// An item of the tree and what it adds to the summary.
pub trait Item: Clone {
    type Summary: Summary;
    fn summary(&self) -> Self::Summary;
}

/// A node's totals: how many items, and their summary.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Totals<S> {
    pub count: usize,
    pub summary: S,
}

impl<S: Summary> Totals<S> {
    fn add(&mut self, other: &Self) {
        self.count = self.count.saturating_add(other.count);
        self.summary.add(&other.summary);
    }
}

#[derive(Clone)]
enum Node<T: Item> {
    Leaf {
        items: Vec<T>,
        totals: Totals<T::Summary>,
    },
    Inner {
        children: Vec<Arc<Node<T>>>,
        totals: Totals<T::Summary>,
    },
}

impl<T: Item> Node<T> {
    fn leaf(items: Vec<T>) -> Self {
        visit(items.len());
        let mut totals = Totals::default();
        for item in &items {
            totals.add(&Totals {
                count: 1,
                summary: item.summary(),
            });
        }
        Self::Leaf { items, totals }
    }

    fn inner(children: Vec<Arc<Self>>) -> Self {
        visit(children.len());
        let mut totals = Totals::default();
        for child in &children {
            totals.add(child.totals());
        }
        Self::Inner { children, totals }
    }

    fn totals(&self) -> &Totals<T::Summary> {
        match self {
            Self::Leaf { totals, .. } | Self::Inner { totals, .. } => totals,
        }
    }

    fn len(&self) -> usize {
        self.totals().count
    }

    /// Items in a leaf, children in an inner node.
    fn entries(&self) -> usize {
        match self {
            Self::Leaf { items, .. } => items.len(),
            Self::Inner { children, .. } => children.len(),
        }
    }

    fn recount(&mut self) {
        *self = match std::mem::replace(
            self,
            Self::Leaf {
                items: Vec::new(),
                totals: Totals::default(),
            },
        ) {
            Self::Leaf { items, .. } => Self::leaf(items),
            Self::Inner { children, .. } => Self::inner(children),
        };
    }

    /// The child holding item `i`, and `i` within it.
    fn child_of(children: &[Arc<Self>], mut i: usize) -> (usize, usize) {
        let last = children.len().saturating_sub(1);
        for (c, child) in children.iter().enumerate() {
            visit(1);
            let len = child.len();
            if i < len || c == last {
                return (c, i);
            }
            i = i.saturating_sub(len);
        }
        (0, i)
    }

    /// Insert `item` at `i`; the right half if this node overflowed.
    fn insert(node: &mut Arc<Self>, i: usize, item: T) -> Option<Arc<Self>> {
        let n = Arc::make_mut(node);
        match n {
            Self::Leaf { items, .. } => {
                items.insert(i.min(items.len()), item);
                let right = (items.len() > MAX).then(|| items.split_off(items.len().div_ceil(2)));
                n.recount();
                right.map(|r| Arc::new(Self::leaf(r)))
            }
            Self::Inner { children, .. } => {
                let (c, at) = Self::child_of(children, i);
                let split = children.get_mut(c).and_then(|child| Self::insert(child, at, item));
                if let Some(split) = split {
                    children.insert(c.saturating_add(1), split);
                }
                let right = (children.len() > MAX).then(|| children.split_off(children.len().div_ceil(2)));
                n.recount();
                right.map(|r| Arc::new(Self::inner(r)))
            }
        }
    }

    /// Remove item `i`; it, if there was one.
    fn remove(node: &mut Arc<Self>, i: usize) -> Option<T> {
        let n = Arc::make_mut(node);
        let removed = match n {
            Self::Leaf { items, .. } => (i < items.len()).then(|| items.remove(i)),
            Self::Inner { children, .. } => {
                let (c, at) = Self::child_of(children, i);
                let removed = children.get_mut(c).and_then(|child| Self::remove(child, at));
                if children.get(c).is_some_and(|child| child.entries() < MIN) {
                    Self::fix_underflow(children, c);
                }
                removed
            }
        };
        n.recount();
        removed
    }

    /// Child `c` has fewer than `MIN` entries: merge it with a neighbour if
    /// both fit in one node, else share the neighbour's evenly.
    fn fix_underflow(children: &mut Vec<Arc<Self>>, c: usize) {
        if children.len() < 2 {
            return;
        }
        let left = if c.saturating_add(1) < children.len() {
            c
        } else {
            c.saturating_sub(1)
        };
        let right = left.saturating_add(1);
        let (head, tail) = children.split_at_mut(right);
        let (Some(l), Some(r)) = (head.last_mut(), tail.first_mut()) else {
            return;
        };
        let (l, r) = (Arc::make_mut(l), Arc::make_mut(r));
        let total = l.entries().saturating_add(r.entries());
        match (l, r) {
            (Self::Leaf { items: a, .. }, Self::Leaf { items: b, .. }) => {
                a.append(b);
                if total > MAX {
                    *b = a.split_off(total.div_ceil(2));
                }
            }
            (Self::Inner { children: a, .. }, Self::Inner { children: b, .. }) => {
                a.append(b);
                if total > MAX {
                    *b = a.split_off(total.div_ceil(2));
                }
            }
            // All leaves are at one depth: siblings are of one kind.
            _ => {}
        }
        if total > MAX {
            for node in [left, right] {
                if let Some(n) = children.get_mut(node) {
                    Arc::make_mut(n).recount();
                }
            }
        } else {
            children.remove(right);
            if let Some(n) = children.get_mut(left) {
                Arc::make_mut(n).recount();
            }
        }
    }

    fn get(&self, i: usize) -> Option<&T> {
        match self {
            Self::Leaf { items, .. } => items.get(i),
            Self::Inner { children, .. } => {
                let (c, at) = Self::child_of(children, i);
                children.get(c)?.get(at)
            }
        }
    }

    fn update<R>(node: &mut Arc<Self>, i: usize, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        let n = Arc::make_mut(node);
        let result = match n {
            Self::Leaf { items, .. } => items.get_mut(i).map(f),
            Self::Inner { children, .. } => {
                let (c, at) = Self::child_of(children, i);
                children.get_mut(c).and_then(|child| Self::update(child, at, f))
            }
        };
        n.recount();
        result
    }
}

/// Group `nodes` into parents of between `MIN` and `MAX` children (one
/// parent may have fewer when there are fewer than `MIN` in all).
fn parents<T: Item>(nodes: Vec<Arc<Node<T>>>) -> Vec<Arc<Node<T>>> {
    let groups = nodes.len().div_ceil(MAX).max(1);
    let mut out = Vec::with_capacity(groups);
    let mut rest = nodes.into_iter();
    for g in 0..groups {
        let take = even_share(groups, g, rest.len());
        out.push(Arc::new(Node::inner(rest.by_ref().take(take).collect())));
    }
    out
}

/// How many of `left` go to group `g` of `groups` when they are spread evenly.
fn even_share(groups: usize, g: usize, left: usize) -> usize {
    let remaining = groups.saturating_sub(g).max(1);
    left.div_ceil(remaining)
}

/// A persistent sequence with summaries. See the module documentation.
pub struct SumTree<T: Item>(Arc<Node<T>>);

impl<T: Item> Clone for SumTree<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Item> Default for SumTree<T> {
    fn default() -> Self {
        Self(Arc::new(Node::leaf(Vec::new())))
    }
}

impl<T: Item + fmt::Debug> fmt::Debug for SumTree<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<T: Item + PartialEq> PartialEq for SumTree<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || (self.len() == other.len() && self.iter().eq(other.iter()))
    }
}

impl<T: Item> FromIterator<T> for SumTree<T> {
    /// Built bottom up in O(n), every node as full as an even spread allows.
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let items: Vec<T> = iter.into_iter().collect();
        if items.len() <= MAX {
            return Self(Arc::new(Node::leaf(items)));
        }
        let leaves = items.len().div_ceil(MAX);
        let mut level = Vec::with_capacity(leaves);
        let mut rest = items.into_iter();
        for g in 0..leaves {
            let take = even_share(leaves, g, rest.len());
            level.push(Arc::new(Node::leaf(rest.by_ref().take(take).collect())));
        }
        while level.len() > 1 {
            level = parents(level);
        }
        level.pop().map_or_else(Self::default, Self)
    }
}

impl<T: Item> SumTree<T> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The count and summary of every item.
    #[must_use]
    pub fn totals(&self) -> &Totals<T::Summary> {
        self.0.totals()
    }

    /// Whether `self` and `other` are the same snapshot (no item compared).
    #[must_use]
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    #[must_use]
    pub fn get(&self, i: usize) -> Option<&T> {
        self.0.get(i)
    }

    /// Insert `item` before item `i` (at the end past it).
    pub fn insert(&mut self, i: usize, item: T) {
        if let Some(right) = Node::insert(&mut self.0, i, item) {
            let left = std::mem::take(self);
            self.0 = Arc::new(Node::inner(vec![left.0, right]));
        }
    }

    pub fn push(&mut self, item: T) {
        self.insert(self.len(), item);
    }

    /// Remove item `i`.
    pub fn remove(&mut self, i: usize) -> Option<T> {
        let removed = Node::remove(&mut self.0, i);
        // A root with one child gives way to it.
        while let Node::Inner { children, .. } = &*self.0
            && children.len() == 1
            && let Some(only) = children.first()
        {
            self.0 = Arc::clone(only);
        }
        removed
    }

    /// Change item `i` in place; `f`'s result, if there is an item `i`.
    pub fn update<R>(&mut self, i: usize, f: impl FnOnce(&mut T) -> R) -> Option<R> {
        if i >= self.len() {
            return None;
        }
        Node::update(&mut self.0, i, f)
    }

    /// Replace items `at..at + removed` with `inserted`. A splice touching
    /// most of the tree is rebuilt in O(n); a small one is O(k log n).
    pub fn splice(&mut self, at: usize, removed: usize, inserted: impl IntoIterator<Item = T>) {
        let at = at.min(self.len());
        let removed = removed.min(self.len().saturating_sub(at));
        let inserted: Vec<T> = inserted.into_iter().collect();
        let edits = removed.saturating_add(inserted.len());
        if edits.saturating_mul(4) > self.len().max(64) {
            let tail = self
                .iter()
                .skip(at.saturating_add(removed))
                .cloned()
                .collect::<Vec<_>>();
            let rebuilt: Self = self.iter().take(at).cloned().chain(inserted).chain(tail).collect();
            *self = rebuilt;
            return;
        }
        for _ in 0..removed {
            self.remove(at);
        }
        for (k, item) in inserted.into_iter().enumerate() {
            self.insert(at.saturating_add(k), item);
        }
    }

    /// The items in order.
    #[must_use]
    pub fn iter(&self) -> Iter<'_, T> {
        self.iter_from(0)
    }

    /// The items from item `i` on, reaching `i` in O(log n).
    #[must_use]
    pub fn iter_from(&self, i: usize) -> Iter<'_, T> {
        let mut stack = Vec::new();
        let mut node: &Node<T> = &self.0;
        let mut at = i;
        loop {
            match node {
                Node::Leaf { items, .. } => {
                    return Iter {
                        stack,
                        leaf: items.get(at.min(items.len())..).unwrap_or_default().iter(),
                    };
                }
                Node::Inner { children, .. } => {
                    let (c, within) = Node::child_of(children, at);
                    stack.push(children.get(c.saturating_add(1)..).unwrap_or_default().iter());
                    let Some(child) = children.get(c) else {
                        return Iter { stack, leaf: [].iter() };
                    };
                    node = child;
                    at = within;
                }
            }
        }
    }

    /// The item where the running summary passes `target`, with the totals
    /// of the items before it: offset to row for heights, in O(log n).
    /// `past(before, item)` says whether `target` lies past the summary
    /// `before + item`, so the search keeps going.
    pub fn seek(&self, mut past: impl FnMut(&Totals<T::Summary>) -> bool) -> (usize, Totals<T::Summary>) {
        let mut before = Totals::default();
        let mut node: &Node<T> = &self.0;
        loop {
            match node {
                Node::Leaf { items, .. } => {
                    for item in items {
                        visit(1);
                        let mut next = before.clone();
                        next.add(&Totals {
                            count: 1,
                            summary: item.summary(),
                        });
                        if !past(&next) {
                            return (before.count, before);
                        }
                        before = next;
                    }
                    return (before.count, before);
                }
                Node::Inner { children, .. } => {
                    let mut chosen = None;
                    for child in children {
                        visit(1);
                        let mut next = before.clone();
                        next.add(child.totals());
                        if !past(&next) {
                            chosen = Some(child);
                            break;
                        }
                        before = next;
                    }
                    match chosen {
                        Some(child) => node = child,
                        None => return (before.count, before),
                    }
                }
            }
        }
    }

    /// The totals of items `0..i`, in O(log n).
    #[must_use]
    pub fn totals_before(&self, i: usize) -> Totals<T::Summary> {
        self.seek(|t| t.count <= i).1
    }

    /// Height of the tree: 0 for a single leaf.
    #[must_use]
    pub fn height(&self) -> usize {
        let mut h: usize = 0;
        let mut node: &Node<T> = &self.0;
        while let Node::Inner { children, .. } = node {
            let Some(first) = children.first() else { break };
            node = first;
            h = h.saturating_add(1);
        }
        h
    }

    /// Check every invariant (leaves at one depth, nodes at least half full
    /// but the root, totals that add up), for tests.
    #[must_use]
    pub fn is_valid(&self) -> bool
    where
        T::Summary: PartialEq,
    {
        fn check<T: Item>(node: &Node<T>, root: bool, depth: usize, leaf_depth: &mut Option<usize>) -> bool
        where
            T::Summary: PartialEq,
        {
            let full = root || node.entries() >= MIN;
            let fits = node.entries() <= MAX;
            let mut fresh = node.clone();
            fresh.recount();
            let sums = fresh.totals() == node.totals();
            let below = match node {
                Node::Leaf { .. } => *leaf_depth.get_or_insert(depth) == depth,
                Node::Inner { children, .. } => {
                    !children.is_empty()
                        && children
                            .iter()
                            .all(|c| check(c, false, depth.saturating_add(1), leaf_depth))
                }
            };
            full && fits && sums && below
        }
        check(&self.0, true, 0, &mut None)
    }
}

/// The items of a [`SumTree`] in order.
pub struct Iter<'a, T: Item> {
    stack: Vec<std::slice::Iter<'a, Arc<Node<T>>>>,
    leaf: std::slice::Iter<'a, T>,
}

impl<'a, T: Item> Iterator for Iter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<&'a T> {
        loop {
            if let Some(item) = self.leaf.next() {
                visit(1);
                return Some(item);
            }
            // Down the next subtree to its first leaf.
            let mut node: &Node<T> = loop {
                let top = self.stack.last_mut()?;
                if let Some(n) = top.next() {
                    break n;
                }
                self.stack.pop();
            };
            loop {
                match node {
                    Node::Leaf { items, .. } => {
                        self.leaf = items.iter();
                        break;
                    }
                    Node::Inner { children, .. } => {
                        let mut it = children.iter();
                        let Some(first) = it.next() else { break };
                        self.stack.push(it);
                        node = first;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of some height, summed.
    #[derive(Clone, Debug, PartialEq)]
    struct Row(u32);

    #[derive(Clone, Debug, Default, PartialEq)]
    struct Height(u64);

    impl Summary for Height {
        fn add(&mut self, other: &Self) {
            self.0 = self.0.saturating_add(other.0);
        }
    }

    impl Item for Row {
        type Summary = Height;
        fn summary(&self) -> Height {
            Height(u64::from(self.0))
        }
    }

    impl Item for u32 {
        type Summary = ();
        fn summary(&self) {}
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

    #[test]
    fn random_edits_match_a_vec_and_keep_the_tree_balanced() {
        for seed in 1..=60_u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let start = rng.below(300);
            let mut model: Vec<u32> = (0..u32::try_from(start).unwrap()).collect();
            let mut tree: SumTree<u32> = model.iter().copied().collect();
            let mut next = 10_000_u32;
            for step in 0..400 {
                let snapshot = tree.clone();
                let frozen = model.clone();
                match rng.below(5) {
                    0 | 1 => {
                        let at = rng.below(model.len().saturating_add(1));
                        model.insert(at, next);
                        tree.insert(at, next);
                        next = next.saturating_add(1);
                    }
                    2 if !model.is_empty() => {
                        let at = rng.below(model.len());
                        assert_eq!(tree.remove(at), Some(model.remove(at)));
                    }
                    3 if !model.is_empty() => {
                        let at = rng.below(model.len());
                        tree.update(at, |v| *v = v.saturating_add(1));
                        if let Some(v) = model.get_mut(at) {
                            *v = v.saturating_add(1);
                        }
                    }
                    _ => {
                        let at = rng.below(model.len().saturating_add(1));
                        let removed = rng.below(model.len().saturating_sub(at).saturating_add(1));
                        let inserted: Vec<u32> = (0..rng.below(40))
                            .map(|k| next.saturating_add(u32::try_from(k).unwrap()))
                            .collect();
                        next = next.saturating_add(40);
                        model.splice(at..at.saturating_add(removed), inserted.iter().copied());
                        tree.splice(at, removed, inserted);
                    }
                }
                assert!(tree.is_valid(), "seed {seed} step {step}: invariants");
                assert_eq!(tree.len(), model.len());
                assert!(
                    tree.iter().copied().eq(model.iter().copied()),
                    "seed {seed} step {step}"
                );
                assert!(
                    snapshot.iter().copied().eq(frozen.iter().copied()),
                    "an older snapshot is untouched"
                );
                let probe = rng.below(model.len().saturating_add(1));
                assert!(tree.iter_from(probe).copied().eq(model.iter().skip(probe).copied()));
            }
        }
    }

    #[test]
    fn a_million_rows_seek_by_height_in_log_time() {
        let n: u32 = 1_000_000;
        let tree: SumTree<Row> = (0..n).map(|i| Row(20 + i % 7)).collect();
        assert!(tree.is_valid());
        assert!(tree.height() <= 5, "height {}", tree.height());
        let total = tree.totals().summary.0;
        let expected: u64 = (0..n).map(|i| u64::from(20 + i % 7)).sum();
        assert_eq!(total, expected);
        // The row under offset y: the first row whose bottom is past y.
        for y in [0_u64, 1, 19, 20, 12_345_678, total.saturating_sub(1)] {
            let (row, before) = tree.seek(|t| t.summary.0 <= y);
            let top = before.summary.0;
            let height = u64::from(tree.get(row).map_or(0, |r| r.0));
            assert!(
                top <= y && y < top.saturating_add(height),
                "y {y}: row {row} spans {top}..+{height}"
            );
        }
        assert_eq!(tree.totals_before(3).summary.0, 20 + 21 + 22);
    }

    #[test]
    fn a_clone_is_a_snapshot_and_an_edit_copies_only_its_path() {
        let a: SumTree<u32> = (0..100_000).collect();
        let mut b = a.clone();
        assert!(a.ptr_eq(&b));
        b.update(50_000, |v| *v = 0);
        assert_eq!(a.get(50_000), Some(&50_000));
        assert_eq!(b.get(50_000), Some(&0));
        assert!(!a.ptr_eq(&b));
    }
}
