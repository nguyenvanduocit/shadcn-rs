//! Keyboard focus: which focusable element receives keys, how the user
//! acts ([`Modality`]), and the focus scopes of open layers
//! ([`FocusScope`], kept by the engine's focus manager, ADR-0024).
//!
//! A component creates a [`FocusHandle`] once (it is part of the component's
//! state, like a signal) and attaches it to an element with `track_focus`.
//! Which handle holds focus is a signal, so reading it re-renders on change.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use crate::edit::TextEdit;
use crate::geometry::NodePath;
use crate::layer::{Entry, Layer, Stack, layer_start, within};
use crate::layout::Frame;
use crate::reactive::Signal;

crate::engine::engine_local! {
    [Fork] static NEXT: Cell<u64> = const { Cell::new(1) };
    /// The windows made, for each one's own path-derived ids.
    [Fork] static WINDOWS: Cell<u64> = const { Cell::new(0) };
    /// The window whose code runs now: what [`focused`], [`set_focus`] and
    /// a handle's `focus()` read and write. A window makes itself current
    /// as it takes input, lays out or runs a caller's code; one made last
    /// is current until then. With no window, one of the thread's own.
    [Fork] static CURRENT: RefCell<Option<Rc<FocusRoot>>> = const { RefCell::new(None) };
}

/// One window's focus: which of its nodes holds it, whether its ring shows,
/// the focus to hand on once laid out, and its focusable nodes as its last
/// layout left them. Each window has its own, so laying one out or moving
/// focus in it leaves another's as it was.
pub struct FocusRoot {
    /// Mixed into its path-derived ids ([`FocusHandle::for_path`]), so the
    /// same place in two windows is two nodes.
    salt: u64,
    focused: Signal<Option<FocusId>>,
    /// Whether the input modality was [`Modality::Keyboard`] when focus last
    /// moved, which is when `focus-visible` shows: written only by
    /// [`set_focus`], from [`MODALITY`], so the two never disagree.
    visible: Signal<bool>,
    /// Focus to hand on once laid out ([`hand_on`]).
    hand_on: Cell<Option<HandOn>>,
    focusables: Focusables,
}

impl FocusRoot {
    /// A window's, current from now on (until another window runs).
    #[must_use]
    pub fn new() -> Rc<Self> {
        let salt = WINDOWS.with(|w| {
            let n = w.get().wrapping_add(1);
            w.set(n);
            n
        });
        let me = Rc::new(Self {
            salt,
            focused: crate::reactive::detached(|| Signal::labeled("focus", None)),
            visible: crate::reactive::detached(|| Signal::labeled("focus-visible", false)),
            hand_on: Cell::new(None),
            focusables: Focusables::default(),
        });
        me.enter();
        me
    }

    /// Make it the current window's: its code runs now.
    pub fn enter(self: &Rc<Self>) {
        CURRENT.with_borrow_mut(|c| {
            if !c.as_ref().is_some_and(|c| Rc::ptr_eq(c, self)) {
                *c = Some(Rc::clone(self));
            }
        });
    }
}

impl Drop for FocusRoot {
    fn drop(&mut self) {
        self.focused.dispose_quietly();
        self.visible.dispose_quietly();
    }
}

impl fmt::Debug for FocusRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FocusRoot")
            .field("salt", &self.salt)
            .finish_non_exhaustive()
    }
}

/// The current window's focus (a clone is a count, not an allocation).
fn current() -> Rc<FocusRoot> {
    CURRENT.with_borrow_mut(|c| Rc::clone(c.get_or_insert_with(FocusRoot::new_detached)))
}

impl FocusRoot {
    /// The thread's own, for code that runs outside any window.
    fn new_detached() -> Rc<Self> {
        Rc::new(Self {
            salt: 0,
            focused: crate::reactive::detached(|| Signal::labeled("focus", None)),
            visible: crate::reactive::detached(|| Signal::labeled("focus-visible", false)),
            hand_on: Cell::new(None),
            focusables: Focusables::default(),
        })
    }
}

