//! Floating layers the engine knows (ADR-0024): every dialog, popover,
//! menu, select, tooltip or hover card is a [`Layer`] given to the node
//! that is its dismissable region ([`crate::Div::layer`]), inside a
//! [`crate::deferred`] subtree. The engine keeps them as a stack in the
//! order they opened, which is also the order they paint in (layout lays
//! deferred subtrees out in opening order, as portals appended to `<body>`
//! stack), and decides, for each input, which of them a press outside, a
//! focus outside, a scroll or Escape asks to close; the layer's owner
//! answers each request and may veto it ([`Dismiss`]).
//!
//! **Inside** is decided once, by ownership: a node is inside a layer when
//! it is in the layer's region, or in a layer owned by a node inside it
//! (the layer a node in the region opened, however deep), or, for a layer
//! that is not modal, in the subtree of the layer's own owner (its
//! trigger: a press on it toggles the layer rather than dismissing it and
//! opening it again), or of the node it names as what opened it
//! ([`Layer::opener`]). The owner of a deferred subtree is the node it was
//! declared in. Reka decides by DOM order, so a later unrelated layer is
//! "inside" every earlier one; here it is not.
//!
//! **Modal** is a fact of the engine: below the top modal layer's deferred
//! subtree nothing is hit, hovered, scrolled or pressed, the compositor
//! scrolls nothing (a barrier in its hit table), and assistive technology
//! is told the layer is modal and the rest is hidden.

use std::cell::RefCell;
use std::rc::Rc;

use crate::events::Modifiers;
use crate::focus::FocusScope;
use crate::geometry::NodePath;
use crate::layout::Frame;

/// What a pointer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerKind {
    Mouse,
    Pen,
    Touch,
}

/// Which button a press is of; a touch or a pen's tip is `Primary`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Other(u16),
}

/// A press outside a layer: where (window pixels), by what, which button,
/// which modifiers. A mouse or pen press counts as it goes down, on any
/// button; a touch counts once it is a tap ([`crate::gesture`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerOutside {
    pub x: f32,
    pub y: f32,
    pub kind: PointerKind,
    pub button: PointerButton,
    pub modifiers: Modifiers,
}

/// Why the engine asks a layer to close.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DismissReason {
    /// Escape, to the top layer that takes it, while no input method composes.
    Escape,
    /// A press outside it (see the module's "inside").
    PointerOutside(PointerOutside),
    /// Focus came to a node outside it.
    FocusOutside,
    /// A scroll container holding its owner scrolled.
    Scroll,
    /// Code asked, through its [`LayerHandle`].
    Programmatic,
}

/// A layer owner's answer to a dismiss request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dismiss {
    /// It closes (the owner wrote its open state).
    Close,
    /// It stays: the request is refused. The input that caused it goes on
    /// as it would have (a vetoed press outside still presses what it is on).
    Veto,
}

/// Which requests a layer takes; the others are never made to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DismissPolicy {
    pub escape: bool,
    pub pointer_outside: bool,
    pub focus_outside: bool,
    pub scroll: bool,
}

impl DismissPolicy {
    /// Escape, a press outside and focus outside: reka's `DismissableLayer`.
    pub const DEFAULT: Self = Self {
        escape: true,
        pointer_outside: true,
        focus_outside: true,
        scroll: false,
    };
    /// Nothing closes it but its owner.
    pub const NONE: Self = Self {
        escape: false,
        pointer_outside: false,
        focus_outside: false,
        scroll: false,
    };

    #[must_use]
    pub const fn escape(self, on: bool) -> Self {
        Self { escape: on, ..self }
    }

    #[must_use]
    pub const fn pointer_outside(self, on: bool) -> Self {
        Self {
            pointer_outside: on,
            ..self
        }
    }

    #[must_use]
    pub const fn focus_outside(self, on: bool) -> Self {
        Self {
            focus_outside: on,
            ..self
        }
    }

    #[must_use]
    pub const fn scroll(self, on: bool) -> Self {
        Self { scroll: on, ..self }
    }

    /// Whether a request for `reason` is made to a layer with this policy.
    #[must_use]
    pub const fn takes(self, reason: &DismissReason) -> bool {
        match reason {
            DismissReason::Escape => self.escape,
            DismissReason::PointerOutside(_) => self.pointer_outside,
            DismissReason::FocusOutside => self.focus_outside,
            DismissReason::Scroll => self.scroll,
            DismissReason::Programmatic => true,
        }
    }
}

