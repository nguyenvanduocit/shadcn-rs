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
    /// Its last run panicked in a turn: it gives what it gave before, and
    /// a cold build reuses that ([`HeldNode`]).
    held: bool,
}

impl<K: ?Sized, T: Clone, E: PartialEq + Clone> Tracked<K, T, E> {
    pub(super) fn new(marks: &Marks, u: u32) -> Self {
        Self {
            node: tracker(&resolve_mark(marks, u)),
            memo: None,
            held: false,
        }
    }

    /// What it gives: run again when what it read changed or the closure
    /// or its input is another. A run that panics in a turn (user code,
    /// ADR-0015, the turn rule) is to the arbiter, and changes nothing:
    /// it gives what it last gave (`None` before it ever gave: its node
    /// shows nothing of it), and runs again when what it read before the
    /// panic changes.
    fn get_over(&mut self, f: &Rc<K>, input: &E, run: impl FnOnce() -> T) -> Option<T> {
        if let Some((g, e, v)) = &self.memo
            && Rc::ptr_eq(g, f)
            && e == input
            && !self.node.is_marked()
        {
            return Some(v.clone());
        }
        crate::component::count_binding();
        let node = self.node;
        match crate::engine::attempt(|| node.track(run)) {
            Ok(v) => {
                self.held = false;
                self.memo = Some((Rc::clone(f), input.clone(), v.clone()));
                Some(v)
            }
            Err(failed) => {
                crate::component::contain_binding(failed);
                self.held = true;
                self.memo.as_ref().map(|(_, _, v)| v.clone())
            }
        }
    }

    /// What it holds over a run that panicked in a turn, for a cold build
    /// to reuse as it is ([`HeldNode`]): `None` when it holds nothing;
    /// `Some(None)` when it holds what it gave before it ever gave.
    fn held(&self) -> Option<Option<T>> {
        self.held.then(|| self.memo.as_ref().map(|(_, _, v)| v.clone()))
    }
}

impl<K: ?Sized, T: Clone> Tracked<K, T> {
    pub(super) fn get(&mut self, f: &Rc<K>, run: impl FnOnce() -> T) -> Option<T> {
        self.get_over(f, &(), run)
    }
}

impl<K: ?Sized, T, E> Drop for Tracked<K, T, E> {
    fn drop(&mut self) {
        // Even while unwinding (a binding panicked): a tracker left
        // subscribed would mark a node gone; disposing is safe then.
        self.node.release(crate::engine::LiveDrop::in_drop());
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
            t.release(crate::engine::LiveDrop::in_drop());
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
    /// after its closure panicked outside a turn (it runs anew at the next
    /// resolve).
    mounted: Option<Box<dyn Mounted>>,
    /// Its closure's last run panicked in a turn: the mount gives what it
    /// gave before, and a cold build reuses the style ([`HeldNode`]).
    held: bool,
}

/// One bound write a style binding holds over a run that panicked, by its
/// place among the node's writes: what it last gave (`None`: it never
/// gave, so it writes nothing), for a cold build to write in its place.
pub(super) type HeldWrite = (usize, Option<Box<dyn crate::value::Mounted>>);

/// What one node's bindings hold over runs that panicked in turns
/// (ADR-0015, rule 12): what a cold build of the frame reuses as the live
/// node holds it, never running those closures again, so a hold never
/// switches the check off and everything else is checked. A style holds
/// per setter: its other bound writes run fresh in the cold build, so a
/// held width hides no stale height on the same node. Each field is
/// empty or `None` when that binding holds nothing.
#[derive(Default)]
pub(super) struct HeldNode {
    /// The node's held bound writes.
    pub(super) style: Vec<HeldWrite>,
    /// Each span's held bound writes, by its place.
    pub(super) spans: Vec<(usize, Vec<HeldWrite>)>,
    /// The bound text (`None` inside: it never gave one).
    pub(super) text: Option<Option<Arc<str>>>,
    /// The drawing (`None` inside: it never drew).
    pub(super) draw: Option<Option<crate::chunk::Drawing>>,
}

impl HeldNode {
    /// What a node's bindings hold; `None` when none holds anything.
    pub(super) fn of(
        style: Option<&StyleBinding>,
        spans: &[Option<StyleBinding>],
        text: Option<&TextBinding>,
        draw: Option<&DrawBinding>,
    ) -> Option<Self> {
        let held = Self {
            style: style.map(StyleBinding::held).unwrap_or_default(),
            spans: spans
                .iter()
                .enumerate()
                .filter_map(|(k, b)| {
                    let h = b.as_ref().map(StyleBinding::held).unwrap_or_default();
                    (!h.is_empty()).then_some((k, h))
                })
                .collect(),
            text: text.and_then(Tracked::held),
            draw: draw.and_then(Tracked::held),
        };
        (!held.style.is_empty() || !held.spans.is_empty() || held.text.is_some() || held.draw.is_some()).then_some(held)
    }
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
    ///
    /// A closure that panics in a turn (user code, ADR-0015, the turn
    /// rule) is to the arbiter and changes nothing: the mount keeps what it
    /// last gave (the node's style as it was), and runs again when what it
    /// read before the panic changes. Outside a turn the panic goes on,
    /// the mount left out.
    fn eval(&mut self, mark: &crate::reactive::Mark) -> bool {
        let Some(mut m) = self.mounted.take() else {
            return false;
        };
        m.prepare(mark);
        let node = &mut self.node;
        let ran = crate::engine::attempt(|| {
            if m.tracked() {
                let node = *node.get_or_insert_with(|| tracker(mark));
                node.track(|| m.eval())
            } else {
                // A source in place of a closure: what the closure read
                // wakes nothing.
                if let Some(node) = node.take() {
                    node.dispose();
                }
                crate::reactive::untracked(|| m.eval())
            }
        });
        let changed = match ran {
            Ok(changed) => {
                self.held = false;
                changed
            }
            Err(failed) => {
                crate::component::contain_binding(failed);
                self.held = true;
                false
            }
        };
        self.mounted = Some(m);
        changed
    }