#[derive(Clone, Copy)]
struct HandOn {
    from: FocusId,
    into: crate::access::NodeRef,
    or: Option<FocusId>,
}

/// Focus held by `from`, which the next frame takes out of reach (its node
/// goes inert or away: a carousel's slide leaving the view), goes on to the
/// first Tab stop inside the node `into` names, else to `or`, so it is not
/// lost (reka's carousel `rescueFocus`). Applied as that frame settles,
/// only if `from` still holds focus and no node holds it any more.
pub fn hand_on(from: FocusId, into: crate::access::NodeRef, or: Option<FocusHandle>) {
    current().hand_on.set(Some(HandOn {
        from,
        into,
        or: or.map(FocusHandle::id),
    }));
}

/// Hand focus on as [`hand_on`] asked; whether it moved.
fn take_hand_on(frame: &Frame, stack: &Stack) -> bool {
    let Some(h) = current().hand_on.take() else {
        return false;
    };
    if focused_now() != Some(h.from) || node_of(frame, h.from).is_some() {
        return false;
    }
    let inside = (0..frame.nodes.len())
        .find(|&i| frame.behaviour(i).is_some_and(|b| b.semantics.node == Some(h.into)))
        .and_then(|o| (o..=subtree_end(frame, o)).find(|&i| tabbable(frame, stack, i)))
        .and_then(|i| focus_at(frame, i));
    let Some(to) = inside.or(h.or.filter(|&id| node_of(frame, id).is_some())) else {
        return false;
    };
    engine_focus(frame, Some(to));
    true
}

/// The identity of one focusable element.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusId(u64);

impl FocusId {
    /// It as an element key: no two ids issued share one, so what it
    /// focuses can be keyed by it among its siblings (a virtual list's
    /// rows).
    #[must_use]
    pub const fn key(self) -> crate::ElementKey {
        self.0
    }
}

/// A focus target that stays the same across frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusHandle {
    id: FocusId,
}

impl Default for FocusHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusHandle {
    #[must_use]
    pub fn new() -> Self {
        let id = NEXT.with(|n| {
            let id = n.get();
            n.set(id.wrapping_add(1));
            id
        });
        Self { id: FocusId(id) }
    }

    /// The handle of a node focusable by its place in the tree of the
    /// current window: the same place in another window is another node.
    #[must_use]
    pub fn for_path(path: &crate::geometry::NodePath) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        current().salt.hash(&mut h);
        path.hash(&mut h);
        // The top bit keeps these apart from counted handles.
        Self {
            id: FocusId(h.finish() | (1 << 63)),
        }
    }

    #[must_use]
    pub fn id(self) -> FocusId {
        self.id
    }

    /// Whether this handle holds focus (reactive).
    #[must_use]
    pub fn is_focused(self) -> bool {
        focused() == Some(self.id)
    }

    /// Give this handle focus; its ring shows when the user is at the
    /// keyboard ([`modality`]), as a script's `focus()` matches
    /// `:focus-visible` after a key.
    pub fn focus(self) {
        set_focus(Some(self.id), false);
    }

    /// Drop focus if this handle holds it.
    pub fn blur(self) {
        if self.is_focused() {
            set_focus(None, false);
        }
    }
}

/// The focused element, if any (reactive).
#[must_use]
pub fn focused() -> Option<FocusId> {
    current().focused.get()
}

/// Whether the current focus shows its ring: the user was at the keyboard
/// when it moved there (reactive).
#[must_use]
pub fn focus_visible() -> bool {
    current().visible.get()
}