impl Default for DismissPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What the owner of a layer is asked.
pub type DismissHandler = Rc<dyn Fn(&DismissReason) -> Dismiss>;

/// A floating layer as the engine keeps it: modal or not, the requests it
/// takes, who answers them, and the focus scope over its region.
#[derive(Clone, Default)]
pub struct Layer {
    pub(crate) modal: bool,
    pub(crate) policy: DismissPolicy,
    pub(crate) on_dismiss: Option<DismissHandler>,
    pub(crate) scope: Option<FocusScope>,
    pub(crate) handle: Option<LayerHandle>,
    /// Nothing in its region is a Tab stop ([`Self::tab_stops`]).
    pub(crate) untabbable: bool,
    /// A branch ([`Self::branch`]).
    pub(crate) branch: bool,
    /// What opened it by code ([`Self::opener`]).
    pub(crate) opener: Option<crate::focus::FocusId>,
}

impl Layer {
    /// A layer that is not modal, takes [`DismissPolicy::DEFAULT`], has no
    /// focus scope and no one answering (its requests go unanswered).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Modal: nothing below its deferred subtree takes the pointer, the
    /// wheel or assistive technology's attention.
    #[must_use]
    pub fn modal(self, modal: bool) -> Self {
        Self { modal, ..self }
    }

    #[must_use]
    pub fn dismiss(self, policy: DismissPolicy) -> Self {
        Self { policy, ..self }
    }

    /// Answer each dismiss request: close (and say so) or veto.
    #[must_use]
    pub fn on_dismiss(self, f: impl Fn(&DismissReason) -> Dismiss + 'static) -> Self {
        Self {
            on_dismiss: Some(Rc::new(f)),
            ..self
        }
    }

    /// Keep focus as `scope` says while the layer is open.
    #[must_use]
    pub fn focus_scope(self, scope: FocusScope) -> Self {
        Self {
            scope: Some(scope),
            ..self
        }
    }

    /// Whether the nodes in its region are Tab stops (default true). Not,
    /// each still takes focus from code and the pointer, and Tab goes on
    /// from one holding focus (reka's hover card sets `tabindex="-1"` on
    /// every one inside it).
    #[must_use]
    pub fn tab_stops(self, stops: bool) -> Self {
        Self {
            untabbable: !stops,
            ..self
        }
    }

    /// A branch of every other layer (reka's `DismissableLayerBranch`): a
    /// press or focus in its region is outside none of them (pressing a
    /// toast leaves an open popover open). It takes no request itself
    /// unless its policy says so.
    #[must_use]
    pub fn branch(self) -> Self {
        Self { branch: true, ..self }
    }

    /// For a layer that is not modal: the node holding focus `id` (what
    /// held focus as code opened it, a toggle outside its owner) is inside
    /// it, as its owner is, so a press or focus there asks nothing and its
    /// click toggles the layer once (reka's non-modal dialog,
    /// `getFocusedBeforeOpen`).
    #[must_use]
    pub fn opener(self, id: Option<crate::focus::FocusId>) -> Self {
        Self { opener: id, ..self }
    }

    /// Let code ask it to close through `handle` ([`LayerHandle::dismiss`]).
    #[must_use]
    pub fn handle(self, handle: LayerHandle) -> Self {
        Self {
            handle: Some(handle),
            ..self
        }
    }

    #[must_use]
    pub fn is_modal(&self) -> bool {
        self.modal
    }

    #[must_use]
    pub fn policy(&self) -> DismissPolicy {
        self.policy
    }

    #[must_use]
    pub fn scope(&self) -> Option<FocusScope> {
        self.scope
    }

    /// Ask its owner, if it takes `reason`; `None` when it does not, or no
    /// one answers.
    pub(crate) fn ask(&self, reason: &DismissReason) -> Option<Dismiss> {
        if !self.policy.takes(reason) {
            return None;
        }
        self.on_dismiss.as_ref().map(|f| f(reason))
    }
}

