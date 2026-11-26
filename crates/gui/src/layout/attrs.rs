//! A node's bound attributes: its accessibility binding phase (ADR-0026).
//!
//! Every attribute setter of a node ([`crate::Div::aria_selected`],
//! [`crate::Div::disabled`], [`crate::Div::role`]…) takes a value or a
//! closure, as a style setter does ([`crate::value::IntoValue`]). A value is
//! written into the node's [`Attrs`] as it is described. A closure is kept
//! on the node ([`AttrOp`]) and mounted here, once per node, where the node
//! resolves: it runs under a tracker of its own ([`Tracked`], the text and
//! drawing bindings' mechanism) that marks only this node's resolve, so a
//! signal it reads changing runs that closure alone, on this node alone.
//! What it gave is kept by the mount, typed, never by the shared
//! definition: a node mounted twice, or built again by a cold check, keeps
//! its own.
//!
//! The node's attributes are its fixed ones with every bound one written
//! over them, in the order they were set. They are written anew only when
//! one of them changed; then the node's behaviour is made anew, and the
//! frame logs the node as changed but not repainted, which is all the
//! accessibility tree sends ([`crate::access`]). An attribute its subtree
//! inherits (`disabled`, `inert`, `dormant`, `pass_through_all`) resolves
//! the subtree as a changed parent does.

use std::any::Any;
use std::rc::Rc;

use crate::element::{AttrOp, Attrs};

use super::Marks;
use super::binding::{Tracked, resolve_mark};

/// One attribute setter given a closure: what it reads, and how what it
/// gives is written into the node's attributes. Shared by every mount of
/// the node that holds it; it keeps no state of a mount.
pub(crate) trait AttrBound: Any {
    /// A mount of it on one node, its tracker running `mark`.
    fn mount(self: Rc<Self>, mark: &crate::reactive::Mark) -> Box<dyn AttrRun>;

    /// Its closure, an `Rc<dyn Fn() -> T>`, for code that decides by what
    /// it gives as it resolves ([`crate::element::Common::role_given`]).
    fn read_any(&self) -> &dyn Any;
}

/// What `read` gives, written by `write`.
struct Def<T> {
    read: Rc<dyn Fn() -> T>,
    write: fn(&mut Attrs, T),
}

/// The definition of an attribute bound to `read`, written by `write`.
pub(crate) fn attr_def<T: Clone + PartialEq + 'static>(
    read: Rc<dyn Fn() -> T>,
    write: fn(&mut Attrs, T),
) -> Rc<dyn AttrBound> {
    Rc::new(Def { read, write })
}

impl<T: Clone + PartialEq + 'static> AttrBound for Def<T> {
    fn mount(self: Rc<Self>, mark: &crate::reactive::Mark) -> Box<dyn AttrRun> {
        Box::new(Run {
            tracked: Tracked::marking(mark),
            def: self,
        })
    }

    fn read_any(&self) -> &dyn Any {
        &self.read
    }
}

/// One bound attribute mounted on one node: its tracker, and what it last
/// gave, typed.
pub(crate) trait AttrRun {
    /// Bind it to `def` in place of the definition it has: whether it
    /// could (a definition of another type is mounted anew). Its closure
    /// runs again at the next [`Self::refresh`] when `def` is another one.
    fn rebind(&mut self, def: &Rc<dyn AttrBound>) -> bool;

    /// Run it again if what it read changed or its closure is another:
    /// whether it gives another value now.
    fn refresh(&mut self) -> bool;

    /// Write what it last gave; nothing before it ever gave.
    fn write(&self, attrs: &mut Attrs);

    /// What it holds over a run that panicked in a turn, for a cold build
    /// to write in its place: `None` when it holds nothing.
    fn held(&self) -> Option<HeldWrite>;

    /// Its tracker: what its closure read.
    fn tracker(&self) -> crate::reactive::RenderNode;
}

struct Run<T> {
    def: Rc<Def<T>>,
    tracked: Tracked<Def<T>, T>,
}

impl<T: Clone + PartialEq + 'static> AttrRun for Run<T> {
    fn rebind(&mut self, def: &Rc<dyn AttrBound>) -> bool {
        if std::ptr::addr_eq(Rc::as_ptr(def), Rc::as_ptr(&self.def)) {
            return true;
        }
        let def: Rc<dyn AttrBound> = Rc::clone(def);
        let any: Rc<dyn Any> = def;
        match any.downcast::<Def<T>>() {
            Ok(d) => {
                self.def = d;
                true
            }
            Err(_) => false,
        }
    }

    fn refresh(&mut self) -> bool {
        let def = &self.def;
        self.tracked.refresh(def, || (def.read)())
    }

    fn write(&self, attrs: &mut Attrs) {
        if let Some(v) = self.tracked.value() {
            (self.def.write)(attrs, v.clone());
        }
    }

    fn held(&self) -> Option<HeldWrite> {
        self.tracked.is_held().then(|| {
            let write = self.def.write;
            self.tracked.value().cloned().map(|v| {
                let w: Box<dyn Fn(&mut Attrs)> = Box::new(move |a: &mut Attrs| write(a, v.clone()));
                w
            })
        })
    }

    fn tracker(&self) -> crate::reactive::RenderNode {
        self.tracked.tracker()
    }
}