/// Move focus to `id`. `by_keyboard`: the keyboard moved it, so the input
/// modality is [`Modality::Keyboard`] from now on; otherwise the modality
/// is left as it is. The ring shows when the modality is `Keyboard`: one
/// fact decides it, whoever moves focus.
pub fn set_focus(id: Option<FocusId>, by_keyboard: bool) {
    // Focus asked for a node laid out but not shown does nothing, as
    // `focus()` on an element that is not rendered does nothing in a
    // browser: what holds focus keeps it.
    if let Some(id) = id
        && current().focusables.hidden.borrow().contains(&id)
    {
        return;
    }
    if by_keyboard {
        set_modality(Modality::Keyboard);
    }
    let visible = id.is_some() && MODALITY.with(|m| m.get_untracked()) == Modality::Keyboard;
    let root = current();
    if root.focused.get_untracked() != id {
        root.focused.set(id);
    }
    if root.visible.get_untracked() != visible {
        root.visible.set(visible);
    }
}

/// How the user acts now: one fact of the engine (reka's
/// `useIsUsingKeyboard`). A key press makes it `Keyboard`; a pointer
/// pressing or moving makes it `Pointer`, before any handler of that
/// event runs. Focus the engine moves itself (a layer opening or closing,
/// a trap pulling focus back) shows its ring when it is `Keyboard`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Modality {
    Keyboard,
    #[default]
    Pointer,
}

/// A window's focusable nodes as its last layout left them: those it laid
/// out but does not show ([`Frame::shown`]), whose focus asked for is
/// refused, and where each shown one lies (its path), for what asks where
/// focus is before the next layout.
#[derive(Default)]
struct Focusables {
    hidden: RefCell<Vec<FocusId>>,
    paths: RefCell<Vec<(FocusId, NodePath)>>,
    /// The frame they were taken from: its epoch and change count.
    of: Cell<Option<(u64, usize)>>,
}

/// Take the current window's focusable nodes from `frame`, into the
/// buffers kept from the last. Layout calls it as it ends, so what runs
/// after it (a list's watcher, a hook after layout, the focus rules) asks
/// against the frame as it is now. A frame already taken (focus alone
/// moved since) is not read again.
pub(crate) fn publish(frame: &Frame) {
    let root = current();
    let f = &root.focusables;
    let key = (frame.epoch(), frame.changes.end());
    if f.of.get() == Some(key) {
        return;
    }
    f.of.set(Some(key));
    let mut hidden = f.hidden.borrow_mut();
    let mut paths = f.paths.borrow_mut();
    hidden.clear();
    paths.clear();
    for i in 0..frame.nodes.len() {
        let (Some(id), Some(n)) = (focus_at(frame, i), frame.nodes.get(i)) else {
            continue;
        };
        if frame.shown(i) {
            // Paths are shared: a clone is a count, not an allocation.
            paths.push((id, n.path.clone()));
        } else {
            hidden.push(id);
        }
    }
}

/// Where the node focused by `id` lies in the current window's last frame,
/// shown: for what keeps it built (a virtual list's row) before the next
/// layout, as focus moves.
pub(crate) fn path_of(id: FocusId) -> Option<NodePath> {
    let root = current();
    let paths = root.focusables.paths.borrow();
    paths.iter().find(|(i, _)| *i == id).map(|(_, p)| p.clone())
}

crate::engine::engine_local! {
    [Shared("a handle made live; the value is the reactive arena's, which forks")] static MODALITY: Signal<Modality> = crate::reactive::detached(|| Signal::labeled("modality", Modality::Pointer));
}

/// How the user acts now (reactive).
#[must_use]
pub fn modality() -> Modality {
    MODALITY.with(Signal::get)
}

/// Record how the user acts, written only when it changes.
pub(crate) fn set_modality(m: Modality) {
    MODALITY.with(|s| {
        if s.get_untracked() != m {
            s.set(m);
        }
    });
}

/// Where a focus scope puts focus as its layer opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoFocus {
    /// Its first node in Tab order; with none, the region itself.
    First,
    /// The region itself (a menu's frame, before an item is highlighted).
    Region,
    /// This node; when it is not there, as [`Self::First`].
    Node(FocusHandle),
    /// Nowhere: focus stays where it is (reka's prevented
    /// `open-auto-focus`), and the owner moves it if it wants.
    Keep,
}