crate::engine::engine_local! {
    [Fork] static NEXT_HANDLE: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
    /// Who answers each handle's requests, as the last settled frame has it.
    [Fresh] static ANSWERS: RefCell<Vec<(LayerHandle, DismissHandler)>> = const { RefCell::new(Vec::new()) };
}

/// A layer's name for code: made once (a component's setup), given to its
/// [`Layer`], used to ask it to close.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerHandle(u64);

impl Default for LayerHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl LayerHandle {
    #[must_use]
    pub fn new() -> Self {
        Self(NEXT_HANDLE.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        }))
    }

    /// Whether a layer with this handle is open in the last frame.
    #[must_use]
    pub fn is_open(self) -> bool {
        ANSWERS.with(|a| a.borrow().iter().any(|(h, _)| *h == self))
    }

    /// Ask the layer to close ([`DismissReason::Programmatic`]); its
    /// owner's answer, `None` when no such layer is open.
    pub fn dismiss(self) -> Option<Dismiss> {
        let f = ANSWERS.with(|a| a.borrow().iter().find(|(h, _)| *h == self).map(|(_, f)| Rc::clone(f)))?;
        Some(f(&DismissReason::Programmatic))
    }
}

/// One layer of a frame: its region's node, the deferred subtree (frame
/// layer number) it is in, that subtree's owner, and what it is.
#[derive(Clone)]
pub(crate) struct Entry {
    pub(crate) node: usize,
    pub(crate) layer: usize,
    pub(crate) owner: Option<usize>,
    pub(crate) spec: Rc<Layer>,
}

/// The layers of one frame, bottom first (opening order, which is paint
/// order), and what the top modal one blocks.
#[derive(Clone, Default)]
pub(crate) struct Stack {
    pub(crate) entries: Vec<Entry>,
    /// Per frame layer number, the node that owns it (`None` for the main
    /// tree, and for a subtree whose owner is not in the frame).
    pub(crate) owners: Vec<Option<usize>>,
    /// Per frame layer number, the node its Tab stops follow: its owner,
    /// or the keyed node inside it it asks for.
    pub(crate) tab_owners: Vec<Option<usize>>,
    /// The first node that takes input: every node before it is below the
    /// top modal layer. 0 without one.
    pub(crate) floor: usize,
    /// The entry of the top modal layer.
    pub(crate) modal: Option<usize>,
    /// Which frame this was read from: its epoch and change count
    /// ([`Changes::end`](crate::layout::Changes::end)).
    read: Option<(u64, usize)>,
}

impl Stack {
    /// The layers of `frame`, kept from the last call while it is the same
    /// frame (its epoch and change log); read again into the same buffers,
    /// so a frame without layers costs no allocation.
    pub(crate) fn refresh(&mut self, frame: &Frame) {
        let key = (frame.epoch(), frame.changes.end());
        if self.read == Some(key) {
            return;
        }
        self.read = Some(key);
        self.owners.clear();
        self.owners.push(None);
        self.tab_owners.clear();
        self.tab_owners.push(None);
        for l in &frame.layers {
            let owner = frame
                .nodes
                .get(l.root)
                .and_then(|root| owner_of(frame, l.root, &root.path));
            self.owners.push(owner);
            let part = owner.zip(l.tab_after).and_then(|(o, k)| {
                let path = &frame.nodes.get(o)?.path.0;
                (o..l.root).find(|&i| {
                    frame.nodes.get(i).is_some_and(|n| {
                        n.path.0.starts_with(path) && n.path.0.last().and_then(|s| s.element_key()) == Some(k)
                    })
                })
            });
            self.tab_owners.push(part.or(owner));
        }
        self.entries.clear();
        for (i, b) in frame.behaviour.iter().enumerate() {
            let Some(spec) = &b.handlers.layer else { continue };
            let Some(n) = frame.nodes.get(i) else { continue };
            if !frame.shown(i) {
                continue;
            }
            self.entries.push(Entry {
                node: i,
                layer: n.layer,
                owner: self.owners.get(n.layer).copied().flatten(),
                spec: Rc::clone(spec),
            });
        }
        self.modal = self.entries.iter().rposition(|e| e.spec.modal);
        self.floor = self
            .modal
            .and_then(|m| self.entries.get(m))
            .map_or(0, |e| layer_start(frame, e.layer));
    }

    /// The layers of `frame`.
    pub(crate) fn of(frame: &Frame) -> Self {
        let mut s = Self::default();
        s.refresh(frame);
        s
    }