/// What a held attribute writes: `None` when it never gave, so it writes
/// nothing and the attribute keeps its fixed value.
pub(super) type HeldWrite = Option<Box<dyn Fn(&mut Attrs)>>;

/// One bound attribute a node holds over a run that panicked, by the
/// attribute ([`super::binding::HeldNode`]).
pub(super) type HeldAttr = (&'static str, HeldWrite);

/// A node's bound attributes, mounted: one run per [`AttrOp`], known by
/// its attribute (never by its place among them: a render that binds
/// another attribute where one was mounts it anew, with nothing of what
/// the other gave), and the attributes they last resolved to.
pub(super) struct AttrBinding {
    /// The mark its runs share: the node needs its resolve.
    mark: crate::reactive::Mark,
    /// In the order of the node's ops as last resolved.
    runs: Vec<(&'static str, Box<dyn AttrRun>)>,
    attrs: Attrs,
}

impl AttrBinding {
    pub(super) fn new(marks: &Marks, u: u32) -> Self {
        Self {
            mark: resolve_mark(marks, u),
            runs: Vec::new(),
            attrs: Attrs::default(),
        }
    }

    /// Resolve the node's attributes: `base` with each of `ops` written
    /// over it as its closure gives, each run again only when what it read
    /// changed or it is new here; whether they changed since the last
    /// resolve ([`Self::attrs`]). `same`: `base` and `ops` are those of
    /// the last resolve (the node's element is the one it was resolved
    /// from); a resolve that is not the same writes them anew.
    ///
    /// `held`: in a cold build, the attributes the live node holds over a
    /// run that panicked, each written as it last gave, its closure never
    /// run; every other one runs fresh. Live, none.
    pub(super) fn resolve(&mut self, base: &Attrs, ops: &[AttrOp], same: bool, held: &[HeldAttr]) -> bool {
        let mut changed = !same;
        let held_at = |name: &str| held.iter().find(|(at, _)| *at == name).map(|(_, w)| w);
        for (k, op) in ops.iter().enumerate() {
            // The run of this attribute, wherever it was: brought to `k`.
            // One of another attribute at `k` goes past the ops, and is
            // dropped below unless a later op is its attribute.
            if self.runs.get(k).is_none_or(|(name, _)| *name != op.name) {
                match self.runs.iter().skip(k).position(|(name, _)| *name == op.name) {
                    Some(at) => self.runs.swap(k, k.saturating_add(at)),
                    None => {
                        let run = (op.name, Rc::clone(&op.def).mount(&self.mark));
                        match self.runs.get_mut(k) {
                            Some(at) => {
                                let other = std::mem::replace(at, run);
                                self.runs.push(other);
                            }
                            None => self.runs.push(run),
                        }
                    }
                }
                changed = true;
            }
            if held_at(op.name).is_some() {
                changed = true;
                continue;
            }
            let Some((_, run)) = self.runs.get_mut(k) else { continue };
            if !run.rebind(&op.def) {
                // Of another value type: the same attribute, mounted anew.
                *run = Rc::clone(&op.def).mount(&self.mark);
                changed = true;
            }
            changed |= run.refresh();
        }
        // A run past the ops is no longer bound: dropped, it reads nothing.
        self.runs.truncate(ops.len());
        if changed {
            self.attrs.clone_from(base);
            for (name, run) in &self.runs {
                match held_at(name) {
                    Some(w) => {
                        if let Some(w) = w {
                            w(&mut self.attrs);
                        }
                    }
                    None => run.write(&mut self.attrs),
                }
            }
        }
        changed
    }

    /// The node's attributes as last resolved.
    pub(super) fn attrs(&self) -> &Attrs {
        &self.attrs
    }

    /// Each bound attribute that holds what it gave over a run that
    /// panicked ([`super::binding::HeldNode`]).
    pub(super) fn held(&self) -> Vec<HeldAttr> {
        self.runs
            .iter()
            .filter_map(|(name, r)| Some((*name, r.held()?)))
            .collect()
    }

    /// Every tracker it holds: what its closures read.
    pub(super) fn nodes(&self) -> impl Iterator<Item = crate::reactive::RenderNode> + '_ {
        self.runs.iter().map(|(_, r)| r.tracker())
    }
}
