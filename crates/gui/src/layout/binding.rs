use std::rc::Rc;
use std::sync::Arc;

use crate::style::Style;

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

/// One closure's subscription and the value it last gave.
pub(super) struct BoundRun {
    pub(super) node: crate::reactive::RenderNode,
    slot: Option<Box<dyn std::any::Any>>,
}

impl BoundRun {
    pub(super) fn new(mark: &crate::reactive::Mark) -> Self {
        Self {
            node: tracker(mark),
            slot: None,
        }
    }
}

impl Drop for BoundRun {
    fn drop(&mut self) {
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

    /// `base` with `ops` replayed over it: the closures whose reads changed
    /// (all of them for other writes) run again, alone.
    pub(super) fn get(&mut self, ops: &Rc<Vec<crate::value::Op>>, base: &Style) -> Style {
        let same_ops = self.memo.as_ref().is_some_and(|(o, ..)| Rc::ptr_eq(o, ops));
        if !same_ops {
            // Trackers past the writes, or of a write no longer bound,
            // keep reading nothing.
            for (k, run) in self.runs.iter_mut().enumerate() {
                if let Some(r) = run
                    && ops.get(k).and_then(crate::value::Op::bound).is_none()
                {
                    r.node.track(|| ());
                }
            }
        }
        let mut ran = false;
        for (k, op) in ops.iter().enumerate() {
            let Some(b) = op.bound() else { continue };
            if self.runs.len() <= k {
                self.runs.resize_with(k.saturating_add(1), || None);
            }
            let Some(slot) = self.runs.get_mut(k) else { continue };
            let run = slot.get_or_insert_with(|| BoundRun::new(&self.mark));
            if !same_ops || run.node.is_marked() || run.slot.is_none() {
                crate::component::count_binding();
                let held = &mut run.slot;
                run.node.track(|| b.eval(held));
                ran = true;
            }
        }
        if !ran
            && !self.replay.node.is_marked()
            && let Some((_, b, out)) = &self.memo
            && same_ops
            && b == base
        {
            return out.clone();
        }
        let runs = &self.runs;
        let out = self.replay.node.track(|| {
            let mut s = base.clone();
            for (k, op) in ops.iter().enumerate() {
                match (
                    op.bound(),
                    runs.get(k).and_then(Option::as_ref).and_then(|r| r.slot.as_deref()),
                ) {
                    (Some(b), Some(v)) => b.write(v, &mut s),
                    _ => op.apply(&mut s),
                }
            }
            s
        });
        match &mut self.memo {
            Some((o, b, v)) => {
                if !Rc::ptr_eq(o, ops) {
                    *o = Rc::clone(ops);
                }
                b.clone_from(base);
                v.clone_from(&out);
            }
            None => self.memo = Some((Rc::clone(ops), base.clone(), out.clone())),
        }
        out
    }
}

/// Its memo holds the text as shown: a resolve that finds it unchanged
/// shares it, copying nothing.
pub(super) type TextBinding = Tracked<dyn Fn() -> String, Arc<str>>;
pub(super) type DrawBinding = Tracked<dyn Fn() -> crate::chunk::Drawing, crate::chunk::Drawing>;