    /// Whether node `i` is below the top modal layer.
    pub(crate) fn blocked(&self, i: usize) -> bool {
        i < self.floor
    }

    /// The entries that take input (at or above the top modal), top first.
    pub(crate) fn live(&self) -> impl Iterator<Item = &Entry> {
        let from = self.modal.unwrap_or(0);
        self.entries.iter().skip(from).rev()
    }

    /// Whether node `h` is inside `e` (the module's rule), or in a branch.
    pub(crate) fn inside(&self, frame: &Frame, e: &Entry, h: usize) -> bool {
        self.reaches(frame, e, h, e.owner.filter(|_| !e.spec.modal))
            || (!e.spec.modal
                && e.spec
                    .opener
                    .and_then(|id| crate::focus::node_of(frame, id))
                    .is_some_and(|o| self.reaches(frame, e, h, Some(o))))
            || self
                .entries
                .iter()
                .any(|b| b.spec.branch && b.node != e.node && within(frame, h, b.node))
    }

    /// Whether node `i` is in the region of a layer whose nodes are no Tab
    /// stops ([`Layer::tab_stops`]).
    pub(crate) fn untabbable(&self, frame: &Frame, i: usize) -> bool {
        let layer = frame.nodes.get(i).map_or(0, |n| n.layer);
        self.entries
            .iter()
            .any(|e| e.spec.untabbable && e.layer == layer && within(frame, i, e.node))
    }

    /// Whether node `h` is in `e`'s focus scope: inside it, not counting its
    /// owner's subtree (a popover's trigger is inside it for dismissing,
    /// and no stop of its Tab ring).
    pub(crate) fn in_scope(&self, frame: &Frame, e: &Entry, h: usize) -> bool {
        self.reaches(frame, e, h, None)
    }

    fn reaches(&self, frame: &Frame, e: &Entry, h: usize, owner: Option<usize>) -> bool {
        let mut i = h;
        for _ in 0..=self.owners.len() {
            if within(frame, i, e.node) || owner.is_some_and(|o| within(frame, i, o)) {
                return true;
            }
            let l = frame.nodes.get(i).map_or(0, |n| n.layer);
            // Outside its region in its own subtree (a dialog's overlay),
            // or in the main tree outside both: outside.
            if l == 0 || l == e.layer {
                return false;
            }
            match self.owners.get(l).copied().flatten() {
                Some(o) => i = o,
                None => return false,
            }
        }
        false
    }

    /// The top entry with a focus scope that takes input: the active scope.
    pub(crate) fn top_scope(&self) -> Option<&Entry> {
        self.live().find(|e| e.spec.scope.is_some())
    }

    /// Who answers each handle, for [`LayerHandle::dismiss`].
    pub(crate) fn publish(&self) {
        ANSWERS.with(|a| {
            let mut a = a.borrow_mut();
            a.clear();
            for e in &self.entries {
                if let (Some(h), Some(f)) = (e.spec.handle, &e.spec.on_dismiss) {
                    a.push((h, Rc::clone(f)));
                }
            }
        });
    }
}

/// Whether node `i` is `ancestor` or below it, within one layer.
pub(crate) fn within(frame: &Frame, i: usize, ancestor: usize) -> bool {
    frame.ancestors(i).any(|a| a == ancestor)
}

/// The first node of frame layer `layer`.
pub(crate) fn layer_start(frame: &Frame, layer: usize) -> usize {
    match layer.checked_sub(1) {
        None => 0,
        Some(l) => frame.layers.get(l).map_or(frame.nodes.len(), |l| l.root),
    }
}

/// The node a deferred subtree whose root is node `root` at `path` was
/// declared in: the node before it in paint order with the longest path
/// that is a proper prefix of `path`.
fn owner_of(frame: &Frame, root: usize, path: &NodePath) -> Option<usize> {
    let segs = &path.0;
    let mut best: Option<(usize, usize)> = None;
    for (i, n) in frame.nodes.iter().enumerate().take(root) {
        let p = &n.path.0;
        if p.len() < segs.len() && segs.starts_with(p) && best.is_none_or(|(_, len)| p.len() > len) {
            best = Some((i, p.len()));
        }
    }
    best.map(|(i, _)| i)
}