/// What a layer does with focus while it is open (reka's `FocusScope`).
/// Scopes stack in opening order (the layers' order); only the top one acts, so a scope opened
/// above another, trapped or not, pauses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusScope {
    /// Focus cannot leave: a press, Tab or code moving it out is undone
    /// (to the node inside that had it last).
    pub trapped: bool,
    /// Tab at its last node goes to its first, Shift+Tab the other way;
    /// otherwise a trapped scope stops there and one that is not lets
    /// focus go on past its owner.
    pub cycle: bool,
    pub auto_focus: AutoFocus,
    /// As it closes, focus goes back to what held it as it opened, unless
    /// something else has taken focus since.
    pub restore: bool,
    /// Where restoring focus goes first, when it is there: a popover's or a
    /// drawer's trigger (reka's `PopoverContent` and `DrawerContent` focus
    /// their trigger, not what held focus as they opened).
    pub return_to: Option<FocusHandle>,
}

impl FocusScope {
    /// Not trapped, cycling, focusing its first node, restoring focus.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            trapped: false,
            cycle: true,
            auto_focus: AutoFocus::First,
            restore: true,
            return_to: None,
        }
    }

    /// Trapped and cycling, as a modal dialog keeps focus.
    #[must_use]
    pub const fn trapped() -> Self {
        Self {
            trapped: true,
            ..Self::new()
        }
    }

    #[must_use]
    pub const fn trap(self, trapped: bool) -> Self {
        Self { trapped, ..self }
    }

    #[must_use]
    pub const fn cycle(self, cycle: bool) -> Self {
        Self { cycle, ..self }
    }

    #[must_use]
    pub const fn auto_focus(self, auto_focus: AutoFocus) -> Self {
        Self { auto_focus, ..self }
    }

    #[must_use]
    pub const fn restore(self, restore: bool) -> Self {
        Self { restore, ..self }
    }

    /// Give focus back to `to` as it closes, when `to` is there; else as
    /// [`Self::restore`] says.
    #[must_use]
    pub const fn return_to(self, to: FocusHandle) -> Self {
        Self {
            return_to: Some(to),
            ..self
        }
    }
}

impl Default for FocusScope {
    fn default() -> Self {
        Self::new()
    }
}

/// The frame node holding focus `id`: one not shown ([`Frame::shown`])
/// holds none, as one not laid out holds none.
pub(crate) fn node_of(frame: &Frame, id: FocusId) -> Option<usize> {
    (0..frame.nodes.len()).find(|&i| frame.shown(i) && focus_at(frame, i) == Some(id))
}

/// The focus id of node `i`, if it is focusable.
fn focus_at(frame: &Frame, i: usize) -> Option<FocusId> {
    frame.behaviour(i).and_then(|b| b.focus).map(FocusHandle::id)
}

/// Whether node `i` is a Tab stop: focusable, shown, not below a modal
/// layer, and not a focus scope's region (which takes focus as a last
/// resort, as reka's `tabindex="-1"` content does, but is no stop). A node
/// out of Tab order ([`crate::Div::tab_stop`]) is one while it holds focus,
/// so Tab goes on from it.
fn tabbable(frame: &Frame, stack: &Stack, i: usize) -> bool {
    let Some(b) = frame.behaviour(i) else {
        return false;
    };
    b.focus
        .is_some_and(|f| Some(f.id()) == focused_now() || b.tab_stop.get())
        && frame.shown(i)
        && !stack.blocked(i)
        && !b.handlers.layer.as_ref().is_some_and(|l| l.scope.is_some())
        && (b.focus.is_some_and(|f| Some(f.id()) == focused_now()) || !stack.untabbable(frame, i))
}

