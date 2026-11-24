use std::rc::Rc;
use std::sync::Arc;

use crate::style::Style;
use crate::value::{Bound, Mounted};

use super::{Marks, need};

/// A closure bound to signals (ADR-0013's bindings by phase), run with a
/// tracker of its own: it runs again only when what it read changed, the
/// closure is another one, or the input `E` it runs over changed; its
/// change dirties its node alone.
pub(super) struct Tracked<K: ?Sized, T, E = ()> {
    node: crate::reactive::RenderNode,
    memo: Option<(Rc<K>, E, T)>,
}

impl<K: ?Sized, T: Clone, E: PartialEq + Clone> Tracked<K, T, E> {
    pub(super) fn new(marks: &Marks, u: u32) -> Self {
        Self {
            node: tracker(&resolve_mark(marks, u)),
            memo: None,
        }
    }

    fn get_over(&mut self, f: &Rc<K>, input: &E, run: impl FnOnce() -> T) -> T {
        if let Some((g, e, v)) = &self.memo
            && Rc::ptr_eq(g, f)
            && e == input
            && !self.node.is_marked()
        {
            return v.clone();
        }
        crate::component::count_binding();
        let v = self.node.track(run);
        self.memo = Some((Rc::clone(f), input.clone(), v.clone()));
        v
    }
}

impl<K: ?Sized, T: Clone> Tracked<K, T> {
    pub(super) fn get(&mut self, f: &Rc<K>, run: impl FnOnce() -> T) -> T {
        self.get_over(f, &(), run)
    }
}

impl<K: ?Sized, T, E> Drop for Tracked<K, T, E> {
    fn drop(&mut self) {
        // Even while unwinding (a binding panicked): a tracker left
        // subscribed would mark a node gone; disposing is safe then.
        self.node.dispose();
    }
}

/// One phase's subscription of one node (ADR-0013: a signal read while
/// patching or placing a node marks only that phase of that node). Made
/// by the first run that reads anything; until then the tree's probe runs
/// the phase and keeps nothing.
#[derive(Default)]
pub(super) struct Tracker(pub(super) Option<crate::reactive::RenderNode>);

impl Tracker {
    pub(super) fn run<R>(
        &mut self,
        probe: crate::reactive::RenderNode,
        marks: &Marks,
        u: u32,
        need: u8,
        f: impl FnOnce() -> R,
    ) -> R {
        if let Some(t) = self.0 {
            return t.track(f);
        }
        let out = probe.track(f);
        if probe.has_deps() {
            let marks = Rc::clone(marks);
            let t = crate::reactive::detached(|| {
                crate::reactive::RenderNode::new(move || {
                    if let Ok(mut m) = marks.try_borrow_mut() {
                        m.push((u, need));
                    }
                    crate::component::frame_due();
                })
            });
            t.take_deps(probe);
            self.0 = Some(t);
        }
        out
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        if let Some(t) = self.0.take() {
            t.dispose();
        }
    }
}

/// A node's bound style (ADR-0022): each bound setter's closure runs under
/// a tracker of its own, so a signal one of them read changing runs that
/// one again and replays the others from what they last gave (a binding
/// that reads `focus_visible` does not run on hover); a source
/// ([`crate::value::source`]) runs when what it reads wakes it, with no
/// tracker. The style is the fixed style with every write replayed over it
/// in the order it was set; it is kept, keyed by the writes and the fixed
/// style they replay over (a node restyled through [`Element::style_mut`](crate::element::Element::style_mut)
/// shares its writes with the node it was cloned from), until one of them
/// or a bound value changes.
pub(super) struct StyleBinding {
    /// The mark its trackers and sources share: node `u` needs its resolve.
    mark: crate::reactive::Mark,
    /// One per write, for those that are bound: what it read, and what it
    /// gave.
    runs: Vec<Option<BoundRun>>,
    /// What the replay itself read: made only while a parent's restyle
    /// replays (the one write that may read signals).
    replay: Option<crate::reactive::RenderNode>,
    /// The writes, the fixed style and the style replayed, as last resolved.
    memo: Option<(Rc<Vec<crate::value::Op>>, Style, Style)>,
}