    /// No longer bound: it reads nothing, waits on nothing and keeps no
    /// definition alive (nor what a closure that panicked read before it
    /// did).
    fn unbind(&mut self) {
        self.mounted = None;
        self.held = false;
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
            node.release(crate::engine::LiveDrop::in_drop());
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

    /// Each bound write that holds what it gave over a run that panicked,
    /// by its place ([`HeldNode`]): the rest are not held.
    fn held(&self) -> Vec<HeldWrite> {
        self.runs
            .iter()
            .enumerate()
            .filter_map(|(k, r)| {
                let r = r.as_ref().filter(|r| r.held)?;
                Some((k, r.mounted.as_ref().map(|m| m.snapshot())))
            })
            .collect()
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
    ///
    /// `held`: in a cold build, the writes the live node holds
    /// ([`HeldNode`]), each written as it last gave in its place, its
    /// closure never run; every other write runs fresh. Live, none.
    pub(super) fn get(&mut self, ops: &Rc<Vec<crate::value::Op>>, base: &Style, held: &[HeldWrite]) -> Style {
        // Out while it runs: a closure that panics (a binding's, a
        // restyle's) leaves none, and the next resolve binds and replays
        // every write anew rather than trust what it half did.
        let mut memo = self.memo.take();
        let same_ops = memo.as_ref().is_some_and(|(o, ..)| Rc::ptr_eq(o, ops));
        if self.runs.len() < ops.len() {
            self.runs.resize_with(ops.len(), || None);
        }
        let mut changed = !same_ops;
        let held_at = |k: usize| held.iter().find(|(at, _)| *at == k).map(|(_, m)| m.as_deref());
        for (k, run) in self.runs.iter_mut().enumerate() {
            match ops.get(k).and_then(crate::value::Op::bound) {
                // Held: written from what it last gave, below.
                Some(_) if held_at(k).is_some() => changed = true,
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
        // A bound write writes only what its mount gave this pass: its
        // closure ran once above, and is never run again here. One that has
        // given nothing (its first run panicked in a turn, held) is absent:
        // the property keeps what the writes before it left (ADR-0015, a
        // contained getter runs once per pass).
        let replay = || {
            let mut s = base.clone();
            for (k, op) in ops.iter().enumerate() {
                if op.bound().is_some() {
                    let mount = match held_at(k) {
                        Some(held) => held,
                        None => runs.get(k).and_then(Option::as_ref).and_then(|r| r.mounted.as_deref()),
                    };
                    if let Some(m) = mount {
                        m.write(&mut s);
                    }
                } else {
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
            node.release(crate::engine::LiveDrop::in_drop());
        }
    }
}

/// Its memo holds the text as shown: a resolve that finds it unchanged
/// shares it, copying nothing.
pub(super) type TextBinding = Tracked<dyn Fn() -> String, Arc<str>>;
pub(super) type DrawBinding = Tracked<dyn Fn() -> crate::chunk::Drawing, crate::chunk::Drawing>;