/// Every Tab stop of `frame` in Tab order: the tree's order, each deferred
/// subtree right after its owner's subtree (where it is declared, not at
/// the end of the page as a portal puts it).
pub(crate) fn tab_order(frame: &Frame, stack: &Stack) -> Vec<usize> {
    let layers = frame.layers.len().saturating_add(1);
    // Where each subtree goes: after which node (the end of its owner's subtree).
    let mut after: Vec<(usize, usize)> = Vec::new();
    let mut orphans: Vec<usize> = Vec::new();
    for l in 1..layers {
        match stack.tab_owners.get(l).copied().flatten() {
            Some(o) => after.push((subtree_end(frame, o), l)),
            None => orphans.push(l),
        }
    }
    let mut out = Vec::new();
    // (layer, next node of it) frames, innermost last.
    let mut walk: Vec<(usize, usize)> = vec![(0, 0)];
    let mut orphans = orphans.into_iter();
    while walk.len() <= layers.saturating_add(1) {
        let Some(top) = walk.last_mut() else {
            match orphans.next() {
                Some(l) => {
                    walk.push((l, layer_start(frame, l)));
                    continue;
                }
                None => break,
            }
        };
        let (l, i) = *top;
        if i >= layer_start(frame, l.saturating_add(1)) {
            walk.pop();
            continue;
        }
        top.1 = i.saturating_add(1);
        if tabbable(frame, stack, i) {
            out.push(i);
        }
        // The first declared after `i` is walked first: pushed last.
        for &(at, k) in after.iter().rev() {
            if at == i {
                walk.push((k, layer_start(frame, k)));
            }
        }
    }
    for r in 0..frame.nodes.len() {
        if frame.behaviour(r).is_some_and(|b| b.tab_reversed) {
            reverse_children(frame, stack, r, &mut out);
        }
    }
    out
}

/// The child of node `r` whose subtree holds stop `i`, a layer declared in
/// that subtree counting as in it.
fn child_holding(frame: &Frame, stack: &Stack, r: usize, mut i: usize) -> Option<usize> {
    let layer = frame.nodes.get(r)?.layer;
    while frame.nodes.get(i)?.layer != layer {
        let l = frame.nodes.get(i)?.layer;
        i = stack.owners.get(l).copied().flatten()?;
    }
    loop {
        let parent = frame.nodes.get(i)?.parent?;
        if parent == r {
            return Some(i);
        }
        i = parent;
    }
}

/// Reorder the stops in `order` that lie under node `r`'s children so the
/// children come last first, each child's own stops kept in their order
/// ([`crate::Div::tab_children_reversed`]).
fn reverse_children(frame: &Frame, stack: &Stack, r: usize, order: &mut [usize]) {
    let held: Vec<Option<usize>> = order.iter().map(|&i| child_holding(frame, stack, r, i)).collect();
    let mut start = 0;
    while start < order.len() {
        if held.get(start).copied().flatten().is_none() {
            start = start.saturating_add(1);
            continue;
        }
        // A run of stops under `r`'s children: its groups, one per child.
        let mut end = start;
        let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
        while let Some(Some(c)) = held.get(end).copied() {
            match groups.last_mut() {
                Some(g) if held.get(g.start).copied().flatten() == Some(c) => g.end = end.saturating_add(1),
                _ => groups.push(end..end.saturating_add(1)),
            }
            end = end.saturating_add(1);
        }
        let run: Vec<usize> = groups
            .iter()
            .rev()
            .flat_map(|g| order.get(g.clone()).unwrap_or_default().iter().copied())
            .collect();
        if let Some(slot) = order.get_mut(start..end) {
            slot.copy_from_slice(&run);
        }
        start = end;
    }
}

/// The last node of node `o`'s subtree (its layer's nodes are in depth-first order).
fn subtree_end(frame: &Frame, o: usize) -> usize {
    let layer = frame.nodes.get(o).map_or(0, |n| n.layer);
    let mut end = o;
    while frame
        .nodes
        .get(end.saturating_add(1))
        .is_some_and(|n| n.layer == layer && within(frame, end.saturating_add(1), o))
    {
        end = end.saturating_add(1);
    }
    end
}

/// The focus without subscribing to it.
fn focused_now() -> Option<FocusId> {
    current().focused.get_untracked()
}