/// One bound setter mounted on one node: the mount ([`Mounted`]) that
/// holds what it last gave on this node, typed, and the tracker its
/// closure runs under; a source's mount registers with what it reads and
/// holds no tracker. What one node's binding gave is never another's, and
/// a cold build's (its own tree, its own runs) never the live node's.
#[derive(Default)]
pub(super) struct BoundRun {
    /// Made at a closure's first run; none for a source.
    pub(super) node: Option<crate::reactive::RenderNode>,
    /// `None` before its first run, once its write is no longer bound, and
    /// after its closure panicked (it runs anew at the next resolve).
    mounted: Option<Box<dyn Mounted>>,
}

impl BoundRun {
    /// Whether it must run: its closure's tracker was marked, or its
    /// source woke it.
    fn due(&self) -> bool {
        self.node.is_some_and(crate::reactive::RenderNode::is_marked) || self.mounted.as_ref().is_some_and(|m| m.due())
    }

    /// Mount `def` here; whether its closure must run (mounted anew, or
    /// another closure of the same setter in place of the one that ran).
    /// A mount of another type is made anew; one of the same keeps its
    /// cell, so a node built again allocates nothing for it.
    fn bind(&mut self, def: &Rc<dyn Bound>) -> bool {
        if let Some(m) = &mut self.mounted {
            if m.is(def) {
                return false;
            }
            if m.rebind(def) {
                return true;
            }
        }
        self.mounted = Some(Rc::clone(def).mount());
        true
    }

    /// Run it, registered with what it reads to wake `mark`: a closure (or
    /// the one mount of a signal) under its tracker, made now if it has
    /// none, another source untracked; whether it gave
    /// something else. Taken out while it runs: a panic leaves no half-run
    /// mount behind.
    fn eval(&mut self, mark: &crate::reactive::Mark) -> bool {
        let Some(mut m) = self.mounted.take() else {
            return false;
        };
        m.prepare(mark);
        let changed = if m.tracked() {
            let node = *self.node.get_or_insert_with(|| tracker(mark));
            node.track(|| m.eval())
        } else {
            // A source in place of a closure: what the closure read wakes
            // nothing.
            if let Some(node) = self.node.take() {
                node.dispose();
            }
            crate::reactive::untracked(|| m.eval())
        };
        self.mounted = Some(m);
        changed
    }

    /// No longer bound: it reads nothing, waits on nothing and keeps no
    /// definition alive (nor what a closure that panicked read before it
    /// did).
    fn unbind(&mut self) {
        self.mounted = None;
        if let Some(node) = self.node
            && node.has_deps()
        {
            node.track(|| ());
        }
    }
}

impl Drop for BoundRun {
    fn drop(&mut self) {
        // Even while unwinding (its closure panicked): a tracker left
        // subscribed would mark a node gone; disposing is safe then.
        if let Some(node) = self.node {
            node.dispose();
        }
    }
}

/// A binding's tracker, owned by the binding that holds it (disposed when
/// it drops), not by a scope of its own: a scope per bound node would cost
/// an arena node and its list for nothing.
fn tracker(mark: &crate::reactive::Mark) -> crate::reactive::RenderNode {
    crate::reactive::detached(|| crate::reactive::RenderNode::marking(mark))
}

/// What a tracker of node `u`'s resolve runs when something it read
/// changed: it asks for that resolve. Whether it changed is the tracker's
/// own flag ([`crate::reactive::RenderNode::is_marked`]), so one mark serves
/// every tracker of the node.
pub(super) fn resolve_mark(marks: &Marks, u: u32) -> crate::reactive::Mark {
    let marks = Rc::clone(marks);
    Rc::new(move || {
        if let Ok(mut m) = marks.try_borrow_mut() {
            m.push((u, need::RESOLVE));
        }
        crate::component::frame_due();
    })
}

impl StyleBinding {
    pub(super) fn new(marks: &Marks, u: u32) -> Self {
        Self {
            mark: resolve_mark(marks, u),
            replay: None,
            runs: Vec::new(),
            memo: None,
        }
    }

