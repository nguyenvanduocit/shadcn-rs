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
/// that reads `focus_visible` does not run on hover). The style is the
/// fixed style with every write replayed over it in the order it was set;
/// it is kept, keyed by the writes and the fixed style they replay over (a
/// node restyled through [`Element::style_mut`](crate::element::Element::style_mut) shares its writes with the
/// node it was cloned from), until one of them or a closure's value changes.
pub(super) struct StyleBinding {
    /// The mark its trackers share: node `u` needs its resolve.
    mark: crate::reactive::Mark,
    /// One per write, for those that are bound: what its closure read, and
    /// what it gave.
    runs: Vec<Option<BoundRun>>,
    /// What the replay itself read (a parent's restyle may read signals).
    replay: BoundRun,
    /// The writes, the fixed style and the style replayed, as last resolved.
    memo: Option<(Rc<Vec<crate::value::Op>>, Style, Style)>,
}

/// One bound setter mounted on one node: its tracker, and the mount
/// ([`Mounted`]) that holds what its closure last gave on this node, typed.
/// What one node's binding gave is never another's, and a cold build's
/// (its own tree, its own runs) never the live node's.
pub(super) struct BoundRun {
    pub(super) node: crate::reactive::RenderNode,
    /// `None` before its first run, once its write is no longer bound, and
    /// after its closure panicked (it runs anew at the next resolve).
    mounted: Option<Box<dyn Mounted>>,
}

impl BoundRun {
    pub(super) fn new(mark: &crate::reactive::Mark) -> Self {
        Self {
            node: tracker(mark),
            mounted: None,
        }
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

    /// Run its closure under its tracker; whether it gave something else.
    /// Taken out while it runs: a panic leaves no half-run mount behind.
    fn eval(&mut self) -> bool {
        let Some(mut m) = self.mounted.take() else {
            return false;
        };
        let changed = self.node.track(|| m.eval());
        self.mounted = Some(m);
        changed
    }

    /// No longer bound: it reads nothing and keeps no definition alive
    /// (nor what a closure that panicked read before it did).
    fn unbind(&mut self) {
        if self.mounted.take().is_some() || self.node.has_deps() {
            self.node.track(|| ());
        }
    }
}

impl Drop for BoundRun {
    fn drop(&mut self) {
        // Even while unwinding (its closure panicked): a tracker left
        // subscribed would mark a node gone; disposing is safe then.
        self.node.dispose();
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
        let mark = resolve_mark(marks, u);
        Self {
            replay: BoundRun::new(&mark),
            mark,
            runs: Vec::new(),
            memo: None,
        }
    }

    /// Every tracker it holds.
    pub(super) fn nodes(&self) -> impl Iterator<Item = crate::reactive::RenderNode> + '_ {
        self.runs
            .iter()
            .flatten()
            .map(|r| r.node)
            .chain(std::iter::once(self.replay.node))
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
                    let run = run.get_or_insert_with(|| BoundRun::new(&self.mark));
                    // The same writes as last resolved, fully: each run
                    // holds a mount of its write's definition already.
                    let fresh = !(same_ops && run.mounted.is_some()) && run.bind(def);
                    if fresh || run.node.is_marked() {
                        crate::component::count_binding();
                        changed |= run.eval();
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
            && !self.replay.node.is_marked()
            && let Some((_, b, out)) = &memo
            && b == base
        {
            let out = out.clone();
            self.memo = memo;
            return out;
        }
        let runs = &self.runs;
        let out = self.replay.node.track(|| {
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
        });
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

/// Its memo holds the text as shown: a resolve that finds it unchanged
/// shares it, copying nothing.
pub(super) type TextBinding = Tracked<dyn Fn() -> String, Arc<str>>;
pub(super) type DrawBinding = Tracked<dyn Fn() -> crate::chunk::Drawing, crate::chunk::Drawing>;