/// Move focus to `to` as the engine does: with its ring when the user is
/// at the keyboard, and all of a text field's text selected (reka's
/// `focus(…, { select: true })`).
pub(crate) fn engine_focus(frame: &Frame, to: Option<FocusId>) {
    set_focus(to, false);
    if let Some(field) = to
        .and_then(|id| node_of(frame, id))
        .and_then(|i| frame.behaviour(i))
        .and_then(|b| b.field)
    {
        field.edit.update(TextEdit::select_all);
    }
}

/// Focus left on a node `frame` (a frame's last pass) does not hold or
/// does not show, which no focus rule moved: nothing holds it, as Chrome's
/// focus fixup gives focus to the body when the focused element goes or
/// stops being rendered. Its id is dropped, so the same handle laid out
/// again later does not take focus back unasked. Whether it was.
pub(crate) fn drop_lost(frame: &Frame) -> bool {
    let lost = focused_now().is_some_and(|id| node_of(frame, id).is_none());
    if lost {
        set_focus(None, false);
    }
    lost
}

/// A scope open in the last settled frame.
struct Open {
    /// Its region's path.
    path: NodePath,
    /// Its owner's path.
    owner: Option<NodePath>,
    /// What held focus as it opened.
    to: Option<FocusId>,
    /// The node inside it that last held focus.
    last_inside: Option<FocusId>,
    scope: FocusScope,
    /// A press outside it, other than a primary press outside a modal
    /// layer, asked it to close in this frame and was answered `Close`:
    /// focus is not given back (reka's `hasInteractedOutside`).
    pressed_outside: bool,
}

/// What settling focus did.
#[derive(Default)]
pub(crate) struct Settled {
    /// Focus moved.
    pub(crate) moved: bool,
    /// The layers focus came outside of, to ask to close.
    pub(crate) outside: Vec<Rc<Layer>>,
}

/// The focus manager (ADR-0024): the stack of open focus scopes, kept
/// across frames by their regions' paths, and the rules run on each laid
/// out frame: a scope that closed gives focus back, one that opened takes
/// it, the top trapped one keeps it, and focus that came outside a layer
/// asks it to close.
#[derive(Default)]
pub(crate) struct FocusManager {
    open: Vec<Open>,
    /// Focus as the last settle left it.
    last: Option<FocusId>,
    /// Every layer's region path as the last settle saw them: a layer not
    /// among them opened in this frame, and focus its opening moved is no
    /// focus outside it.
    seen: Vec<NodePath>,
    /// The frame (its epoch and change count, which a trim never repeats)
    /// and the focus the last settle left: the rules read nothing else, so
    /// the same again is settled already.
    settled: Option<((u64, usize), Option<FocusId>)>,
}