    /// Every tracker it holds.
    pub(super) fn nodes(&self) -> impl Iterator<Item = crate::reactive::RenderNode> + '_ {
        self.runs.iter().flatten().filter_map(|r| r.node).chain(self.replay)
    }

    /// Its sources' registrations, taken out into `into`: its node is
    /// hidden, and they wake its parent in its place.
    pub(super) fn take_waits(&mut self, into: &mut Vec<crate::value::source::Wait>) {
        into.extend(
            self.runs
                .iter_mut()
                .flatten()
                .filter_map(|r| r.mounted.as_mut().and_then(|m| m.take_wait())),
        );
    }

    /// `base` with `ops` replayed over it: the closures whose reads changed,
    /// or that are new on this node, run again, alone. A closure that
    /// gave what it gave before changes nothing: with the writes and
    /// `base` the same, the style kept stands.
    pub(super) fn get(&mut self, ops: &Rc<Vec<crate::value::Op>>, base: &Style) -> Style {
        // Out while it runs: a closure that panics (a binding's, a
        // restyle's) leaves none, and the next resolve binds and replays
        // every write anew rather than trust what it half did.
        let mut memo = self.memo.take();
        let same_ops = memo.as_ref().is_some_and(|(o, ..)| Rc::ptr_eq(o, ops));
        if self.runs.len() < ops.len() {
            self.runs.resize_with(ops.len(), || None);
        }
        let mut changed = !same_ops;
        for (k, run) in self.runs.iter_mut().enumerate() {
            match ops.get(k).and_then(crate::value::Op::bound) {
                Some(def) => {
                    let run = run.get_or_insert_with(BoundRun::default);
                    // The same writes as last resolved, fully: each run
                    // holds a mount of its write's definition already.
                    let fresh = !(same_ops && run.mounted.is_some()) && run.bind(def);
                    if fresh || run.due() {
                        crate::component::count_binding();
                        changed |= run.eval(&self.mark);
                    }
                }
                // Past the writes, or of a write no longer bound.
                None => {
                    if let Some(run) = run {
                        run.unbind();
                    }
                }
            }
        }
        if !changed
            && !self.replay.is_some_and(crate::reactive::RenderNode::is_marked)
            && let Some((_, b, out)) = &memo
            && b == base
        {
            let out = out.clone();
            self.memo = memo;
            return out;
        }
        let runs = &self.runs;
        let replay = || {
            let mut s = base.clone();
            for (k, op) in ops.iter().enumerate() {
                let wrote = op.bound().is_some()
                    && runs
                        .get(k)
                        .and_then(Option::as_ref)
                        .and_then(|r| r.mounted.as_deref())
                        .is_some_and(|m| m.write(&mut s));
                if !wrote {
                    op.apply(&mut s);
                }
            }
            s
        };
        // Only a parent's restyle reads as it replays: a fixed write writes
        // a value, a bound one what its mount gave above.
        let out = if ops.iter().any(crate::value::Op::restyles) {
            let node = *self.replay.get_or_insert_with(|| tracker(&self.mark));
            node.track(replay)
        } else {
            if let Some(node) = self.replay.take() {
                node.dispose();
            }
            crate::reactive::untracked(replay)
        };
        match &mut memo {
            Some((o, b, v)) => {
                if !Rc::ptr_eq(o, ops) {
                    *o = Rc::clone(ops);
                }
                b.clone_from(base);
                v.clone_from(&out);
            }
            None => memo = Some((Rc::clone(ops), base.clone(), out.clone())),
        }
        self.memo = memo;
        out
    }
}

impl Drop for StyleBinding {
    fn drop(&mut self) {
        // Even while unwinding (a restyle panicked), as a run's tracker.
        if let Some(node) = self.replay.take() {
            node.dispose();
        }
    }
}

/// Its memo holds the text as shown: a resolve that finds it unchanged
/// shares it, copying nothing.
pub(super) type TextBinding = Tracked<dyn Fn() -> String, Arc<str>>;
pub(super) type DrawBinding = Tracked<dyn Fn() -> crate::chunk::Drawing, crate::chunk::Drawing>;