impl FocusManager {
    /// Apply the rules to `frame`, whose layers are `stack`. A frame and a
    /// focus already settled cost nothing; otherwise, while no scope opens
    /// or closes, nothing is allocated.
    pub(crate) fn settle(&mut self, frame: &Frame, stack: &Stack) -> Settled {
        let before = focused_now();
        let handed = take_hand_on(frame, stack);
        let key = (frame.epoch(), frame.changes.end());
        if !handed && self.settled == Some((key, before)) {
            return Settled::default();
        }
        // Paths are shared: a clone is a count, not an allocation.
        let path = |i: usize| frame.nodes.get(i).map(|n| n.path.clone());
        let now = || {
            stack
                .entries
                .iter()
                .filter(|e| e.spec.scope.is_some())
                .filter_map(move |e| Some((e, path(e.node)?)))
        };
        // Closed: bottom first, so a chain closing together gives focus
        // back to the outermost's (the others then find it taken).
        let present = |o: &Open| now().any(|(_, p)| p == o.path);
        let closed: Vec<Open> = if self.open.iter().all(present) {
            Vec::new()
        } else {
            let (closed, kept) = std::mem::take(&mut self.open).into_iter().partition(|o| !present(o));
            self.open = kept;
            closed
        };
        for c in closed {
            if !c.scope.restore {
                continue;
            }
            // Focus left on a node that went or is below a modal: it was in
            // the scope, or nowhere a reader can be. Focus moved elsewhere
            // or dropped (a press on nothing focusable) since: kept.
            let f = focused_now();
            let gone = f.is_some_and(|id| node_of(frame, id).is_none_or(|i| stack.blocked(i)));
            if !gone {
                continue;
            }
            if c.pressed_outside {
                // Focus left with it: nothing holds it, as a browser's
                // `<body>` does when focused content is removed.
                set_focus(None, false);
                continue;
            }
            let live = |id: &FocusId| node_of(frame, *id).is_some_and(|i| !stack.blocked(i));
            let to = c
                .scope
                .return_to
                .map(FocusHandle::id)
                .filter(live)
                .or_else(|| c.to.filter(live))
                .or_else(|| c.owner.as_ref().and_then(|o| near_owner(frame, stack, o)));
            // Nowhere to give it back to (its opener went with it): left
            // where it was, gone, for an enclosing trap's rule below to take
            // back inside it, else for the fixup at the end to drop.
            if to.is_some() {
                engine_focus(frame, to);
            }
        }
        // Opened: bottom first, so the top one's focus wins.
        let mut opened = false;
        for (e, p) in now() {
            if let Some(o) = self.open.iter_mut().find(|o| o.path == p) {
                o.scope = e.spec.scope.unwrap_or_default();
                continue;
            }
            opened = true;
            let scope = e.spec.scope.unwrap_or_default();
            let held = focused_now();
            let inside = held
                .and_then(|id| node_of(frame, id))
                .is_some_and(|i| stack.in_scope(frame, e, i));
            if !inside {
                let to = match scope.auto_focus {
                    AutoFocus::First => first_in(frame, stack, e),
                    AutoFocus::Region => focus_at(frame, e.node),
                    AutoFocus::Node(h) if node_of(frame, h.id()).is_some() => Some(h.id()),
                    AutoFocus::Node(_) => first_in(frame, stack, e),
                    AutoFocus::Keep => None,
                };
                if to.is_some() {
                    engine_focus(frame, to);
                }
            }
            // What held focus as it opened: focus already moved inside it
            // in the frame it opened (its own code focused it), what held
            // it before that frame.
            let to = if inside { self.last } else { held };
            self.open.push(Open {
                path: p,
                owner: e.owner.and_then(path),
                to,
                last_inside: None,
                scope,
                pressed_outside: false,
            });
        }
        if opened {
            // Kept in the stack's order, bottom first, for the next close.
            self.open
                .sort_by_key(|o| now().position(|(_, p)| p == o.path).unwrap_or(usize::MAX));
        }
        // The top scope, trapped, keeps focus.
        if let Some(top) = stack.top_scope()
            && top.spec.scope.is_some_and(|s| s.trapped)
            && let Some(f) = focused_now()
        {
            let pull = match node_of(frame, f) {
                // The node holding it went away: the region takes it.
                None => Some(focus_at(frame, top.node).or_else(|| first_in(frame, stack, top))),
                Some(i) if !stack.in_scope(frame, top, i) => {
                    let last = path(top.node)
                        .and_then(|p| self.open.iter().find(|o| o.path == p))
                        .and_then(|o| o.last_inside)
                        .filter(|id| node_of(frame, *id).is_some_and(|i| stack.in_scope(frame, top, i)));
                    Some(last.or_else(|| first_in(frame, stack, top)))
                }
                Some(_) => None,
            };
            if let Some(to) = pull {
                engine_focus(frame, to);
            }
        }
        // Focus that came to a node outside a layer asks it to close.
        let f = focused_now();
        let mut outside = Vec::new();
        if f != self.last
            && let Some(i) = f.and_then(|id| node_of(frame, id))
        {
            for e in &stack.entries {
                let opened_now = path(e.node).is_none_or(|p| !self.seen.contains(&p));
                if e.spec.policy.focus_outside && !opened_now && !stack.inside(frame, e, i) {
                    outside.push(Rc::clone(&e.spec));
                }
            }
        }
        if let Some(i) = f.and_then(|id| node_of(frame, id)) {
            for (e, p) in now() {
                if stack.in_scope(frame, e, i)
                    && let Some(o) = self.open.iter_mut().find(|o| o.path == p)
                {
                    o.last_inside = f;
                }
            }
        }
        // A press outside that did not close its layer in this frame says
        // nothing about a later close.
        for o in &mut self.open {
            o.pressed_outside = false;
        }
        // Focus left on a node removed or not shown is dropped once the
        // frame's last pass is laid out ([`drop_lost`]), not here: a later
        // pass of this frame may show it again.
        self.last = f;
        self.seen.clear();
        self.seen.extend(stack.entries.iter().filter_map(|e| path(e.node)));
        self.settled = Some((key, f));
        Settled {
            moved: f != before,
            outside,
        }
    }

    /// A press outside the layer whose region is node `i` closed it, and
    /// it is not a primary press outside a modal layer: its scope gives no
    /// focus back as it closes (ADR-0024).
    pub(crate) fn pressed_outside(&mut self, frame: &Frame, i: usize) {
        let Some(p) = frame.nodes.get(i).map(|n| &n.path) else {
            return;
        };
        if let Some(o) = self.open.iter_mut().find(|o| o.path == *p) {
            o.pressed_outside = true;
        }
    }

    /// Where Tab (Shift+Tab when `back`) moves focus in `frame`: through the
    /// top scope's stops, at its ends cycling, stopping (trapped) or going
    /// on past its owner; with no scope, through the whole order, cycling.
    /// `None` leaves focus where it is.
    pub(crate) fn tab(frame: &Frame, stack: &Stack, back: bool) -> Option<FocusId> {
        let order = tab_order(frame, stack);
        let current = focused_now().and_then(|id| node_of(frame, id));
        let top = stack.top_scope();
        let ring: Vec<usize> = match top {
            Some(e) => order.iter().copied().filter(|&i| stack.in_scope(frame, e, i)).collect(),
            None => order.clone(),
        };
        let step = |ring: &[usize], at: Option<usize>, wrap: bool| -> Option<usize> {
            let len = ring.len();
            let next = match (at, back) {
                (None, false) => Some(0),
                (None, true) => len.checked_sub(1),
                (Some(p), false) if p.saturating_add(1) < len => Some(p.saturating_add(1)),
                (Some(p), true) if p > 0 => Some(p.saturating_sub(1)),
                (Some(_), false) => wrap.then_some(0),
                (Some(_), true) => wrap.then(|| len.saturating_sub(1)),
            };
            next.and_then(|n| ring.get(n).copied())
        };
        let at = current.and_then(|c| ring.iter().position(|&i| i == c));
        let scope = top.and_then(|e| e.spec.scope).unwrap_or_default();
        let to = match step(&ring, at, scope.cycle || top.is_none()) {
            Some(i) => Some(i),
            None if top.is_some() && !scope.trapped && !scope.cycle => {
                // Past its end, focus goes on in the page's order.
                let at = current.and_then(|c| order.iter().position(|&i| i == c));
                step(&order, at, true)
            }
            None => None,
        };
        to.and_then(|i| focus_at(frame, i))
    }
}

/// The first Tab stop inside `e`; with none, its region.
fn first_in(frame: &Frame, stack: &Stack, e: &Entry) -> Option<FocusId> {
    tab_order(frame, stack)
        .into_iter()
        .find(|&i| stack.in_scope(frame, e, i))
        .and_then(|i| focus_at(frame, i))
        .or_else(|| focus_at(frame, e.node))
}

/// Where focus goes back to when what held it is gone: the first focusable
/// node of the owner at `owner` (its trigger), else its nearest focusable
/// ancestor.
fn near_owner(frame: &Frame, stack: &Stack, owner: &NodePath) -> Option<FocusId> {
    let o = frame.nodes.iter().position(|n| n.path == *owner)?;
    if stack.blocked(o) {
        return None;
    }
    (o..=subtree_end(frame, o))
        .find_map(|i| focus_at(frame, i))
        .or_else(|| frame.ancestors(o).find_map(|i| focus_at(frame, i)))
}
