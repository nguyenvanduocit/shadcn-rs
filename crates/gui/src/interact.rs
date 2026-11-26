//! Routing pointer, touch, wheel and keyboard input to the nodes of the
//! last frame: hit testing (stopping at the top modal layer), hover and
//! press state, clicks, taps, long presses and drags, dismissing layers
//! (ADR-0024: a press or focus outside, Escape, a scroll), scrolling, Tab
//! through the focus scopes, and text editing.

use std::borrow::Cow;
use std::collections::HashSet;

use accesskit::{Action, ActionData, ActionRequest};

use crate::access;
use crate::compositor::motion;
use crate::edit::TextField;
use crate::element::{InputMode, InputType};
use crate::engine::{Commit, Owned};
use crate::events::{
    ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey, PointerId, TouchEvent, TouchPhase,
};
use crate::focus::{self, FocusId, FocusManager, Modality};
use crate::geometry::{Bounds, NodePath};
use crate::gesture::Gesture;
use crate::layer::{Dismiss, DismissReason, PointerButton, PointerKind, PointerOutside, Stack};
use crate::layout::{Behaviour, Content, Frame, LayoutTree, UiState, input_text};
use crate::reactive::Signal;
use crate::scroll::ScrollHandle;
use crate::style::{Cursor, ResolvedText};
use crate::text::Step;
use crate::text::TextSystem;

/// Pixels scrolled per wheel line.
pub const LINE_SCROLL: f32 = 40.0;

/// What the pointer and the keyboard are doing to one node, one signal per
/// flag (ADR-0022): a binding that reads [`Self::focus_visible`] runs again
/// when focus shows or hides, not when the pointer comes over the node.
///
/// A handle to a node's flags, not the flags: made anywhere
/// ([`Interaction::new`] costs a number, no signal), given to a node
/// ([`crate::Div::interaction`]) and read in that node's bindings, or in its
/// children's for what a container's state changes below it
/// (`.background(move || if i.hovered() { a } else { b })`). The flags are
/// the node's own ([`NodeFlags`]): the layout tree makes them once, when
/// the node is first built with a handle, keeps them while the node lives
/// and binds the handle it was last given to them. A part a render makes
/// again with a new handle each time (a table row, a checkbox's box) binds
/// the new handle to the same flags: a render makes no signal, and the
/// handle reads the node as it is. Hit testing and focus write the flags,
/// each only when it changes; the layout tree writes all of them from the
/// window's input state when the node is built or patched.
///
/// Read where no reader runs, a flag is the value it holds now. Read before
/// its node is built (an effect made before the tree), a handle reads off,
/// and the reader runs again once the handle is bound; a render reading
/// one so does not (it would make a new handle and read it again).
///
/// It names one node: given to two, it holds the flags of whichever was
/// built last. A handle whose node has gone (or was given another handle)
/// reads every flag off, the look of a node at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Interaction(u64);

/// A node's flags, one signal per flag, made by the layout tree once per
/// node that is given an [`Interaction`] and disposed with the node: what
/// hit testing and focus write, and what the handles bound to it read.
/// A closure reads a flag's signal under its own tracker; a source
/// ([`Interaction::hover`]) registers with the handle (its chain of the
/// flag sources' list, `value::source`), which a write of a flag wakes,
/// for that flag alone, through `bound`.
#[derive(Clone)]
pub(crate) struct NodeFlags {
    hovered: Owned<bool>,
    pressed: Owned<bool>,
    focused: Owned<bool>,
    focus_visible: Owned<bool>,
    focus_within: Owned<bool>,
    disabled: Owned<bool>,
    /// The handle bound to them, shared by every copy: the world's that
    /// made the flags (a check binds none of the live ones, review r18).
    bound: std::rc::Rc<crate::engine::WorldCell<Option<Interaction>>>,
}

crate::engine::fork_clone_by_clone!(NodeFlags);

impl PartialEq for NodeFlags {
    fn eq(&self, other: &Self) -> bool {
        self.hovered == other.hovered && std::rc::Rc::ptr_eq(&self.bound, &other.bound)
    }
}

/// Wake the sources of `handle` registered for any of `mask` (none in a
/// cold frame: what it writes is undone). A write of a flag of the node it
/// is bound to wakes them for that flag; binding it to a node, or
/// unbinding it, wakes all of them.
fn wake_sources(handle: Interaction, mask: u8) {
    crate::value::source::wake_handle(handle.0, mask);
}

impl std::fmt::Debug for NodeFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NodeFlags").field(&self.hovered).finish()
    }
}

/// One of an [`Interaction`]'s flags, as a source names it
/// ([`Interaction::when`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flag {
    /// [`Interaction::hovered`].
    Hovered,
    /// [`Interaction::pressed`].
    Pressed,
    /// [`Interaction::focused`].
    Focused,
    /// [`Interaction::focus_visible`].
    FocusVisible,
    /// [`Interaction::focus_within`].
    FocusWithin,
    /// [`Interaction::disabled`].
    Disabled,
}

impl Flag {
    /// Its bit in a wake's mask.
    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::Hovered => 1,
            Self::Pressed => 1 << 1,
            Self::Focused => 1 << 2,
            Self::FocusVisible => 1 << 3,
            Self::FocusWithin => 1 << 4,
            Self::Disabled => 1 << 5,
        }
    }
}

/// The handles made, numbered ([`crate::engine::unique_id`]): a number a
/// fork hands out is never handed out again live.
static INTERACTIONS: crate::engine::IdKind = crate::engine::IdKind::new("interaction");

// What the handles of this thread are bound to: a piece each, so a cold
// frame forks each by its own policy (its maps by overlay, its scalars
// whole) and no composite log needs a mutable path to its maps (ADR-0034,
// S7: a logged map is written through its overlay API alone).
crate::engine::engine_local! {
    /// The flags each handle is bound to.
    [Fork(log)] static BOUND: std::cell::RefCell<crate::engine::ForkMap<u64, NodeFlags>> =
        std::cell::RefCell::default();
    /// Handles read before they were bound, outside a render, each with
    /// when it was first read (the oldest goes past [`EARLY`]).
    [Fork(log)] static EARLY_READ: std::cell::RefCell<crate::engine::ForkMap<u64, u64>> =
        std::cell::RefCell::default();
    /// The clock dating [`EARLY_READ`]'s reads.
    [Fork] static READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// What the early readers subscribed to: bumped when one of them is
    /// bound.
    [Fork] static BIND: std::cell::Cell<Option<Signal<u64>>> = const { std::cell::Cell::new(None) };
    /// The drag handler running now claimed its pointer's press
    /// ([`claim_press`]); read and cleared around each drag event.
    [Fork] static CLAIMED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The press of the pointer whose drag this is, is a drag, not a click:
/// its release clicks nothing under it (a carousel dragged past its slop,
/// reka's `swallowNextClick`). Called from an `on_drag` handler; it claims
/// only the press of the pointer (the mouse, or one contact) whose drag
/// event the handler is given, and that pointer's next press starts
/// unclaimed.
pub fn claim_press() {
    CLAIMED.with(|c| c.set(true));
}

/// Whether the window has the keyboard focus (tracked; true until the
/// platform says otherwise): toasts pause while it has not (reka's
/// `pauseOnWindowBlur`), a drag ends where it is when it loses it.
#[must_use]
pub fn window_active() -> bool {
    crate::focus::window_signals().active.get()
}

/// Record whether the window has the keyboard focus, written only when it
/// changed (the platform's focus event, a scenario's [`crate::app::Scenario::set_window_active`]).
pub(crate) fn set_window_active(t: &crate::engine::Turn, active: bool) {
    let a = crate::focus::window_signals().active;
    crate::engine::commit(t, |c| a.put(active, c));
}

/// Where the pointer is over the window, in window pixels, `None` while it
/// is outside it (tracked): what a hover card's or a tooltip's grace area
/// follows between its trigger and its content once the pointer has left
/// both (reka's `pointermove` on the document).
#[must_use]
pub fn pointer_position() -> Option<(f32, f32)> {
    crate::focus::window_signals().pointer.get()
}

/// Record where the pointer is, written only when it moved.
fn track_pointer(at: Option<(f32, f32)>) {
    let p = crate::focus::window_signals().pointer;
    crate::engine::commit_here(|c| p.put(at, c));
}

/// The user acts by `m` now, as the input that shows it comes in, before
/// any handler of it runs.
fn acts_by(m: Modality) {
    crate::engine::commit_here(|c| focus::set_modality(m, c));
}

/// How many handles read before they were bound are remembered.
const EARLY: usize = 64;

/// What a pointer or keyboard event is to the nodes it reaches, which
/// decides how a node that cannot be used stops it (ADR-0025 §2, the
/// primitives' convention 6). Disabled is inherited (a node under a
/// disabled one is disabled, [`disabled`]), so a press inside a disabled
/// subtree lands on disabled nodes up to the subtree's root, and on
/// enabled ones above it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    /// It acts on what it lands on: a click, Enter or Space, a screen
    /// reader's Click. A disabled or unavailable node takes none and none
    /// passes through it to its ancestors, as `aria-disabled` asks.
    Activation,
    /// A gesture an ancestor may own: a press, a drag, a touch, a long
    /// press, a secondary click. It passes a disabled or unavailable node
    /// by to its ancestors, as pointer events bubble past an
    /// `aria-disabled` element on the web and, in Chrome, past a disabled
    /// form control (a drawer swipes from a disabled button or an
    /// unavailable day, a context menu opens around an unavailable item);
    /// the node's own handler does not run, as it would act on it.
    Gesture,
}

impl Reach {
    /// Whether `b`'s own handler of this kind runs.
    fn runs_on(self, b: &Behaviour) -> bool {
        b.takes_clicks()
    }

    /// Whether the event goes on through `b` to its ancestors.
    fn passes(self, b: &Behaviour) -> bool {
        match self {
            Self::Activation => b.takes_clicks(),
            Self::Gesture => true,
        }
    }

    /// The nearest node from `h` up (itself first) that takes an event of
    /// this kind with a handler `has` finds on it.
    fn target(self, frame: &Frame, h: usize, has: impl Fn(&Behaviour) -> bool) -> Option<usize> {
        for i in frame.ancestors(h) {
            let b = frame.behaviour(i)?;
            if self.runs_on(b) && has(b) {
                return Some(i);
            }
            if !self.passes(b) {
                return None;
            }
        }
        None
    }

    /// Whether an event of this kind from `from` goes up to `to`: `to` is
    /// `from` or above it, and every node from `from` up to below `to`
    /// passes it on.
    fn reaches(self, frame: &Frame, from: usize, to: usize) -> bool {
        for i in frame.ancestors(from) {
            if i == to {
                return true;
            }
            if !frame.behaviour(i).is_some_and(|b| self.passes(b)) {
                return false;
            }
        }
        false
    }
}

/// Whether node `i` of `frame` is disabled ([`crate::Div::disabled`]), by
/// itself or by an ancestor: it takes no focus, keys or activation, and
/// its own handlers do not run. A node the frame does not hold is.
fn disabled(frame: &Frame, i: usize) -> bool {
    frame.behaviour(i).is_none_or(|b| b.disabled)
}

/// Whether the press `chain` (the node hit, then its ancestors) went down
/// on a control below the dragged node `drag`: a text input, or a node
/// that takes clicks or whose role is a control's and that is neither
/// disabled nor unavailable (such a one takes no press, so the drag has it).
fn on_control(frame: &Frame, chain: &[usize], drag: usize) -> bool {
    use accesskit::Role;
    chain.iter().take_while(|&&i| i != drag).any(|&i| {
        frame.behaviour(i).is_some_and(|b| {
            let control = b.handlers.click.is_some()
                || matches!(
                    b.semantics.role,
                    Some(
                        Role::Button
                            | Role::Link
                            | Role::CheckBox
                            | Role::RadioButton
                            | Role::Switch
                            | Role::Slider
                            | Role::ComboBox
                            | Role::TextInput
                            | Role::MultilineTextInput
                            | Role::Label
                    )
                );
            (b.field.is_some() && !b.disabled) || (control && b.takes_clicks())
        })
    })
}

/// Whether scroll container `i` can still scroll by `pan` along the axis
/// the pan mostly goes (down and right positive).
fn can_scroll_toward(frame: &Frame, i: usize, pan: (f32, f32)) -> bool {
    let Some(s) = frame.nodes.get(i).and_then(laid_scroll) else {
        return false;
    };
    let (d, offset, max) = if pan.1.abs() >= pan.0.abs() {
        (pan.1, s.offset.1, s.max.1)
    } else {
        (pan.0, s.offset.0, s.max.0)
    };
    max > 0.0 && ((d > 0.0 && offset < max) || (d < 0.0 && offset > 0.0))
}

/// A flag, tracked; off once its node has gone.
fn read(flag: Owned<bool>) -> bool {
    flag.is_alive() && flag.get()
}

impl Default for Interaction {
    fn default() -> Self {
        Self::new()
    }
}

impl Interaction {
    /// A handle bound to no node yet: every flag off.
    #[must_use]
    pub fn new() -> Self {
        Self(crate::engine::unique_id(&INTERACTIONS))
    }

    /// Whether a node's flags are bound to it (a cold layout binds only
    /// the handles no live node holds: those its own renders made).
    pub(crate) fn is_bound(self) -> bool {
        BOUND.with_borrow(|b| b.contains_key(&self.0))
    }

    /// The flags it is bound to; unbound, `None`, and a reader outside a
    /// render subscribed to its binding.
    fn flags(self) -> Option<NodeFlags> {
        let (flags, bind) = self.lookup();
        if let Some(bind) = bind
            && bind.is_alive()
        {
            let _subscribe = bind.get();
        }
        flags
    }

    /// `flag` as it is now, untracked (a source's mount, woken through
    /// [`Self::sources`], not by its binding): one lookup.
    pub(crate) fn peek(self, flag: Flag) -> bool {
        let signal = BOUND.with_borrow(|b| b.get(&self.0).map(|f| f.signal(flag)));
        signal.is_some_and(|s| s.is_alive() && s.get_untracked())
    }

    /// `flag`, tracked (a source read as a closure is).
    pub(crate) fn flag(self, flag: Flag) -> bool {
        self.flags().is_some_and(|f| read(f.signal(flag)))
    }

    /// Its number: the chain its sources register in, bound or not.
    pub(crate) const fn id(self) -> u64 {
        self.0
    }

    /// The flags it is bound to, and, unbound and read outside a render,
    /// what is written when it is bound (it is recorded as read early).
    fn lookup(self) -> (Option<NodeFlags>, Option<Signal<u64>>) {
        if let Some(f) = BOUND.with_borrow(|b| b.get(&self.0).cloned()) {
            return (Some(f), None);
        }
        if crate::reactive::phase() == crate::reactive::Phase::Render {
            return (None, None);
        }
        EARLY_READ.with_borrow_mut(|early| {
            if !early.contains_key(&self.0) {
                // A handle read early and never bound (its node never came)
                // goes once others are.
                if early.len() >= EARLY
                    && let Some(oldest) = early.iter().min_by_key(|(_, at)| **at).map(|(k, _)| *k)
                {
                    early.remove(&oldest);
                }
                let at = READS.get().wrapping_add(1);
                READS.set(at);
                early.insert(self.0, at);
            }
        });
        let bind = BIND.get().unwrap_or_else(|| {
            let made = crate::reactive::detached(|| Signal::new(0_u64));
            BIND.set(Some(made));
            made
        });
        (None, Some(bind))
    }

    /// The pointer is over the node or one of its descendants, and the node
    /// is not disabled (CSS `:hover`, of a node that takes the pointer).
    #[must_use]
    pub fn hovered(self) -> bool {
        self.flags().is_some_and(|f| read(f.hovered))
    }

    /// A pointer went down on the node or one of its descendants and is
    /// still down (the mouse's primary button, a finger, a pen: any of
    /// them, whatever the others do), and the node is not disabled
    /// (`:active`).
    #[must_use]
    pub fn pressed(self) -> bool {
        self.flags().is_some_and(|f| read(f.pressed))
    }

    /// The node holds keyboard focus (`:focus`).
    #[must_use]
    pub fn focused(self) -> bool {
        self.flags().is_some_and(|f| read(f.focused))
    }

    /// The node holds focus and shows it: focus came by keyboard, or the
    /// node is a text input, which shows focus however it came
    /// (`:focus-visible`).
    #[must_use]
    pub fn focus_visible(self) -> bool {
        self.flags().is_some_and(|f| read(f.focus_visible))
    }

    /// The node or one of its descendants holds focus (`:focus-within`).
    #[must_use]
    pub fn focus_within(self) -> bool {
        self.flags().is_some_and(|f| read(f.focus_within))
    }

    /// The node is disabled ([`crate::Div::disabled`], `:disabled`): it takes
    /// no clicks, keys or focus, and is never hovered or pressed.
    #[must_use]
    pub fn disabled(self) -> bool {
        self.flags().is_some_and(|f| read(f.disabled))
    }
}

/// A node's flags and the handle bound to them, as the layout tree keeps
/// them on the node: made with the first handle, bound to each handle the
/// node is given after, unbound and disposed when the node goes.
#[derive(Default)]
pub(crate) struct NodeInteraction {
    flags: Option<NodeFlags>,
    bound: Option<Interaction>,
}

impl NodeInteraction {
    /// Bind `handle` to the node's flags, made now if the node has none;
    /// the flags.
    pub(crate) fn bind(&mut self, handle: Interaction) -> NodeFlags {
        let flags = self
            .flags
            .get_or_insert_with(|| NodeFlags {
                hovered: Owned::labeled("hovered", false),
                pressed: Owned::labeled("pressed", false),
                focused: Owned::labeled("focused", false),
                focus_visible: Owned::labeled("focus visible", false),
                focus_within: Owned::labeled("focus within", false),
                disabled: Owned::labeled("disabled", false),
                bound: std::rc::Rc::new(crate::engine::WorldCell::new(
                    "the handle bound to a node's flags",
                    None,
                )),
            })
            .clone();
        if self.bound != Some(handle) {
            self.unbind();
            self.bound = Some(handle);
            let cold = crate::value::source::cold();
            if !cold {
                flags.bound.set(Some(handle));
            }
            BOUND.with_borrow_mut(|b| {
                // The node it named before (given to two, it names the
                // last) writes it no more.
                if let Some(was) = b.insert(handle.0, flags.clone())
                    && was != flags
                    && !cold
                {
                    was.bound.set(None);
                }
            });
            let bumped = EARLY_READ
                .with_borrow_mut(|early| early.remove(&handle.0))
                .map(|_| BIND.get());
            // Who read it before it was bound reads it again (a closure,
            // through the signal); its sources read the node it names now.
            if let Some(Some(bind)) = bumped
                && bind.is_alive()
            {
                bind.update(|n| *n = n.wrapping_add(1));
            }
            wake_sources(handle, crate::value::source::EVERY);
        }
        flags
    }

    /// The node's flags, if it was ever given a handle.
    pub(crate) fn flags(&self) -> Option<NodeFlags> {
        self.flags.clone()
    }

    /// The handle bound to the node, if any.
    pub(crate) const fn handle(&self) -> Option<Interaction> {
        self.bound
    }

    /// The node was given no handle: its flags go.
    pub(crate) fn clear(&mut self) {
        self.unbind();
        if let Some(f) = self.flags.take() {
            f.dispose();
        }
    }

    /// Unbind the handle it holds, if it is still bound to these flags.
    fn unbind(&mut self) {
        let (Some(handle), Some(flags)) = (self.bound.take(), &self.flags) else {
            return;
        };
        // The thread may be ending (its handles with it).
        let unbound = BOUND
            .try_with(|b| {
                if let Ok(mut b) = b.try_borrow_mut()
                    && b.get(&handle.0) == Some(flags)
                {
                    b.remove(&handle.0);
                    return true;
                }
                false
            })
            .unwrap_or(false);
        if unbound {
            if !crate::value::source::cold() {
                flags.bound.set(None);
            }
            // Its sources read every flag off now.
            wake_sources(handle, crate::value::source::EVERY);
        }
    }
}

impl Drop for NodeInteraction {
    fn drop(&mut self) {
        self.clear();
    }
}

impl NodeFlags {
    /// Write the flags that differ from `flags` (the layout tree, building
    /// the node).
    pub(crate) fn write(&self, flags: Flags) {
        let changed = crate::engine::commit_here(|c| {
            self.put(Flag::Hovered, flags.hovered, c)
                | self.put(Flag::Pressed, flags.pressed, c)
                | self.put(Flag::Focused, flags.focused, c)
                | self.put(Flag::FocusVisible, flags.focus_visible, c)
                | self.put(Flag::FocusWithin, flags.focus_within, c)
                | self.put(Flag::Disabled, flags.disabled, c)
        });
        self.wake(changed);
    }

    /// Hit testing: the pointer came over the node or left it.
    pub(crate) fn set_hovered(&self, on: bool, c: &Commit) {
        self.wake(self.put(Flag::Hovered, on, c));
    }

    /// Hit testing: the button went down on the node or came up.
    pub(crate) fn set_pressed(&self, on: bool, c: &Commit) {
        self.wake(self.put(Flag::Pressed, on, c));
    }

    /// Focus moved, or came to show or hide.
    pub(crate) fn set_focus(&self, focused: bool, visible: bool, within: bool, c: &Commit) {
        let changed = self.put(Flag::Focused, focused, c)
            | self.put(Flag::FocusVisible, visible, c)
            | self.put(Flag::FocusWithin, within, c);
        self.wake(changed);
    }

    /// The signal of `flag`.
    const fn signal(&self, flag: Flag) -> Owned<bool> {
        match flag {
            Flag::Hovered => self.hovered,
            Flag::Pressed => self.pressed,
            Flag::Focused => self.focused,
            Flag::FocusVisible => self.focus_visible,
            Flag::FocusWithin => self.focus_within,
            Flag::Disabled => self.disabled,
        }
    }

    /// Write `on` to `flag`; its bit when that changed it, else 0.
    fn put(&self, flag: Flag, on: bool, c: &Commit) -> u8 {
        if self.signal(flag).put(on, c) { flag.bit() } else { 0 }
    }

    /// The sources of the handle bound to them registered for any of
    /// `changed` run again.
    fn wake(&self, changed: u8) {
        if let Some(handle) = self.bound.get() {
            wake_sources(handle, changed);
        }
    }

    /// Out of the graph (its node went); safe from a `Drop`.
    fn dispose(self) {
        for s in [
            self.hovered,
            self.pressed,
            self.focused,
            self.focus_visible,
            self.focus_within,
            self.disabled,
        ] {
            s.dispose_quietly();
        }
    }
}

/// The value of each of an [`Interaction`]'s flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    pub(crate) hovered: bool,
    pub(crate) pressed: bool,
    pub(crate) focused: bool,
    pub(crate) focus_visible: bool,
    pub(crate) focus_within: bool,
    pub(crate) disabled: bool,
}

/// The focus flags (`focused`, `focus_visible`, `focus_within`) of the
/// node at `path` in `layer`, which holds the focus or not (`focused`) and
/// is a text field or not, while the focused node is at `focus` (its path
/// and layer) and focus came by keyboard or not. One rule for the layout
/// tree, which writes them where it builds a node, and for the input
/// state, which writes them where focus moves: focus shows on a node the
/// keyboard focused, and on a text field however it was focused, as
/// Chrome's `:focus-visible`; it is within a node it is in or under, in
/// the node's layer.
pub(crate) fn focus_flags(
    path: &NodePath,
    layer: usize,
    focused: bool,
    field: bool,
    focus: Option<(&NodePath, usize)>,
    by_keyboard: bool,
) -> (bool, bool, bool) {
    let within = focused || focus.is_some_and(|(f, l)| l == layer && f.0.starts_with(&path.0));
    (focused, focused && (by_keyboard || field), within)
}

/// A node held across frames by its identity, not by its index: its path
/// (position and key, what [`access::id_of`] and the compositor's node key
/// are made from) and the component instance that made it. A frame rebuilt
/// while it is held (a node before it came or went) moves the indices
/// after, and the path finds the same node there, or finds it gone; a node
/// another instance made at its path (a component replaced by another in
/// one rebuild) is another node, and it is gone. Where it was last found is
/// kept, so that while the frame is only written in place finding it costs
/// one compare.
#[derive(Clone, Debug)]
struct Held {
    path: NodePath,
    /// The instance that made it ([`owner_of`]).
    owner: Option<crate::reactive::OwnerId>,
    /// The frame epoch and index it was last found at: the world's that
    /// made it, a check finding a live one keeps nothing (ADR-0034, S7).
    at: crate::engine::WorldCell<(u64, usize)>,
}

impl Held {
    /// Node `i` of `frame`.
    fn of(frame: &Frame, i: usize) -> Option<Self> {
        let n = frame.nodes.get(i)?;
        Some(Self {
            path: n.path.clone(),
            owner: owner_of(frame, i),
            at: crate::engine::WorldCell::new("where a held node was last found", (frame.epoch(), i)),
        })
    }

    /// Its index in `frame`, if `frame` holds it. Looked for once per
    /// rebuild of the frame: while nodes are only written in place it is
    /// where (or, gone, as absent as) it was last found; after one, by its
    /// key in the frame's index ([`NODES`]), not a walk of the frame.
    fn find(&self, frame: &Frame) -> Option<usize> {
        const GONE: usize = usize::MAX;
        let (epoch, i) = self.at.get();
        let here =
            |i: usize| frame.nodes.get(i).is_some_and(|n| n.path == self.path) && owner_of(frame, i) == self.owner;
        if epoch == frame.epoch() && (i == GONE || here(i)) {
            return (i != GONE).then_some(i);
        }
        let found = if here(i) {
            Some(i)
        } else {
            let key = crate::compositor::NodeKey::of(&self.path);
            NODES.with(|n| {
                let mut n = n.borrow_mut();
                n.refresh(frame);
                n.keyed(key, here)
            })
        };
        if self.at.is_here() {
            self.at.set((frame.epoch(), found.unwrap_or(GONE)));
        }
        found
    }
}

/// The component instance that made node `i` of `frame`: the nearest whose
/// output's root is it or above it, every node having one under a
/// component; else (a tree made outside any) what owned its handlers as
/// they were made.
pub(crate) fn owner_of(frame: &Frame, i: usize) -> Option<crate::reactive::OwnerId> {
    frame
        .ancestors(i)
        .find_map(|j| frame.behaviour(j).and_then(|b| b.handlers.instance))
        .or_else(|| frame.behaviour(i).and_then(|b| b.handlers.owner))
}

/// A node a gesture captured as it started, its handler of that gesture
/// and its box: those it had when last found, so a node that went still
/// gets the cancel that lets it go, with where it last was.
struct Grip<E> {
    node: Held,
    handler: crate::element::Handler<E>,
    /// Its box as the frame shown last showed it ([`Shown::bounds`]).
    shown_box: Bounds,
    /// The nodes captured with it ([`crate::Div::capture_group`]).
    group: Option<crate::events::CaptureGroup>,
}

/// Where a node's handler of a gesture's kind is.
type Pick<E> = fn(&Behaviour) -> Option<&crate::element::Handler<E>>;

fn drag_of(b: &Behaviour) -> Option<&crate::element::Handler<DragEvent>> {
    b.handlers.drag.as_ref()
}

fn touch_of(b: &Behaviour) -> Option<&crate::element::Handler<TouchEvent>> {
    b.handlers.touch.as_ref()
}

impl<E> Grip<E> {
    /// Node `i` of `frame` and its handler `pick` finds, its box as
    /// `shown` showed it ([`Shown::bounds`]).
    fn of(frame: &Frame, shown: &Shown, i: usize, pick: Pick<E>) -> Option<Self> {
        let b = frame.behaviour(i)?;
        Some(Self {
            handler: std::rc::Rc::clone(pick(b)?),
            shown_box: shown.bounds(i),
            node: Held::of(frame, i)?,
            group: b.handlers.capture_group,
        })
    }

    /// Whether it holds node `i` of `frame`, or a node of the capture
    /// group `group` (node `i`'s).
    fn holds(&self, frame: &Frame, i: usize, group: Option<crate::events::CaptureGroup>) -> bool {
        (group.is_some() && self.group == group) || self.node.find(frame) == Some(i)
    }

    /// Its node's index in `frame` while it still takes the gesture: there,
    /// neither disabled nor unavailable (the test a gesture starts by,
    /// [`Reach::runs_on`]), and with a handler of its kind, which it keeps,
    /// with its box as `shown` showed it. `None` once it does not: the
    /// gesture is cancelled.
    fn live(&mut self, frame: &Frame, shown: &Shown, pick: Pick<E>) -> Option<usize> {
        let i = self.node.find(frame)?;
        self.shown_box = shown.bounds(i);
        let b = frame.behaviour(i).filter(|b| b.takes_clicks())?;
        self.handler = std::rc::Rc::clone(pick(b)?);
        Some(i)
    }

    /// Its node's box as `shown` showed it, or, gone, as it was last found.
    fn bounds_in(&mut self, frame: &Frame, shown: &Shown) -> Bounds {
        if let Some(i) = self.node.find(frame) {
            self.shown_box = shown.bounds(i);
        }
        self.shown_box
    }

    /// What the log calls its node in `frame`: as the frame has it, or by
    /// path when it went.
    fn label(&self, frame: &Frame) -> String {
        match self.node.find(frame) {
            Some(i) => label(frame, i),
            None => format!("{} (gone)", self.node.path),
        }
    }
}

/// A pointer that is down (the mouse's primary button, a finger, a pen)
/// and all it holds, as Chrome keeps it per pointer: what it went down on
/// and shows pressed, what it captured (its drag, its raw touch phases),
/// its gesture, its claim. Nothing of one pointer is another's: a tap or a
/// pan of one leaves every other as it was.
struct Pointer {
    id: PointerId,
    kind: PointerKind,
    /// The node under it as it went down (a contact's: what its long
    /// press and pan start from). `None` once a frame did not hold it: a
    /// node made again with the same key is another, and is not pressed.
    hit: Option<Held>,
    /// The path of the node it went down on, kept once that node has gone
    /// ([`Self::hit`] is then `None`): a contact's tap lifting on what is
    /// left above it clicks there.
    down: Option<NodePath>,
    /// Its hit and the nodes above it show pressed (`:active`).
    pressing: bool,
    /// Where it went down, and where it was last, on screen.
    start: (f32, f32),
    last: (f32, f32),
    /// The node receiving its drag events (a captured pointer).
    drag: Option<Grip<DragEvent>>,
    /// It went down on a control inside the node it drags.
    on_control: bool,
    /// Where its drag last saw it in the dragged node's own space
    /// ([`InputState::local`]). A frame moves the drag only when that
    /// changed (what it drags moved under a resting pointer), not for
    /// every frame that placed the same boxes again.
    seen: Option<(f32, f32)>,
    /// The text input it selects in by dragging.
    selecting: Option<Held>,
    /// A drag handler of its claimed it ([`claim_press`]): its release
    /// clicks nothing.
    claimed: bool,
    /// The node whose `on_touch` takes a contact's raw phases: found once,
    /// as it went down, and kept until it lifts, or until that node is
    /// disabled, made unavailable or goes, which gets `Cancelled` then.
    raw: Option<Grip<TouchEvent>>,
    /// A contact's tap, long press and pan, being recognized.
    gesture: Option<crate::gesture::Press>,
    /// A contact pans: its moves scroll.
    panning: bool,
    /// A contact held into a long press that ran a handler (`on_long_press`,
    /// or the `on_secondary_click` it falls back to): its moves neither pan
    /// nor drag, and its lift is a release ([`crate::Div::on_release`]).
    long: bool,
    /// The container a contact's pan latched to: its own, which no other
    /// contact going down or lifting ends.
    latch: crate::compositor::WheelGesture<Held>,
    /// Its press's number, new for each pointer that goes down: its pan's
    /// gesture identity ([`crate::compositor::WheelStep::gesture`]), an
    /// identity alone, never an order.
    press: u64,
}

/// Where presses take their numbers ([`Pointer::press`]): an identity, so
/// from the source a fork does not roll back (ADR-0028).
static PRESSES: crate::engine::IdKind = crate::engine::IdKind::new("press");

impl Pointer {
    fn new(id: PointerId, kind: PointerKind, at: (f32, f32)) -> Self {
        Self {
            id,
            kind,
            hit: None,
            down: None,
            pressing: false,
            start: at,
            last: at,
            drag: None,
            on_control: false,
            seen: None,
            selecting: None,
            claimed: false,
            raw: None,
            gesture: None,
            panning: false,
            long: false,
            latch: crate::compositor::WheelGesture::default(),
            press: crate::engine::unique_id(&PRESSES),
        }
    }

    /// The contact id its touch events carry.
    fn contact(&self) -> u64 {
        match self.id {
            PointerId::Contact(id) => id,
            PointerId::Mouse => 0,
        }
    }
}

/// Run `f`, a drag handler being given an event; whether it claimed the
/// press ([`claim_press`]). The claim is the pointer's whose event it is:
/// a drag event dispatched inside `f` (nested) claims for its own pointer
/// and leaves the claim `f` made before or after it as it was.
fn claiming(f: impl FnOnce()) -> bool {
    /// Puts the outer claim back however `f` ends, a panic unwinding
    /// through it included.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            CLAIMED.with(|c| c.set(self.0));
        }
    }
    let restore = Restore(CLAIMED.with(|c| c.replace(false)));
    f();
    let mine = CLAIMED.with(std::cell::Cell::get);
    drop(restore);
    mine
}

/// A node's hover handler, told whether the pointer is over it.
type HoverHandler = std::rc::Rc<dyn Fn(bool)>;

/// The window's input state that lives between frames: hover, the pointers
/// down, focus.
#[derive(Default)]
pub struct InputState {
    /// The key being handled is the platform's repeat of a held key.
    repeating: bool,
    pub(crate) state: UiState,
    /// Where the mouse is, while over the window.
    pointer: Option<(f32, f32)>,
    /// The pointers down, each with all it holds.
    pointers: Vec<Pointer>,
    modifiers: Modifiers,
    /// The field an input method is composing in, so its composition goes
    /// when focus leaves it.
    composing: Option<TextField>,
    /// What the screen shows, as last known ([`Self::show`]): input lands
    /// where it shows each node, and a frame compares against it.
    shown: Option<std::sync::Arc<crate::compositor::Presented>>,
    /// Its boxes by node, made once per hit table rather than searched for
    /// each node of each input ([`Shown`]).
    hit_keys: HitKeys,
    /// While one input is routed in another frame than the one on screen
    /// ([`Self::route_in`]): the frame on screen and its boxes by node,
    /// set aside for that dispatch. Hover and the cursor read the frame on
    /// screen alone; only the dispatch reads the one routed in.
    screen: Option<(Option<std::sync::Arc<crate::compositor::Presented>>, HitKeys)>,
    /// The buffer of the boxes by node the last such dispatch made, kept.
    routed_keys: HitKeys,
    /// What the input method's area last worked out of the focused field's
    /// text, kept while the screen shows the same record of it.
    ime_text: Option<ImeText>,
    /// The scroll container the wheel's gesture under way latched to, by
    /// the compositor's rules ([`crate::compositor::WheelGesture`]); a
    /// contact's pan keeps its own ([`Pointer::latch`]).
    scroll_latch: crate::compositor::WheelGesture<Held>,
    /// That gesture's motion as the UI routed it (its recent events, its
    /// lift), handed with its latch ([`Self::hand_gesture`]).
    scroll_motion: crate::compositor::GestureMotion,
    /// When a drag selecting in a text field held past its edge last
    /// scrolled it ([`Self::autoscroll`]).
    autoscrolled: Option<std::time::Duration>,
    /// The wakeup it queued for its next step, cancelled when it stops.
    autoscroll_timer: Option<crate::clock::TimerId>,
    /// The scroll handles the last main frame gave a reach (ADR-0015).
    reaching: Vec<ScrollHandle>,
    /// The frame's scroll containers by key: what a compositor delta finds
    /// its container in ([`Self::take_scroll`]).
    scrollers: ScrollIndex,
    /// The containers the last compositor deltas moved, kept as room.
    scrolled: Vec<usize>,
    /// What placing text fields last said of their scroll, swapped with
    /// the layout's own list each settle ([`LayoutTree::swap_fields`]).
    fields: Vec<crate::layout::FieldSettled>,
    /// The focus and whether it showed when the nodes' [`Interaction`]s
    /// were last written: focus moved since when it differs.
    shown_focus: Option<(Option<FocusId>, bool)>,
    /// The nodes hit testing wrote hovered, pressed, and focused or with
    /// focus within: a change writes the nodes that left the chain and
    /// those that came to it, not the frame.
    hover_marks: Marked,
    press_marks: Marked,
    focus_marks: Marked,
    /// Room for a chain of nodes, kept so a pointer move allocates nothing.
    chain: Vec<usize>,
    was: Vec<usize>,
    /// The hover handlers a change tells, gathered after the change is
    /// written ([`Self::commit_hover`]); kept so a move allocates nothing.
    told: Vec<(HoverHandler, bool)>,
    /// The layers of the frame input goes to (ADR-0024).
    layers: Stack,
    /// The open focus scopes.
    focus: FocusManager,
}

/// Nodes of a frame a flag was written on, by index, and the frame they
/// index: the [`Frame::epoch`] it had then.
#[derive(Default)]
struct Marked {
    epoch: Option<u64>,
    nodes: Vec<usize>,
}

impl Marked {
    /// The nodes marked, as indices of `frame`, into `into`: those kept
    /// when `frame` has not been rebuilt since they were marked, else the
    /// nodes `rest` picks (one walk of the frame, as its rebuild was).
    fn take(&self, frame: &Frame, into: &mut Vec<usize>, rest: impl Fn(usize) -> bool) {
        into.clear();
        if self.epoch == Some(frame.epoch()) {
            into.extend_from_slice(&self.nodes);
        } else if self.epoch.is_some() {
            into.extend((0..frame.nodes.len()).filter(|&i| rest(i)));
        }
    }

    /// Mark `nodes` of `frame`.
    fn mark(&mut self, frame: &Frame, nodes: &[usize]) {
        self.epoch = Some(frame.epoch());
        self.nodes.clear();
        self.nodes.extend_from_slice(nodes);
    }
}

/// Call `f(i, on)` for each node in one of `was` and `now` alone: on when
/// it is in `now`. Both are chains of ancestors, a few nodes each.
fn each_change(was: &[usize], now: &[usize], mut f: impl FnMut(usize, bool)) {
    for &i in was {
        if !now.contains(&i) {
            f(i, false);
        }
    }
    for &i in now {
        if !was.contains(&i) {
            f(i, true);
        }
    }
}

/// [`each_change`] for `was` and `now` sorted, without repeats: one walk
/// of both, however many nodes (every pointer's chain, pressed).
fn each_change_sorted(was: &[usize], now: &[usize], mut f: impl FnMut(usize, bool)) {
    let (mut a, mut b) = (was.iter().peekable(), now.iter().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (Some(&x), Some(&y)) if x == y => {
                a.next();
                b.next();
            }
            (Some(&x), Some(&y)) if x < y => {
                f(x, false);
                a.next();
            }
            (Some(_), Some(&y)) | (None, Some(&y)) => {
                f(y, true);
                b.next();
            }
            (Some(&x), None) => {
                f(x, false);
                a.next();
            }
            (None, None) => break,
        }
    }
}

/// Where an input method types, in window coordinates, and the input it
/// belongs to: the text being composed, or the caret and what is selected on
/// its line, with about three characters either side so the candidate
/// window need not move as the composition grows (Parley's `PlainEditor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImeTarget {
    pub field: FocusId,
    pub area: Bounds,
}

/// What an input did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The frame must be rebuilt (hover, press, focus or scroll changed, or a handler ran).
    pub redraw: bool,
    /// A line for the debug log naming what handled it.
    pub handled_by: Option<String>,
}

impl Outcome {
    fn redraw() -> Self {
        Self {
            redraw: true,
            handled_by: None,
        }
    }

    fn by(what: String) -> Self {
        Self {
            redraw: true,
            handled_by: Some(what),
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            redraw: self.redraw || other.redraw,
            handled_by: self.handled_by.or(other.handled_by),
        }
    }
}

fn label(frame: &Frame, i: usize) -> String {
    frame
        .nodes
        .get(i)
        .map(|n| format!("{} {}", n.path, crate::inspect::describe(n)))
        .unwrap_or_default()
}

pub(crate) mod shown;
use shown::{HitKeys, Shown, ShownField, laid_bounds, laid_content_box, laid_scroll, painted, shown_node};

/// The part of the input method's area a field's shown text alone decides
/// ([`InputState::ime_target`]), kept by the record it was worked out of.
/// A record is shared, unchanged, by every frame that shows the field the
/// same, and a new one is made when its text, caret, selection,
/// composition, box or type changes: so the same record (held weakly, its
/// address never another's) means the same answer, and a 20k-line field
/// that shows unchanged is laid out once, not on every wake. The text
/// settings ([`crate::text::settings::TextSettings`], reloaded as the window regains
/// focus) are not part of the key: they decide only how glyphs rasterize
/// (antialiasing, subpixel order, gamma, stem darkening, read by the
/// renderer alone), never where text or its caret lies.
struct ImeText {
    record: std::sync::Weak<crate::compositor::ShownText>,
    focus: FocusId,
    /// Where its text sits from its box's corner, unscrolled.
    rel: (f32, f32),
    /// Its caret with the composition, or the selection on the caret's
    /// line, in its text.
    caret: Bounds,
}

/// Lays out `field`'s text as it showed and works out [`ImeText::rel`]
/// and [`ImeText::caret`].
fn ime_text_of(text: &mut TextSystem, field: &ShownField) -> ((f32, f32), Bounds) {
    let (content, area) = (&field.content, field.area);
    let it = input_text(
        text,
        content,
        &field.text,
        area,
        field.multiline,
        field.focused,
        (0.0, 0.0),
    );
    let caret = text.caret(content, &it.t, it.width, field.caret);
    let (from, to, composing) = match field.preedit {
        Some((a, b)) => (a, b, true),
        None => (field.selection.0, field.selection.1, false),
    };
    let mut bounds = Bounds {
        x: caret.x,
        y: caret.y,
        width: 0.0,
        height: caret.height,
    };
    for b in text.selection(&**content, &it.t, it.width, from, to) {
        // All of a composition; of a selection, its part on the caret's line.
        if composing || (b.y - caret.y).abs() < 0.5 {
            bounds = union(bounds, b);
        }
    }
    ((it.origin.0 - area.x, it.origin.1 - area.y), bounds)
}

#[cfg(test)]
crate::engine::engine_local! {
    /// Frame nodes the input side's indexes looked at on this thread
    /// ([`ScrollIndex`], the layer stack's refresh).
    [Fork] static SCROLL_INDEX_VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Frame nodes the input side's indexes looked at on this thread so far.
#[cfg(test)]
pub(crate) fn index_visits() -> u64 {
    SCROLL_INDEX_VISITS.with(std::cell::Cell::get)
}

/// One frame node looked at by an index the input side keeps of a frame
/// ([`ScrollIndex`], [`crate::layer::Stack`]): counted in tests, so a
/// cost test sees every walk on its path.
pub(crate) fn visited() {
    #[cfg(test)]
    SCROLL_INDEX_VISITS.with(|n| n.set(n.get().saturating_add(1)));
}

/// A frame's scroll containers (those with an offset: containers and
/// text fields) by node key, sorted: a compositor delta finds its
/// container by a binary search. Kept up to date from what the frame says
/// changed since it last read it ([`Frame::changes`]): a frame written in
/// place costs what was written, one rebuilt a walk of its nodes (as the
/// rebuild walked them), and taking in a delta costs no walk at all.
#[derive(Default)]
struct ScrollIndex {
    read: crate::layout::ReadAt,
    by_key: Vec<(crate::compositor::NodeKey, usize)>,
}

impl ScrollIndex {
    /// Bring it up to `frame`.
    fn refresh(&mut self, frame: &Frame) {
        use crate::compositor::NodeKey;
        match frame.changes.since(&mut self.read) {
            crate::layout::Since::Logged(log) => {
                for c in log {
                    let Ok(i) = usize::try_from(c.index) else { continue };
                    visited();
                    // Written in place: the node is the one it was (its
                    // path too), only whether it scrolls may have changed.
                    let Some(n) = frame.nodes.get(i) else { continue };
                    let entry = (NodeKey::of(&n.path), i);
                    match (self.by_key.binary_search(&entry), laid_scroll(n).is_some()) {
                        (Err(at), true) => self.by_key.insert(at, entry),
                        (Ok(at), false) => {
                            self.by_key.remove(at);
                        }
                        _ => {}
                    }
                }
            }
            crate::layout::Since::Rebuilt | crate::layout::Since::FellBehind => {
                self.by_key.clear();
                for (i, n) in frame.nodes.iter().enumerate() {
                    visited();
                    if laid_scroll(n).is_some() {
                        self.by_key.push((NodeKey::of(&n.path), i));
                    }
                }
                self.by_key.sort_unstable();
            }
        }
    }

    /// The container of `frame` keyed `key` made by `owner`: the one a
    /// delta naming both scrolled, by identity (one another instance
    /// replaced at its path while the delta was on its way is another).
    fn find(
        &self,
        frame: &Frame,
        key: crate::compositor::NodeKey,
        owner: Option<crate::reactive::OwnerId>,
    ) -> Option<usize> {
        let from = self.by_key.partition_point(|e| e.0 < key);
        self.by_key
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|e| e.0 == key)
            .map(|e| e.1)
            .find(|&i| {
                visited();
                owner_of(frame, i) == owner
            })
    }
}

/// Every node of a frame by its key, sorted: a wheel the compositor hands
/// on finds the node it was over ([`crate::compositor::HitTarget`]) by a
/// binary search, not a walk of the frame. Kept from what the frame says
/// changed since it last read it, as [`ScrollIndex`] is: a write in place
/// keeps every node's path, so it costs what was written; a rebuild costs
/// a walk of the nodes, once, on the next lookup. The last node found is
/// kept with it, so the rest of a gesture handed on over the same node of
/// the same build looks nothing up.
#[derive(Default)]
struct NodeIndex {
    read: crate::layout::ReadAt,
    by_key: Vec<(crate::compositor::NodeKey, usize)>,
    last: Option<(crate::compositor::HitTarget, usize)>,
}

impl crate::engine::parts_seal::Sealed for NodeIndex {}
impl crate::engine::Parts for NodeIndex {
    /// The engine's own index: no user value in it.
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        out.push(Box::new(self));
    }
}

impl NodeIndex {
    /// The node of `frame` that `target` names, by identity: its key and
    /// the instance that made it.
    fn find(&mut self, frame: &Frame, target: crate::compositor::HitTarget) -> Option<usize> {
        self.refresh(frame);
        if let Some((t, i)) = self.last
            && t == target
        {
            return Some(i);
        }
        let found = self.keyed(target.key, |i| owner_of(frame, i) == target.owner);
        self.last = found.map(|i| (target, i));
        found
    }

    /// Brought up to `frame` (a rebuild since is read once, a write in
    /// place costs what was written).
    fn refresh(&mut self, frame: &Frame) {
        use crate::compositor::NodeKey;
        match frame.changes.since(&mut self.read) {
            crate::layout::Since::Logged(log) => {
                for c in log {
                    visited();
                    // Written in place: its path is the one it had, but
                    // what made it is asked again.
                    if self
                        .last
                        .is_some_and(|(_, i)| usize::try_from(c.index).is_ok_and(|c| c == i))
                    {
                        self.last = None;
                    }
                }
            }
            crate::layout::Since::Rebuilt | crate::layout::Since::FellBehind => {
                self.last = None;
                self.by_key.clear();
                for (i, n) in frame.nodes.iter().enumerate() {
                    visited();
                    self.by_key.push((NodeKey::of(&n.path), i));
                }
                self.by_key.sort_unstable();
            }
        }
    }

    /// The first node keyed `key` that `accept` takes.
    fn keyed(&self, key: crate::compositor::NodeKey, accept: impl Fn(usize) -> bool) -> Option<usize> {
        let from = self.by_key.partition_point(|e| e.0 < key);
        self.by_key
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|e| e.0 == key)
            .map(|e| e.1)
            .find(|&i| {
                visited();
                accept(i)
            })
    }
}

crate::engine::engine_local! {
    /// The last frame's nodes by key, for the input side's lookups by
    /// identity ([`Held::find`], a wheel handed on): one index per thread,
    /// kept up to date from the frame's change log. A cache: a cold frame
    /// starts its own.
    [Fresh] static NODES: std::cell::RefCell<NodeIndex> = std::cell::RefCell::new(NodeIndex::default());
}

/// Who holds a scroll container's offset, each by one rule: code, through
/// its [`ScrollHandle`]; a text field, through its view
/// ([`TextField::scroll_generation`]); or else the window's interaction
/// state, by the container's identity. Each keeps a generation the UI's
/// own writes bump and the compositor's scroll taken in does not.
enum Holder<'f> {
    Handle(&'f ScrollHandle),
    Field(TextField),
    State(&'f NodePath, Option<crate::reactive::OwnerId>),
}

impl<'f> Holder<'f> {
    /// Who holds the offset of `frame`'s node `i` (a scroll container or a
    /// text field).
    fn of(frame: &'f Frame, i: usize) -> Option<Self> {
        let b = frame.behaviour(i);
        // A text input takes no scroll handle ([`crate::Div::track_scroll`]):
        // its field's view is the one owner of its offset.
        if let Some(f) = b.and_then(|b| b.field) {
            return Some(Self::Field(f));
        }
        if let Some(h) = b.and_then(|b| b.scroll.as_ref()) {
            return Some(Self::Handle(h));
        }
        frame.nodes.get(i).map(|n| Self::State(&n.path, owner_of(frame, i)))
    }

    fn id(path: &NodePath, owner: Option<crate::reactive::OwnerId>) -> crate::layout::ScrollId {
        (path.clone(), owner)
    }

    /// How many times the UI jumped it, as of now (not as last laid out):
    /// what the compositor's fling is compared with as it is taken in.
    fn generation(&self, state: &UiState) -> u64 {
        match self {
            Self::Handle(h) => h.generation(),
            Self::Field(f) => f.scroll_generation(),
            Self::State(p, o) => state
                .scroll_generation
                .get(&Self::id(p, *o))
                .copied()
                .unwrap_or_default(),
        }
    }

    /// Move it by `delta`: the compositor's scroll taken in, or the UI's
    /// own wheel (`wheel`, counted as one step on a handle, its overlay
    /// scrollbar shown against an edge too). Added to where the UI holds it
    /// now, what it took in or wrote since `laid` (its last layout)
    /// included, clamped to that layout's range (ADR-0032, I2 and I5): a
    /// displacement, never a jump, so no generation starts and what the
    /// compositor moves by itself goes on. Whether it moved. Exactly
    /// ([`crate::compositor::motion`]): what is added is what it moves,
    /// up to the range's end.
    fn add(&self, state: &mut UiState, laid: crate::layout::Scroll, delta: motion::Exact, wheel: bool) -> bool {
        let clamp = |at: motion::Exact| motion::clamp2(motion::add2(at, delta), laid.end);
        match self {
            Self::Handle(h) if wheel => {
                let was = h.offset_now();
                h.wheel(delta);
                h.offset_now() != was
            }
            Self::Handle(h) => h.add(delta),
            Self::Field(f) => {
                let (was, next) = (f.pos(), clamp(f.pos()));
                f.synced(next);
                next != was
            }
            Self::State(p, o) => {
                let id = Self::id(p, *o);
                let was = state.scroll.get(&id).copied().unwrap_or(laid.pos);
                let next = clamp(was);
                if next == was {
                    return false;
                }
                state.set_scroll(id, next);
                true
            }
        }
    }
}

/// Whether `path` is the node at `down` or above it: its segments begin
/// `down`'s. While the node at `down` is `gone`, only those above it are:
/// one made again with its key is another.
fn covers(path: &NodePath, down: &NodePath, gone: bool) -> bool {
    let (a, b) = (&path.0[..], &down.0[..]);
    let within = if gone { a.len() < b.len() } else { a.len() <= b.len() };
    within && b.get(..a.len()) == Some(a)
}

/// What a press that went down on node `from` and came up on node `h`
/// clicks, as Chrome's click target: the nearest common ancestor of the
/// two that has a click handler, if the press reaches it, no node from
/// `from` up to that ancestor being disabled or unavailable. Found up the
/// two chains, whatever the size of the tree.
fn click_target(frame: &Frame, from: usize, h: usize) -> Option<usize> {
    let down = &frame.nodes.get(from)?.path.0[..];
    let common = frame.ancestors(h).find(|&i| {
        frame
            .nodes
            .get(i)
            .is_some_and(|n| down.get(..n.path.0.len()) == Some(&n.path.0[..]))
    })?;
    if !Reach::Activation.reaches(frame, from, common) {
        return None;
    }
    Reach::Activation.target(frame, common, |b| b.handlers.click.is_some())
}

/// The topmost node under `(x, y)` that takes the pointer in `frame`
/// before any frame of it was presented: by the one hit rule
/// ([`crate::clip::hit_in`]) on the frame as paint makes it, no layer
/// drawn ([`painted`]); nothing below the top modal layer does (ADR-0024).
#[must_use]
pub fn hit(frame: &Frame, x: f32, y: f32) -> Option<usize> {
    let laid = painted(frame);
    shown_node(frame, &laid, (x, y), Stack::of(frame).floor, false)
}

/// The node of `frame` under `(x, y)` on screen while `presented` shows:
/// the devtools' pick, by the rule a click lands by ([`crate::clip::hit_in`]
/// on what was presented, its node found in `frame` by path and instance).
/// Without a frame presented, the frame as paint makes it ([`hit`]).
#[must_use]
pub fn hit_presented(frame: &Frame, presented: Option<&crate::compositor::Presented>, x: f32, y: f32) -> Option<usize> {
    match presented {
        Some(p) => shown_node(frame, p, (x, y), Stack::of(frame).floor, false),
        None => hit(frame, x, y),
    }
}

/// The node `frame` has for `target`: the one at its path that the same
/// component instance made, found by a binary search of the frame's key
/// index ([`NODES`]), not a walk of the frame: a click, a hover and each
/// candidate under a modal layer cost a lookup, at any frame size. One
/// another instance made at that path since is another node: input meant
/// for the one shown is not delivered to it (as Blink never reuses a hit
/// target across a change of the document, `HitTestCache`).
fn node_of(frame: &Frame, target: crate::compositor::HitTarget) -> Option<usize> {
    NODES.with(|n| n.borrow_mut().find(frame, target))
}

#[cfg(test)]
impl InputState {
    /// An input state that took `frame` in as `shown` presents it, as the
    /// window's input does before it routes a point.
    pub(crate) fn showing(frame: &Frame, shown: std::sync::Arc<crate::compositor::Presented>) -> Self {
        let mut s = Self::default();
        s.show(Some(shown));
        s.take_in(frame);
        s
    }

    /// The node input at `(x, y)` lands on ([`Self::locate`]).
    pub(crate) fn hit_shown(&self, frame: &Frame, x: f32, y: f32) -> Option<usize> {
        self.locate(frame, x, y).0
    }
}

impl InputState {
    #[must_use]
    pub fn state(&self) -> &UiState {
        &self.state
    }

    /// The layers open, bottom first: the node of each in `frame`.
    pub(crate) fn layer_nodes(&self) -> impl Iterator<Item = usize> + '_ {
        self.layers.entries.iter().map(|e| e.node)
    }

    /// The screen shows `presented` now (`None`: the frame as laid out,
    /// the devtools'): input that comes next lands where it shows each
    /// node, and each frame after is compared against it, so a press, the
    /// frames after and the moves after all see a node at one place.
    pub fn show(&mut self, presented: Option<std::sync::Arc<crate::compositor::Presented>>) {
        let same = match (&self.shown, &presented) {
            (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same && let Some(p) = &presented {
            self.hit_keys.fill(&p.hits);
        }
        self.shown = presented;
    }

    /// Route the input dispatched next in `routed`, the frame it came under
    /// (ADR-0015), until [`Self::routed`]: one frame per question. The
    /// frame on screen ([`Self::show`]) is set aside meanwhile and is what
    /// hover and the cursor read after, so an input routed in an older
    /// frame never rolls them back to it (a key, a modifier, a press where
    /// the pointer rests).
    pub(crate) fn route_in(&mut self, routed: std::sync::Arc<crate::compositor::Presented>) {
        debug_assert!(self.screen.is_none(), "one dispatch at a time");
        if self.shown.as_ref().is_some_and(|s| std::sync::Arc::ptr_eq(s, &routed)) {
            return;
        }
        let keys = std::mem::replace(&mut self.hit_keys, std::mem::take(&mut self.routed_keys));
        self.screen = Some((self.shown.take(), keys));
        self.show(Some(routed));
    }

    /// The dispatch [`Self::route_in`] began is over. A move or a leave
    /// (`hovers`) worked hover out where it put the pointer, in the frame
    /// it was routed in: that frame is what hover was last worked out in,
    /// until the frame on screen is checked against it. Anything else
    /// leaves the frame on screen as it was.
    pub(crate) fn routed(&mut self, hovers: bool) {
        let Some((shown, keys)) = self.screen.take() else {
            return;
        };
        if hovers {
            self.routed_keys = keys;
        } else {
            self.routed_keys = std::mem::replace(&mut self.hit_keys, keys);
            self.shown = shown;
        }
    }

    /// Read `frame` before input goes to it: its layers, when it changed.
    /// Where the screen shows each of its nodes is the frame shown's
    /// ([`Self::show`], [`Shown`]).
    fn take_in(&mut self, frame: &Frame) {
        self.layers.refresh(frame);
    }

    /// What the screen showed, for `frame`'s nodes ([`Shown`]): the frame
    /// input is routed in during a dispatch, the frame on screen
    /// otherwise; with none shown to this state (the devtools' frame as
    /// laid out; a pipeline always shows one), the frame laid out. The
    /// only way input reads geometry.
    fn shown<'s>(&'s self, frame: &'s Frame) -> Shown<'s> {
        Shown::new(frame, self.shown.as_deref(), &self.hit_keys)
    }

    /// The node under `(x, y)` on screen and the point where it lies in
    /// it, above the top modal layer of the frame last taken in.
    fn locate(&self, frame: &Frame, x: f32, y: f32) -> (Option<usize>, f32, f32) {
        let shown = self.shown(frame);
        match shown.hit(self.layers.floor, false, (x, y)) {
            Some(i) => {
                let (lx, ly) = shown.to_node(i, (x, y));
                (Some(i), lx, ly)
            }
            None => (None, x, y),
        }
    }

    /// The frame shown, when input is routed through one.
    #[must_use]
    pub(crate) fn presented(&self) -> Option<&crate::compositor::Presented> {
        self.shown.as_deref()
    }

    /// How far each container moving may scroll before the commit after
    /// this frame's shows (a main frame brought it, ADR-0015): what a
    /// virtual list builds beyond its viewport. A reach holds for one
    /// frame: a container no longer moving has none. Whether any was set.
    pub fn reach(&mut self, frame: &Frame, reach: &[crate::compositor::ScrollDelta]) -> bool {
        for h in self.reaching.drain(..) {
            h.set_reach((0.0, 0.0));
        }
        if reach.is_empty() {
            return false;
        }
        self.scrollers.refresh(frame);
        for d in reach {
            if let Some(i) = self.scrollers.find(frame, d.key, d.owner)
                && let Some(Holder::Handle(h)) = Holder::of(frame, i)
                && d.reach != (0.0, 0.0)
            {
                h.set_reach(d.reach);
                self.reaching.push(*h);
            }
        }
        !self.reaching.is_empty()
    }

    /// Take in what the compositor moved (ADR-0032, I2: before an input is
    /// handled, and before a frame is built): each container's offset
    /// moves by its delta, added to where the UI holds it now, once, in the
    /// order taken; nothing is refused, so a jump the UI made since stays
    /// and what the compositor moved after it lands on top. Its wheel steps
    /// are counted, each once. Whether any moved or counted.
    pub fn take_scroll(&mut self, frame: &Frame, deltas: &[crate::compositor::ScrollDelta]) -> bool {
        if deltas.is_empty() {
            return false;
        }
        // Each delta finds its container by key, not by a walk of the
        // frame: what it costs is the deltas', whatever the frame's size.
        self.scrollers.refresh(frame);
        let mut moved = false;
        let mut scrolled = std::mem::take(&mut self.scrolled);
        scrolled.clear();
        for d in deltas {
            // By identity: a container another instance replaced at its
            // path while the delta was on its way is another.
            let Some(i) = self.scrollers.find(frame, d.key, d.owner) else {
                continue;
            };
            let (Some(s), Some(holder)) = (frame.nodes.get(i).and_then(laid_scroll), Holder::of(frame, i)) else {
                continue;
            };
            // Each wheel step the compositor took counts once, moved or
            // held against its edge (its overlay scrollbar shows).
            if let Holder::Handle(h) = &holder
                && d.steps > 0
            {
                h.wheeled_by(d.steps);
                moved = true;
            }
            // The compositor's fling, moved before a jump the UI made since,
            // is left out: the jump stops it (cc's clobbering commit). The
            // reader's own input is taken whole (ADR-0032 I4).
            let delta = if d.synthetic != (0, 0) && holder.generation(&self.state) != d.generation {
                motion::sub2(d.delta, d.synthetic)
            } else {
                d.delta
            };
            if holder.add(&mut self.state, s, delta, false) {
                moved = true;
                scrolled.push(i);
            }
        }
        if !scrolled.is_empty() {
            // What a scroll dismisses is read from the layers alone.
            self.layers.refresh(frame);
            for &i in &scrolled {
                self.dismiss_scrolled(frame, i);
            }
        }
        self.scrolled = scrolled;
        moved
    }

    /// Scroll the container at `i` by `delta` from where it was laid out,
    /// exactly ([`crate::compositor::motion`]), as the UI's own scroll does
    /// (its wheel, a drag past its edge, a node brought into view): added
    /// to where the UI holds it now ([`Holder::add`]), so what the
    /// compositor moved and the UI took in since stays, and what it moves
    /// by itself goes on.
    fn scroll_by(&mut self, frame: &Frame, i: usize, delta: motion::Exact) {
        if let (Some(holder), Some(s)) = (Holder::of(frame, i), frame.nodes.get(i).and_then(laid_scroll)) {
            holder.add(&mut self.state, s, delta, true);
        }
    }

    /// The device pixels per logical pixel the next frame is drawn at.
    pub fn set_scale(&mut self, scale: f32) {
        self.state.scale = scale;
    }

    pub fn set_modifiers(&mut self, m: Modifiers) {
        self.modifiers = m;
    }

    /// Refresh the focus part of the state from the focus signals, and,
    /// when focus moved or came to show or hide since, the focus flags of
    /// the [`Interaction`]s of the nodes of `tree`'s frame (the frame the
    /// next is built over; the layout tree writes those of the nodes it
    /// builds): the nodes that held focus or focus within and those that
    /// hold it now.
    pub fn sync_focus(&mut self, tree: &LayoutTree) {
        self.state.focused = focus::focused();
        self.state.focus_visible = focus::focus_visible();
        // Where the focused node lies, as the last pass found it
        // ([`LayoutTree::focus_location`]): a pass reads it before it
        // descends, so an ancestor built before the focused node (a cold
        // frame's are all built so) is told focus is within it as the live
        // one is. A pass that finds it elsewhere is followed by another
        // ([`LayoutTree::focus_settled`]).
        let frame = tree.frame();
        let found = self.state.focused.and_then(|id| tree.focus_location(id));
        let at = found.map(|(i, _, _)| i);
        let focus_at = found.map(|(_, p, l)| (p, l));
        let moved = self.state.focus_at.as_ref().map(|(p, l)| (p, *l)) != focus_at;
        if moved {
            self.state.focus_at = focus_at.map(|(p, l)| (p.clone(), l));
        }
        let now = (self.state.focused, self.state.focus_visible);
        if self.shown_focus == Some(now) && !moved {
            return;
        }
        self.shown_focus = Some(now);
        let (mut chain, mut was) = (std::mem::take(&mut self.chain), std::mem::take(&mut self.was));
        chain.clear();
        chain.extend(at.into_iter().flat_map(|at| frame.ancestors(at)));
        // Rebuilt since: every node with an `Interaction` may hold focus.
        self.focus_marks.take(frame, &mut was, |i| {
            frame.behaviour(i).is_some_and(|b| b.interaction.is_some())
        });
        let keyboard = self.state.focus_visible;
        let write = |i: usize, c: &Commit| {
            let (Some(b), Some(node)) = (frame.behaviour(i), frame.nodes.get(i)) else {
                return;
            };
            let Some(it) = &b.interaction else { return };
            let (focused, visible, within) = focus_flags(
                &node.path,
                node.layer,
                at == Some(i),
                b.field.is_some(),
                focus_at,
                keyboard,
            );
            it.set_focus(focused, visible, within, c);
        };
        // One step: effects reading the flags run once all are written,
        // and the marks with them (the hover transaction, ADR-0015).
        let marks = &mut self.focus_marks;
        crate::engine::commit_here(|c| {
            for &i in &was {
                if !chain.contains(&i) {
                    write(i, c);
                }
            }
            // Whether focus shows may have changed without it moving.
            for &i in &chain {
                write(i, c);
            }
            marks.mark(frame, &chain);
            // Kept inside: an effect that panics as the batch ends finds
            // the chain and the marks as one.
            (self.chain, self.was) = (chain, was);
        });
    }

    /// The chain of nodes from `h` up, in `self.chain`.
    fn chain_from(&mut self, frame: &Frame, h: Option<usize>) {
        self.chain.clear();
        self.chain.extend(h.into_iter().flat_map(|h| frame.ancestors(h)));
    }

    /// Whether `self.chain` holds other nodes than `paths` names.
    fn chain_differs(&self, frame: &Frame, marks: &Marked, paths: &HashSet<NodePath>) -> bool {
        if marks.epoch == Some(frame.epoch()) {
            return marks.nodes != self.chain;
        }
        self.chain.len() != paths.len()
            || self
                .chain
                .iter()
                .any(|&i| frame.nodes.get(i).is_none_or(|n| !paths.contains(&n.path)))
    }

    /// `paths` as the nodes of `self.chain` (kept, so a change allocates
    /// nothing once it has held as many).
    fn set_paths(frame: &Frame, chain: &[usize], paths: &mut HashSet<NodePath>) {
        paths.clear();
        paths.extend(chain.iter().filter_map(|&i| frame.nodes.get(i).map(|n| n.path.clone())));
    }

    /// Write the nodes pressed: those any pointer presses (its hit and the
    /// nodes above it), as a node is `:active` while one pointer is down on
    /// it whatever the others do. The flags of those that left them or came
    /// to them, and the paths the next frame is built with.
    fn write_pressed(&mut self, frame: &Frame) {
        self.chain.clear();
        for p in self.pointers.iter().filter(|p| p.pressing) {
            let Some(h) = p.hit.as_ref().and_then(|h| h.find(frame)) else {
                continue;
            };
            self.chain.extend(frame.ancestors(h));
        }
        // Sorted, once each: the nodes marked before are too (kept so, or
        // taken in frame order), so the change is one walk of both.
        self.chain.sort_unstable();
        self.chain.dedup();
        let mut was = std::mem::take(&mut self.was);
        let active = &self.state.active;
        self.press_marks.take(frame, &mut was, |i| {
            frame.nodes.get(i).is_some_and(|n| active.contains(&n.path))
        });
        // One step: the flags, the paths and the marks, effects reading
        // the flags run after all of them (the hover transaction, ADR-0015).
        crate::engine::commit_here(|c| {
            each_change_sorted(&was, &self.chain, |i, on| {
                if let Some(b) = frame.behaviour(i)
                    && let Some(it) = &b.interaction
                {
                    it.set_pressed(on && !b.disabled, c);
                }
            });
            Self::set_paths(frame, &self.chain, &mut self.state.active);
            self.press_marks.mark(frame, &self.chain);
            self.was = was;
        });
    }

    /// The pointer's position, if it is over the window.
    #[must_use]
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    /// The cursor the pointer should show. While a press drags a node, the
    /// cursor of that node (or the nearest ancestor with one) shows
    /// wherever the pointer is, as a captured pointer keeps it: a splitter
    /// dragged past its handle keeps its resize cursor. Otherwise the one
    /// the frame on screen shows under the pointer ([`Shown::cursor_at`]),
    /// a value of that frame: a node removed since shows its cursor until
    /// a frame without it does.
    #[must_use]
    pub fn cursor(&self, frame: &Frame) -> Cursor {
        let shown = self.shown(frame);
        if let Some(i) = self
            .pointers
            .iter()
            .find(|p| p.id == PointerId::Mouse)
            .and_then(|p| p.drag.as_ref())
            .and_then(|g| g.node.find(frame))
            && let Some(c) = shown.cursor_of(i)
        {
            return c;
        }
        self.pointer.map_or(Cursor::Default, |p| shown.cursor_at(p))
    }

    /// The pointer is at `(x, y)`, from input that is routed nowhere (made
    /// under a frame no longer kept): its place is taken, nothing is done,
    /// and the next hover check works it out there ([`Self::rehover`]).
    pub(crate) fn pointer_at(&mut self, x: f32, y: f32) {
        self.pointer = Some((x, y));
        track_pointer(self.pointer);
    }

    /// Re-run hover against a new frame (the pointer has not moved but the
    /// boxes under it may have), and move each drag whose node moved under
    /// its resting pointer.
    pub fn rehover(&mut self, frame: &Frame, text: &mut TextSystem) -> Outcome {
        match self.pointer {
            Some((x, y)) => self.hover_at(frame, text, x, y, false),
            None => {
                self.take_in(frame);
                self.cancel_lost(frame).merge(self.follow_drags(frame, None))
            }
        }
    }

    /// The pointer moved to `(x, y)`: the user acts by pointer now.
    pub fn pointer_moved(&mut self, frame: &Frame, text: &mut TextSystem, x: f32, y: f32) -> Outcome {
        acts_by(Modality::Pointer);
        self.hover_at(frame, text, x, y, true)
    }

    /// Hover at `(x, y)`, where the mouse `moved` to or (a rehover) rests.
    fn hover_at(&mut self, frame: &Frame, text: &mut TextSystem, x: f32, y: f32, moved: bool) -> Outcome {
        self.take_in(frame);
        let mut out = self.cancel_lost(frame);
        self.pointer = Some((x, y));
        track_pointer(self.pointer);
        if let Some(p) = self.pointers.iter_mut().find(|p| p.id == PointerId::Mouse) {
            p.last = (x, y);
        }
        let (h, ..) = self.locate(frame, x, y);
        self.chain_from(frame, h);
        if self.chain_differs(frame, &self.hover_marks, &self.state.hovered) {
            let mut was = std::mem::take(&mut self.was);
            let hovered = &self.state.hovered;
            self.hover_marks.take(frame, &mut was, |i| {
                frame.nodes.get(i).is_some_and(|n| hovered.contains(&n.path))
            });
            self.commit_hover(frame, was);
            self.tell_hover();
            out = out.merge(Outcome::redraw());
        } else {
            self.hover_marks.mark(frame, &self.chain);
        }
        // Only a move the pointer made is a pointer move, as Chrome's
        // `pointermove`: boxes moving under a resting pointer change what
        // it hovers (above), nothing more.
        // Every handler up the chain is told, though one panics (the
        // dispatch keeps its panic for its end), as Chrome still
        // dispatches to the next.
        if moved && let Some(h) = h {
            let shown = self.shown(frame);
            for i in frame.ancestors(h) {
                if let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.pointer.clone()) {
                    let ((x, y), bounds) = shown.event(i, (x, y));
                    f(&crate::events::PointerEvent { x, y, bounds });
                }
            }
        }
        // The mouse's selection follows it (lost since: stopped above).
        let selecting = self
            .pointers
            .iter()
            .find(|p| p.id == PointerId::Mouse)
            .and_then(|p| p.selecting.as_ref())
            .and_then(|s| s.find(frame));
        // Only as it moves, as Chrome extends a selection: boxes moving under
        // a resting pointer (a field's focus frame expanding its text)
        // extend nothing.
        if moved && let Some(i) = selecting {
            let (x, y) = self.shown(frame).to_node(i, (x, y));
            self.place_caret(frame, text, i, x, y, true);
            out = out.merge(Outcome::redraw());
        }
        out.merge(self.follow_drags(frame, moved.then_some(PointerId::Mouse)))
    }

    /// Move each pointer's drag to where the pointer is in the dragged
    /// node's own space as this frame has it, when that changed since its
    /// drag last saw it (the node moved, scrolled, scaled or turned under a
    /// resting pointer), or when the pointer itself `moved`. Its box and
    /// the point are read off this frame, every time.
    fn follow_drags(&mut self, frame: &Frame, moved: Option<PointerId>) -> Outcome {
        let mut out = Outcome::default();
        for k in 0..self.pointers.len() {
            let Some(p) = self.pointers.get_mut(k) else { break };
            let (id, kind, on_control, at) = (p.id, p.kind, p.on_control, p.last);
            // Lost since (gone or disabled): cancelled before this.
            let shown = Shown::new(frame, self.shown.as_deref(), &self.hit_keys);
            let Some((i, f, bounds)) = p.drag.as_mut().and_then(|g| {
                let i = g.live(frame, &shown, drag_of)?;
                Some((i, std::rc::Rc::clone(&g.handler), g.shown_box))
            }) else {
                continue;
            };
            // Under a scale of 0 it lies nowhere in the node: no frame
            // moves it until it shows again.
            let local = shown.local(i, at);
            let Some(p) = self.pointers.get_mut(k) else { break };
            let seen = std::mem::replace(&mut p.seen, local);
            if moved == Some(id) || (local.is_some() && seen != local) {
                let (x, y) = self.shown(frame).to_node(i, at);
                let ev = DragEvent {
                    x,
                    y,
                    bounds,
                    phase: DragPhase::Move,
                    pointer: id,
                    kind,
                    on_control,
                };
                if claiming(|| f(&ev))
                    && let Some(p) = self.pointers.get_mut(k)
                {
                    p.claimed = true;
                }
                out = out.merge(Outcome::redraw());
            }
        }
        out
    }

    pub fn pointer_left(&mut self, frame: &Frame) -> Outcome {
        self.pointer = None;
        track_pointer(None);
        if self.state.hovered.is_empty() {
            return Outcome::default();
        }
        let mut was = std::mem::take(&mut self.was);
        let hovered = &self.state.hovered;
        self.hover_marks.take(frame, &mut was, |i| {
            frame.nodes.get(i).is_some_and(|n| hovered.contains(&n.path))
        });
        self.chain.clear();
        self.commit_hover(frame, was);
        self.tell_hover();
        Outcome::redraw()
    }

    /// Hover moves from the nodes `was` to [`Self::chain`]: everything it
    /// keeps is written here, as one step, before any handler runs — the
    /// nodes' hovered flags, the hovered paths, the marks the next change
    /// is found by — and the handlers that change tells are gathered
    /// ([`Self::told`]) for [`Self::tell_hover`] to run after. The one
    /// rule of hover (ADR-0015): no user code runs between steps that must
    /// agree, so a handler that panics leaves hover whole, as it was
    /// written, and the next check (the key it is worked out by,
    /// `app::Pipeline::checked`, is written by its caller before this too)
    /// starts from it.
    fn commit_hover(&mut self, frame: &Frame, was: Vec<usize>) {
        let mut told = std::mem::take(&mut self.told);
        told.clear();
        // A flag is a signal: writing it runs the effects reading it. In
        // one batch, they run once every write below is done, so none sees
        // part of the chain, and one that panics finds hover whole.
        crate::engine::commit_here(|c| {
            each_change(&was, &self.chain, |i, on| {
                let Some(b) = frame.behaviour(i) else { return };
                if let Some(it) = &b.interaction {
                    it.set_hovered(on && !b.disabled, c);
                }
                if let Some(f) = &b.handlers.hover {
                    told.push((std::rc::Rc::clone(f), on));
                }
            });
            Self::set_paths(frame, &self.chain, &mut self.state.hovered);
            self.hover_marks.mark(frame, &self.chain);
            self.was = was;
            self.told = told;
        });
    }

    /// Run the hover handlers [`Self::commit_hover`] gathered, in order: the
    /// state they may read is written already. Every one is told, though
    /// one panics: inside the dispatch ([`crate::engine::dispatch`]) a
    /// handler's panic is kept for its end, as Chrome reports a listener's
    /// exception and still dispatches to the next element.
    fn tell_hover(&mut self) {
        let told = std::mem::take(&mut self.told);
        for (f, on) in &told {
            f(*on);
        }
        self.told = told;
    }

    /// Whether `i` is `ancestor` or below it.
    fn within(frame: &Frame, i: usize, ancestor: usize) -> bool {
        frame.ancestors(i).any(|a| a == ancestor)
    }

    /// Ask every layer that takes input and that a press at `h` is outside
    /// of to close, top first: at or above the top modal layer, as the
    /// frame was when the press came (a layer its handlers open is not
    /// asked). What they answered.
    fn dismiss_outside(&mut self, frame: &Frame, h: Option<usize>, press: PointerOutside) -> Outcome {
        self.dismiss_outside_closing(frame, h, press).0
    }

    /// [`Self::dismiss_outside`], and whether the top modal layer answered
    /// that it closes.
    fn dismiss_outside_closing(&mut self, frame: &Frame, h: Option<usize>, press: PointerOutside) -> (Outcome, bool) {
        let top_modal = self
            .layers
            .modal
            .and_then(|m| self.layers.entries.get(m))
            .map(|e| e.node);
        let mut closed_modal = false;
        let asked: Vec<_> = self
            .layers
            .live()
            .filter(|e| e.spec.policy.pointer_outside && h.is_none_or(|h| !self.layers.inside(frame, e, h)))
            .map(|e| (std::rc::Rc::clone(&e.spec), e.node))
            .collect();
        let mut out = Outcome::default();
        // Only a primary press, without Control (a secondary click on a
        // Mac), outside a modal layer gives focus back as it closes it
        // (reka's dialogs, popovers and menus alike).
        let gives_back = press.button == PointerButton::Primary && !press.modifiers.control;
        for (spec, node) in asked {
            if let Some(answer) = spec.ask(&DismissReason::PointerOutside(press)) {
                if answer == Dismiss::Close && !(spec.modal && gives_back) {
                    self.focus.pressed_outside(frame, node);
                }
                closed_modal |= answer == Dismiss::Close && Some(node) == top_modal;
                out = out.merge(Outcome::by(format!(
                    "{:?} press outside ({:.0},{:.0}) -> {answer:?} {}",
                    press.button,
                    press.x,
                    press.y,
                    label(frame, node)
                )));
            }
        }
        (out, closed_modal)
    }

    /// Ask the layers whose owner is inside scroll container `i` and that
    /// take scrolls to close (a tooltip whose trigger scrolled away).
    fn dismiss_scrolled(&mut self, frame: &Frame, i: usize) -> Outcome {
        let asked: Vec<_> = self
            .layers
            .live()
            .filter(|e| e.spec.policy.scroll && e.owner.is_some_and(|o| Self::within(frame, o, i)))
            .map(|e| std::rc::Rc::clone(&e.spec))
            .collect();
        let mut out = Outcome::default();
        for spec in asked {
            if spec.ask(&DismissReason::Scroll).is_some() {
                out = out.merge(Outcome::by(format!("scroll of {} -> dismiss", label(frame, i))));
            }
        }
        out
    }

    /// The node focus goes to on a press of chain `chain` (innermost
    /// first): the nearest focusable or what the nearest node handing on a
    /// press focuses, or nothing; `Err` when focus stays (the top focus
    /// scope is trapped and the press is outside it, a node handing on a
    /// press hands on none, or its `focus_on_press` refuses).
    fn focus_on_press(&self, frame: &Frame, chain: &[usize]) -> Result<Option<FocusId>, ()> {
        // The nearest node that takes focus, or that hands a press on it to
        // another's focus ([`crate::Div::press_focuses`]): focus, not an
        // action, so an unavailable node's runs too.
        let at = match chain.iter().find_map(|&i| {
            let b = frame.behaviour(i)?;
            if let Some(f) = b.focus {
                return Some(Some((i, f.id())));
            }
            let to = b.handlers.press_focuses.clone()?;
            Some(to().and_then(|h| Some((focused_node(frame, h.id())?, h.id()))))
        }) {
            Some(None) => return Err(()),
            Some(at) => at,
            None => None,
        };
        if let Some(top) = self.layers.top_scope()
            && top.spec.scope.is_some_and(|s| s.trapped)
            && at.is_none_or(|(i, _)| !self.layers.in_scope(frame, top, i))
        {
            return Err(());
        }
        if let Some((i, _)) = at
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.press_focus.clone())
            && !f(&self.modifiers)
        {
            return Err(());
        }
        Ok(at.map(|(_, id)| id))
    }

    /// The mouse's primary button went down at `(x, y)`.
    pub fn mouse_down(&mut self, frame: &Frame, text: &mut TextSystem, x: f32, y: f32) -> Outcome {
        self.take_in(frame);
        acts_by(Modality::Pointer);
        self.pointer = Some((x, y));
        // A press while the button is held (its release never came) ends
        // that press first, as lost.
        let ended = self.let_go(frame, |p| p.id == PointerId::Mouse);
        let (h, ..) = self.locate(frame, x, y);
        // A mouse dismisses as it goes down, as menus of the OS do (a pen
        // too, in `touch`).
        let outside = self.dismiss_outside(
            frame,
            h,
            self.outside_at(x, y, PointerKind::Mouse, PointerButton::Primary),
        );
        self.pointers
            .push(Pointer::new(PointerId::Mouse, PointerKind::Mouse, (x, y)));
        ended
            .merge(self.press_primary(frame, text, PointerId::Mouse, x, y))
            .merge(outside)
    }

    /// What a press at `(x, y)` of `button` is to a layer it is outside of.
    fn outside_at(&self, x: f32, y: f32, kind: PointerKind, button: PointerButton) -> PointerOutside {
        PointerOutside {
            x,
            y,
            kind,
            button,
            modifiers: self.modifiers,
        }
    }

    /// Press pointer `id` (the mouse's primary button, or a contact's tap)
    /// at `(x, y)` on screen: focus, its pressed state, the caret,
    /// `on_mouse_down` and a drag, all its own.
    fn press_primary(&mut self, frame: &Frame, text: &mut TextSystem, id: PointerId, x: f32, y: f32) -> Outcome {
        let (h, ..) = self.locate(frame, x, y);
        let mut out = Outcome::redraw();
        let chain: Vec<usize> = h.map(|i| frame.ancestors(i).collect()).unwrap_or_default();
        // A press inside a disabled subtree moves no focus and places no
        // caret; its gestures pass the subtree by (`Reach::Gesture`).
        let disabled = h.is_some_and(|h| disabled(frame, h));
        if !disabled && let Ok(to) = self.focus_on_press(frame, &chain) {
            focus::set_focus(to, false);
        }
        let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) else {
            return out;
        };
        p.hit = h.and_then(|i| Held::of(frame, i));
        p.down = p.hit.as_ref().map(|h| h.path.clone());
        p.pressing = true;
        p.start = (x, y);
        p.last = (x, y);
        p.claimed = false;
        let kind = p.kind;
        self.write_pressed(frame);
        if !disabled
            && let Some(i) = chain
                .iter()
                .copied()
                .find(|&i| frame.behaviour(i).is_some_and(|b| b.field.is_some()))
        {
            let (fx, fy) = self.shown(frame).to_node(i, (x, y));
            self.place_caret(frame, text, i, fx, fy, self.modifiers.shift);
            if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
                p.selecting = Held::of(frame, i);
            }
        }
        // A press and a drag are gestures: they pass a disabled or
        // unavailable node by to the ancestor that owns them
        // (`Reach::Gesture`).
        let gesture = |has: fn(&Behaviour) -> bool| h.and_then(|h| Reach::Gesture.target(frame, h, has));
        if let Some(i) = gesture(|b| b.handlers.mouse_down.is_some())
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.mouse_down.clone())
        {
            let ((fx, fy), bounds) = self.shown(frame).event(i, (x, y));
            f(&ClickEvent {
                x: fx,
                y: fy,
                bounds,
                modifiers: self.modifiers,
            });
            out = out.merge(Outcome::by(format!("mouse down -> {}", label(frame, i))));
        }
        if let Some(i) = gesture(|b| b.handlers.drag.is_some())
            && !self.captured(frame, i, id)
            && let Some(grip) = Grip::of(frame, &self.shown(frame), i, drag_of)
        {
            let on_control = on_control(frame, &chain, i);
            // Where the frame has the press for the node it drags (which
            // the compositor may show otherwise than the node hit), as its
            // moves will have it.
            let (dx, dy) = self.shown(frame).to_node(i, (x, y));
            let ev = DragEvent {
                x: dx,
                y: dy,
                bounds: grip.shown_box,
                phase: DragPhase::Start,
                pointer: id,
                kind,
                on_control,
            };
            let claimed = claiming(|| (grip.handler)(&ev));
            let seen = self.shown(frame).local(i, (x, y));
            if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
                p.on_control = on_control;
                p.drag = Some(grip);
                p.seen = seen;
                p.claimed |= claimed;
            }
            out = out.merge(Outcome::by(format!("drag start -> {}", label(frame, i))));
        }
        out
    }

    pub fn mouse_up(&mut self, frame: &Frame, x: f32, y: f32) -> Outcome {
        self.take_in(frame);
        let cancelled = self.cancel_lost(frame);
        if let Some(p) = self.pointers.iter_mut().find(|p| p.id == PointerId::Mouse) {
            p.last = (x, y);
        }
        cancelled.merge(self.release(frame, PointerId::Mouse, x, y))
    }

    /// Pointer `id` comes up at `(x, y)` on screen (the mouse's primary
    /// button, a contact's tap): it ends its drag there, and clicks the
    /// node under it if it went down on it or below it and no drag of its
    /// claimed it, else releases there. Nothing of it is left.
    fn release(&mut self, frame: &Frame, id: PointerId, x: f32, y: f32) -> Outcome {
        let Some(k) = self.pointers.iter().position(|p| p.id == id) else {
            return Outcome::default();
        };
        let mut p = self.pointers.remove(k);
        if p.selecting.is_some() {
            self.stop_autoscroll();
        }
        self.write_pressed(frame);
        let mut out = Outcome::redraw();
        // Lost since (gone or disabled): cancelled before this.
        if let Some(grip) = p.drag.as_mut()
            && let Some(i) = grip.live(frame, &self.shown(frame), drag_of)
        {
            let (fx, fy) = self.shown(frame).to_node(i, (x, y));
            let ev = DragEvent {
                x: fx,
                y: fy,
                bounds: grip.shown_box,
                phase: DragPhase::End,
                pointer: p.id,
                kind: p.kind,
                on_control: p.on_control,
            };
            p.claimed |= claiming(|| (grip.handler)(&ev));
        }
        if p.claimed {
            return out;
        }
        let (Some(h), ..) = self.locate(frame, x, y) else {
            return out;
        };
        let pressed_at = p.hit.and_then(|p| p.find(frame));
        // A link inside a paragraph takes the click before the paragraph's
        // ancestors: the paragraph's own activation.
        let shown = self.shown(frame);
        let ((hx, hy), h_bounds) = shown.event(h, (x, y));
        let shown_span = (pressed_at == Some(h) && Reach::Activation.target(frame, h, |_| true) == Some(h))
            .then(|| shown.span_at(h, (hx, hy)))
            .flatten();
        // A span the frame showed there and gone since takes the click and
        // does nothing, as a node gone since clicks nothing.
        if shown_span.as_ref().is_some_and(|s| s.live.is_none()) {
            return out;
        }
        if let Some(span) = shown_span.and_then(|s| s.live)
            && let Some(f) = span.click.clone()
        {
            f(&ClickEvent {
                x: hx,
                y: hy,
                bounds: h_bounds,
                modifiers: self.modifiers,
            });
            let range = &span.range;
            return Outcome::by(format!(
                "click ({hx:.0},{hy:.0}) -> span {range:?} of {}",
                label(frame, h)
            ));
        }
        // Gone since it went down (a node made again at its path is
        // another), it clicks nothing, as Blink forgets the mouse down
        // node once it is removed; a contact's tap lifting on what is left
        // above it presses that instead ([`Self::tap`]).
        if let Some(i) = pressed_at.and_then(|d| click_target(frame, d, h))
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.click.clone())
        {
            let ((fx, fy), bounds) = shown.event(i, (x, y));
            f(&ClickEvent {
                x: fx,
                y: fy,
                bounds,
                modifiers: self.modifiers,
            });
            out = Outcome::by(format!("click ({fx:.0},{fy:.0}) -> {}", label(frame, i)));
        }
        out.merge(self.release_at(frame, h, x, y, pressed_at))
    }

    /// Run the nearest `on_release` of node `h`'s chain for a press that
    /// went down outside it (`pressed_at`), released at `(x, y)` on screen:
    /// a menu item picked by a press elsewhere dragged onto it.
    fn release_at(&self, frame: &Frame, h: usize, x: f32, y: f32, pressed_at: Option<usize>) -> Outcome {
        let target = Reach::Activation.target(frame, h, |b| b.handlers.release.is_some());
        if let Some(i) = target
            && !pressed_at.is_some_and(|p| Self::within(frame, p, i))
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.release.clone())
        {
            let ((x, y), bounds) = self.shown(frame).event(i, (x, y));
            f(&ClickEvent {
                x,
                y,
                bounds,
                modifiers: self.modifiers,
            });
            return Outcome::by(format!("release ({x:.0},{y:.0}) -> {}", label(frame, i)));
        }
        Outcome::default()
    }

    /// A mouse button other than the primary went down at `(x, y)`: it
    /// dismisses a layer it is outside of, whichever button it is (ADR-0024);
    /// the secondary one also runs `on_secondary_click`.
    pub fn button_down(&mut self, frame: &Frame, x: f32, y: f32, button: PointerButton) -> Outcome {
        self.take_in(frame);
        acts_by(Modality::Pointer);
        self.pointer = Some((x, y));
        let (h, ..) = self.locate(frame, x, y);
        let (outside, closed_modal) =
            self.dismiss_outside_closing(frame, h, self.outside_at(x, y, PointerKind::Mouse, button));
        match button {
            // A secondary press that closes the top modal layer reaches what
            // is under it, as a platform's context menu hands a right-click
            // outside it to what is below: another context menu's area opens
            // its menu in the same press (reka's `ContextMenu` test).
            PointerButton::Secondary if closed_modal => {
                let below = self.layers.modal.map_or(0, |m| {
                    self.layers
                        .entries
                        .get(..m)
                        .and_then(|under| under.iter().rposition(|e| e.spec.modal))
                        .and_then(|i| self.layers.entries.get(i))
                        .map_or(0, |e| crate::layer::layer_start(frame, e.layer))
                });
                let h = self.shown(frame).hit(below, true, (x, y));
                self.secondary_at(frame, h, x, y).merge(outside)
            }
            PointerButton::Secondary => self.secondary_at(frame, h, x, y).merge(outside),
            _ => outside,
        }
    }

    /// Run the nearest `on_secondary_click` of node `h`'s chain, a press at
    /// `(x, y)` on screen.
    fn secondary_at(&self, frame: &Frame, h: Option<usize>, x: f32, y: f32) -> Outcome {
        let mut out = Outcome::redraw();
        let Some(h) = h else { return out };
        let target = Reach::Gesture.target(frame, h, |b| b.handlers.secondary_click.is_some());
        if let Some(i) = target
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.secondary_click.clone())
        {
            let ((x, y), bounds) = self.shown(frame).event(i, (x, y));
            f(&ClickEvent {
                x,
                y,
                bounds,
                modifiers: self.modifiers,
            });
            out = Outcome::by(format!("secondary click ({x:.0},{y:.0}) -> {}", label(frame, i)));
        }
        out
    }

    /// The wheel turned by `(dx, dy)` at the pointer (the devtools'); below
    /// a modal layer nothing scrolls.
    pub fn wheel(&mut self, frame: &Frame, dx: f32, dy: f32) -> Outcome {
        match self.pointer {
            Some(point) => {
                let step = crate::compositor::WheelStep {
                    point,
                    // The frame clock's now: a notched turn's latch times
                    // turns of this one source alone.
                    time: crate::clock::current_nanos(),
                    ..crate::compositor::WheelStep::default()
                };
                self.wheel_at(frame, step, (dx, dy), None)
            }
            None => Outcome::default(),
        }
    }

    /// The wheel turned by `(dx, dy)` at `(x, y)` on screen (the platform's,
    /// handed on by the render thread, under the frame shown as it came):
    /// routed from `target`, the node the render thread found it over in
    /// that frame, while it is there by identity (its key and the instance
    /// that made it, as every latch is checked); gone, or without one,
    /// from what is at `(x, y)` now, wherever the mouse is.
    pub fn wheel_at(
        &mut self,
        frame: &Frame,
        step: crate::compositor::WheelStep,
        (dx, dy): (f32, f32),
        target: Option<crate::compositor::HitTarget>,
    ) -> Outcome {
        self.take_in(frame);
        let (x, y) = step.point;
        // A node no longer shown (hidden, inert since), or under a modal
        // layer opened since, takes no wheel, as it takes no other input:
        // the target is gone, and what is at the point now takes it.
        let reaches = |h: &usize| frame.shown(*h) && !self.layers.blocked(*h);
        let h = target
            .and_then(|t| NODES.with(|n| n.borrow_mut().find(frame, t)))
            .filter(reaches)
            .or_else(|| self.locate(frame, x, y).0.filter(reaches));
        let Some(h) = h else {
            return Outcome::default();
        };
        self.scroll_from(frame, h, step, dx, dy, PointerId::Mouse)
    }

    /// The wheel gesture the compositor latched to a container goes on
    /// here (ADR-0033: input goes straight to the UI from now on): the
    /// wheel's latch is that gesture, latched to that container as `frame`
    /// holds it, so the rest of it scrolls there and passes nothing on.
    /// A container `frame` no longer holds leaves the latch as it was.
    pub(crate) fn take_gesture(&mut self, frame: &Frame, gesture: crate::compositor::HandedGesture) {
        let held = gesture.0.map(
            |t| {
                NODES
                    .with(|n| n.borrow_mut().find(frame, *t))
                    .and_then(|i| Held::of(frame, i))
            },
            Some,
        );
        if let Some(held) = held {
            self.scroll_latch = held;
            // Its motion goes on here: the rest of its events and its lift.
            self.scroll_motion = gesture.2.unwrap_or_default();
        } else if let Some(lifted) = gesture.2.filter(|m| m.lifted.is_some()) {
            // A lift with no latch left (its own handed back to it, on its
            // way when input was diverted again): kept to go back again.
            self.scroll_motion = lifted;
        }
    }

    /// The wheel's gesture it latched, by identity (tests: one side holds
    /// a gesture at a time).
    #[cfg(test)]
    pub(crate) fn holds_gesture(&self) -> Option<u64> {
        self.scroll_latch.holds().map(|(id, _)| id)
    }

    /// The wheel's gesture under way, handed back to the compositor as
    /// input comes through it again (ADR-0033, Codex r12 1): its latch from
    /// then on, so the rest of the gesture stays on its container. One
    /// side latches a gesture at a time: the UI keeps none it hands, unless
    /// the compositor would route the gesture to it anyway (a wheel handler
    /// on the container, or between it and the one around it, the paint's
    /// `ScrollNode::wheel` and `wheel_above`), where it keeps it (`.1`).
    pub(crate) fn hand_gesture(&mut self, frame: &Frame) -> Option<crate::compositor::HandedGesture> {
        let mut keeps = false;
        let handed = self.scroll_latch.map(
            |held| {
                let i = held.find(frame)?;
                keeps = frame
                    .ancestors(i)
                    .take_while(|&a| a == i || frame.nodes.get(a).is_some_and(|n| laid_scroll(n).is_none()))
                    .any(|a| frame.behaviour(a).is_some_and(|b| b.handlers.wheel.is_some()));
                Some(crate::compositor::HitTarget {
                    key: crate::compositor::NodeKey::of(&held.path),
                    owner: held.owner,
                })
            },
            Some,
        );
        if !keeps {
            self.scroll_latch.end();
        }
        // Its motion with it, for the compositor to fling from (none it
        // keeps: the UI routes that gesture on). A lift whose latch ended
        // here since (the platform's momentum over, or a latch the frame
        // no longer holds) still flings, and goes back alone, to the
        // container it moved, unless another gesture is latched since
        // (fingers down, a notched wheel), which stopped its fling.
        let motion = std::mem::take(&mut self.scroll_motion);
        match handed {
            Some(g) if g.holds().is_some() => {
                let mine = g.holds().is_some_and(|(began, _)| began == motion.began);
                Some(crate::compositor::HandedGesture(
                    g,
                    keeps,
                    (mine && !keeps).then_some(motion),
                ))
            }
            other if motion.lifted.is_some() && motion.to.is_some() => Some(crate::compositor::HandedGesture(
                other.unwrap_or_default(),
                false,
                Some(motion),
            )),
            other => other.map(|g| crate::compositor::HandedGesture(g, keeps, None)),
        }
    }

    /// Whether a scroll gesture may reach node `i` now, by one rule for
    /// what it latched to and what it walks up to: shown, not below the
    /// top modal layer, and passing a gesture on.
    fn reaches_scroll(&self, frame: &Frame, i: usize) -> bool {
        frame.shown(i) && !self.layers.blocked(i) && frame.behaviour(i).is_none_or(|b| Reach::Gesture.passes(b))
    }

    /// The scroll gesture `whose` latches: the wheel's, or a contact's pan's.
    fn latch_of(&mut self, whose: PointerId) -> Option<&mut crate::compositor::WheelGesture<Held>> {
        match whose {
            PointerId::Mouse => Some(&mut self.scroll_latch),
            PointerId::Contact(_) => self.pointers.iter_mut().find(|p| p.id == whose).map(|p| &mut p.latch),
        }
    }

    /// Scroll by `(dx, dy)` what takes it from node `h` up, the pointer at
    /// `step.point` on screen: a wheel handler, else a container, as Chrome
    /// scrolls. A gesture stays latched to the container that took its
    /// first event, even once it cannot go on, by the rules the compositor
    /// latches by ([`crate::compositor::WheelGesture`]), each in `whose`
    /// gesture; a pan is one gesture from its contact down. A new one starts at the innermost container that can
    /// still scroll that way, passing those at their end to the one around
    /// them. Layers whose owner it holds and that take scrolls are asked
    /// to close.
    fn scroll_from(
        &mut self,
        frame: &Frame,
        h: usize,
        step: crate::compositor::WheelStep,
        dx: f32,
        dy: f32,
        whose: PointerId,
    ) -> Outcome {
        let at = step.point;
        // The compositor's rules, the same functions: what can move that
        // way, and what takes it at all (held against its edge).
        let along_of = |n: &crate::layout::FrameNode| {
            if n.look.text.direction == crate::style::Direction::Rtl {
                -dx
            } else {
                dx
            }
        };
        let can_scroll = |i: usize| {
            frame.nodes.get(i).is_some_and(|n| {
                laid_scroll(n)
                    .is_some_and(|s| crate::compositor::wheel_moves(s.pos, s.end, motion::units2((along_of(n), dy))))
            })
        };
        let takes = |i: usize| {
            frame.nodes.get(i).is_some_and(|n| {
                laid_scroll(n).is_some_and(|s| crate::compositor::wheel_takes(s.end, motion::units2((along_of(n), dy))))
            })
        };
        // The innermost container under it: moved onto another, a notched
        // wheel's turns are a new gesture.
        let under = frame
            .ancestors(h)
            .filter_map(|i| frame.nodes.get(i))
            .find(|n| laid_scroll(n).is_some())
            .map(|n| crate::compositor::NodeKey::of(&n.path));
        let held = self
            .latch_of(whose)
            .and_then(|l| l.held(&step, || Some(under)))
            .map(|held| held.find(frame));
        // What it latched to takes it while input could still reach it
        // ([`Self::reaches_scroll`]): a modal opened over a list mid-gesture
        // ends its gesture there.
        let latched = held.flatten().filter(|&i| self.reaches_scroll(frame, i));
        if held.is_some()
            && latched.is_none()
            && let Some(l) = self.latch_of(whose)
        {
            l.end();
        }
        // A gesture's reach: a disabled or unavailable node's own handler
        // does not run (a number field disabled since the frame it came
        // under), and it passes the wheel on up.
        let wheel = |i: usize| {
            frame
                .behaviour(i)
                .filter(|b| Reach::Gesture.runs_on(b))
                .and_then(|b| b.handlers.wheel.clone())
        };
        // The nearest that takes it: a node whose wheel handler consumes it
        // (a canvas zooming, a focused number field stepping), or the
        // container its gesture latched to, or else one that can scroll
        // that way. A handler that declines passes it on up.
        let mut target = latched;
        // By the same rule: what went down before a modal opened (a pan's
        // contact) finds nothing below it.
        for i in frame.ancestors(h).filter(|&i| self.reaches_scroll(frame, i)) {
            if let Some(f) = wheel(i) {
                let ((x, y), bounds) = self.shown(frame).event(i, at);
                let consumed = f(&crate::events::WheelEvent { x, y, dx, dy, bounds });
                if consumed {
                    return Outcome::redraw();
                }
            }
            if latched.is_none() && can_scroll(i) {
                target = Some(i);
                break;
            }
        }
        // None can move that way: the innermost that scrolls along that
        // axis takes it against its edge, as the compositor's route has it.
        if target.is_none() {
            target = frame
                .ancestors(h)
                .filter(|&i| self.reaches_scroll(frame, i))
                .find(|&i| takes(i));
        }
        let Some((i, node)) = target.and_then(|i| Some((i, frame.nodes.get(i)?))) else {
            return Outcome::default();
        };
        let to = Held::of(frame, i);
        if let Some(l) = self.latch_of(whose) {
            l.went(&step, under, to, latched.is_some());
        }
        // A precise gesture's motion, as the compositor keeps it: its fling
        // at the lift is the same whichever side took the lift.
        if whose == PointerId::Mouse
            && step.precise
            && let Some((began, _)) = self.scroll_latch.holds()
        {
            let to = crate::compositor::HitTarget {
                key: crate::compositor::NodeKey::of(&node.path),
                owner: owner_of(frame, i),
            };
            self.scroll_motion
                .record(began, step.time, motion::units2((dx, dy)), step.phase.lifted(), to);
        }
        let Some(scroll) = laid_scroll(node) else {
            return Outcome::default();
        };
        // Offsets run from the inline start: a right-to-left container
        // scrolls further as the content moves right.
        let along = if node.look.text.direction == crate::style::Direction::Rtl {
            -dx
        } else {
            dx
        };
        // Exactly ([`crate::compositor::motion`]): the router handed the
        // step on in whole units.
        let step = motion::units2((along, dy));
        let next = motion::clamp2(motion::add2(scroll.pos, step), scroll.end);
        if next == scroll.pos {
            // Against its edge: along an axis it scrolls it took the wheel
            // all the same, so a handle's overlay scrollbar shows, but
            // nothing moves, so nothing is dismissed; along one it does not
            // scroll (a latched list's sideways step) it took nothing.
            return match Holder::of(frame, i) {
                Some(Holder::Handle(h)) if takes(i) => {
                    h.wheeled_by(1);
                    Outcome::redraw()
                }
                _ => Outcome::default(),
            };
        }
        // Its own scroll: a displacement on where the UI holds it.
        self.scroll_by(frame, i, motion::sub2(next, scroll.pos));
        Outcome::redraw().merge(self.dismiss_scrolled(frame, i))
    }

    /// A finger or a pen touched, moved, lifted or was taken away
    /// ([`crate::gesture`]); each contact is a pointer of its own, as
    /// Chrome's pointer events have them, with its own press, capture and
    /// gesture. A tap presses and clicks, as it lifts, the node under where
    /// it went down in the frame as it is then (one target for all of it,
    /// as Chrome's `GestureTap`), after asking the layers it is outside of
    /// to close (a finger dismisses once it is a tap, so a pan or a long
    /// press does not; a pen as it goes down, as a mouse does); a long
    /// press runs the nearest `on_long_press`, else `on_secondary_click`; a
    /// pan drags a node that takes drags, else scrolls. A contact that goes
    /// down beside another makes neither a tap nor a long press (Chrome's
    /// `CancelTaps`); each may still drag, and the first to pan scrolls.
    pub fn touch(&mut self, frame: &Frame, text: &mut TextSystem, ev: &TouchEvent) -> Outcome {
        self.take_in(frame);
        let cancelled = self.cancel_lost(frame);
        acts_by(Modality::Pointer);
        let out = match ev.phase {
            TouchPhase::Started => self.contact_down(frame, ev),
            TouchPhase::Moved => self.contact_moved(frame, ev),
            TouchPhase::Ended => self.contact_up(frame, text, ev),
            TouchPhase::Cancelled => self.contact_cancelled(frame, ev),
        };
        cancelled.merge(out)
    }

    /// The raw touch handler contact `id` captured, as the frame has it
    /// now (lost since, it was cancelled before this).
    fn raw_of(&mut self, frame: &Frame, id: PointerId) -> Option<crate::element::Handler<TouchEvent>> {
        let shown = Shown::new(frame, self.shown.as_deref(), &self.hit_keys);
        let p = self.pointers.iter_mut().find(|p| p.id == id)?;
        let g = p.raw.as_mut()?;
        g.live(frame, &shown, touch_of)?;
        Some(std::rc::Rc::clone(&g.handler))
    }

    /// Cancel what any pointer holds on a node that has gone since (the
    /// frame was rebuilt without it), been disabled, by itself or by an
    /// ancestor, been made unavailable, or lost its handler: its drag gets
    /// [`DragPhase::Cancel`] and its raw touch handler
    /// [`TouchPhase::Cancelled`], once, through the handler it was last
    /// found with, so that it lets go; nothing reaches either after. Each
    /// pointer is cancelled only for what it held. A node a pointer went
    /// down on that has gone is no longer pressed nor clicked by it, though
    /// one made again with the same key comes; a text selection in a field
    /// gone or disabled stops.
    fn cancel_lost(&mut self, frame: &Frame) -> Outcome {
        let mut out = Outcome::default();
        let mut unpressed = false;
        for k in 0..self.pointers.len() {
            let Some(p) = self.pointers.get_mut(k) else { break };
            if p.selecting
                .as_ref()
                .is_some_and(|s| s.find(frame).is_none_or(|i| disabled(frame, i)))
            {
                p.selecting = None;
            }
            if p.hit.as_ref().is_some_and(|h| h.find(frame).is_none()) {
                p.hit = None;
                unpressed |= p.pressing;
            }
            let shown = Shown::new(frame, self.shown.as_deref(), &self.hit_keys);
            let drag = p.drag.take_if(|g| g.live(frame, &shown, drag_of).is_none());
            let raw = p.raw.take_if(|g| g.live(frame, &shown, touch_of).is_none());
            let (id, contact, kind, last, on_control) = (p.id, p.contact(), p.kind, p.last, p.on_control);
            if let Some(mut grip) = drag {
                out = out.merge(self.cancel_drag(frame, &mut grip, last, id, kind, on_control));
            }
            if let Some(grip) = raw {
                (grip.handler)(&TouchEvent {
                    id: contact,
                    kind,
                    phase: TouchPhase::Cancelled,
                    x: last.0,
                    y: last.1,
                });
                out = out.merge(Outcome::by(format!("touch cancel -> {}", grip.label(frame))));
            }
        }
        if unpressed {
            self.write_pressed(frame);
            out = out.merge(Outcome::redraw());
        }
        out
    }

    /// The pointers `which` picks let go of all they hold, as Chrome's
    /// `pointercancel` and `lostpointercapture`: each drag gets
    /// [`DragPhase::Cancel`] and each raw touch handler
    /// [`TouchPhase::Cancelled`], once, where the pointer was last; they
    /// show pressed no more, and their releases do nothing.
    fn let_go(&mut self, frame: &Frame, which: impl Fn(&Pointer) -> bool) -> Outcome {
        if !self.pointers.iter().any(&which) {
            return Outcome::default();
        }
        let (gone, kept): (Vec<Pointer>, Vec<Pointer>) =
            std::mem::take(&mut self.pointers).into_iter().partition(which);
        self.pointers = kept;
        self.write_pressed(frame);
        let mut out = Outcome::redraw();
        for mut p in gone {
            if let Some(grip) = p.drag.as_mut() {
                out = out.merge(self.cancel_drag(frame, grip, p.last, p.id, p.kind, p.on_control));
            }
            let contact = p.contact();
            if let Some(grip) = p.raw.as_mut() {
                grip.live(frame, &self.shown(frame), touch_of);
                (grip.handler)(&TouchEvent {
                    id: contact,
                    kind: p.kind,
                    phase: TouchPhase::Cancelled,
                    x: p.last.0,
                    y: p.last.1,
                });
                out = out.merge(Outcome::by(format!("touch cancel -> {}", grip.label(frame))));
            }
        }
        out
    }

    /// The component instances `leaving` picks are about to be disposed
    /// (unmounted by the resolve that ran): each capture a node of theirs
    /// holds, a drag or a raw touch, gets its cancel now, its handler run
    /// while its instance is alive (it may let go of what it shares with
    /// others, a splitter's partner handle), in `frame`, the last that held
    /// those nodes. Their nodes are no longer pressed, clicked nor
    /// selected in; the pointers stay down.
    pub(crate) fn release_owned(
        &mut self,
        frame: &Frame,
        leaving: &dyn Fn(crate::reactive::OwnerId) -> bool,
    ) -> Outcome {
        let gone = |h: &Held| h.owner.is_some_and(leaving);
        let mut out = Outcome::default();
        let mut unpressed = false;
        for k in 0..self.pointers.len() {
            let Some(p) = self.pointers.get_mut(k) else { break };
            if p.selecting.as_ref().is_some_and(gone) {
                p.selecting = None;
            }
            if p.hit.as_ref().is_some_and(gone) {
                p.hit = None;
                unpressed |= p.pressing;
            }
            let drag = p.drag.take_if(|g| gone(&g.node));
            let raw = p.raw.take_if(|g| gone(&g.node));
            let (id, contact, kind, last, on_control) = (p.id, p.contact(), p.kind, p.last, p.on_control);
            if let Some(mut grip) = drag {
                out = out.merge(self.cancel_drag(frame, &mut grip, last, id, kind, on_control));
            }
            if let Some(grip) = raw {
                (grip.handler)(&TouchEvent {
                    id: contact,
                    kind,
                    phase: TouchPhase::Cancelled,
                    x: last.0,
                    y: last.1,
                });
                out = out.merge(Outcome::by(format!("touch cancel -> {}", grip.label(frame))));
            }
        }
        if unpressed {
            self.write_pressed(frame);
            out = out.merge(Outcome::redraw());
        }
        out
    }

    /// Whether node `i` of `frame` is captured by a pointer other than
    /// `id`: another pointer's drag or raw touch holds it or a node of its
    /// capture group, whichever this one would take of it. One pointer
    /// captures a node at a time, as Chrome's pointer capture: another
    /// pressing it meanwhile gets nothing of it, its drag nor its touch,
    /// unless it takes several ([`crate::Div::multi_pointer`]).
    fn captured(&self, frame: &Frame, i: usize, id: PointerId) -> bool {
        let Some(b) = frame.behaviour(i) else {
            return false;
        };
        if b.handlers.multi_pointer {
            return false;
        }
        let group = b.handlers.capture_group;
        self.pointers.iter().filter(|p| p.id != id).any(|p| {
            p.drag.as_ref().is_some_and(|g| g.holds(frame, i, group))
                || p.raw.as_ref().is_some_and(|g| g.holds(frame, i, group))
        })
    }

    /// The window lost the keyboard focus: every pointer down lets go of
    /// all it holds, once ([`Self::let_go`]), as Chrome cancels pointers
    /// (`pointercancel`, `lostpointercapture`) when its window loses focus
    /// or capture; nothing shows pressed, and a release after clicks
    /// nothing.
    pub fn blur(&mut self, frame: &Frame) -> Outcome {
        self.take_in(frame);
        self.let_go(frame, |_| true)
    }

    /// Give the drag `grip` holds its [`DragPhase::Cancel`] at screen point
    /// `at`, with its node's box as the frame has it now (gone, as it last
    /// was), at the point where the frame has it for that node.
    fn cancel_drag(
        &self,
        frame: &Frame,
        grip: &mut Grip<DragEvent>,
        at: (f32, f32),
        pointer: PointerId,
        kind: PointerKind,
        on_control: bool,
    ) -> Outcome {
        let shown = self.shown(frame);
        let bounds = grip.bounds_in(frame, &shown);
        let (x, y) = match grip.node.find(frame) {
            Some(i) => shown.to_node(i, at),
            None => at,
        };
        (grip.handler)(&DragEvent {
            x,
            y,
            bounds,
            phase: DragPhase::Cancel,
            pointer,
            kind,
            on_control,
        });
        Outcome::by(format!("drag cancel -> {}", grip.label(frame)))
    }

    /// A contact went down: a pointer of its own, pressing what is under
    /// it, its raw phases captured by the nearest `on_touch` there.
    fn contact_down(&mut self, frame: &Frame, ev: &TouchEvent) -> Outcome {
        let id = PointerId::Contact(ev.id);
        if self.pointers.iter().any(|p| p.id == id) {
            return Outcome::default();
        }
        let (x, y) = (ev.x, ev.y);
        let (h, ..) = self.locate(frame, x, y);
        // Its raw phases go to the nearest `on_touch` where it went down,
        // found now and kept for its later phases whatever is under it
        // then (a captured pointer); `Ended` once what it turned out to be
        // has run.
        let raw = h
            .and_then(|h| Reach::Gesture.target(frame, h, |b| b.handlers.touch.is_some()))
            .filter(|&i| !self.captured(frame, i, id))
            .and_then(|i| Grip::of(frame, &self.shown(frame), i, touch_of));
        if let Some(g) = &raw {
            (g.handler)(ev);
        }
        // A pen dismisses as it goes down, as a mouse does; a finger only
        // once it is a tap (a pan or a long press does not).
        let outside = if ev.kind == PointerKind::Pen {
            self.dismiss_outside(frame, h, self.outside_at(x, y, ev.kind, PointerButton::Primary))
        } else {
            Outcome::default()
        };
        let mut gesture = crate::gesture::Press::down(ev.kind, x, y, clock_now());
        // Beside another contact, none of them is a tap or a long press.
        let mut beside = false;
        for g in self.pointers.iter_mut().filter_map(|p| p.gesture.as_mut()) {
            g.not_single();
            beside = true;
        }
        if beside {
            gesture.not_single();
        }
        // The long press's own delay, if its node asks for one.
        let after = h
            .and_then(|h| Reach::Gesture.target(frame, h, |b| b.handlers.long_press.is_some()))
            .and_then(|i| frame.behaviour(i)?.handlers.long_press_after)
            .unwrap_or(crate::gesture::LONG_PRESS);
        gesture.long_after(after);
        if gesture.waiting() {
            // Wake the frame clock when it would be long.
            crate::clock::after(after, || {});
        }
        let mut p = Pointer::new(id, ev.kind, (x, y));
        p.hit = h.and_then(|h| Held::of(frame, h));
        p.down = p.hit.as_ref().map(|h| h.path.clone());
        p.pressing = true;
        p.raw = raw;
        p.gesture = Some(gesture);
        self.pointers.push(p);
        self.write_pressed(frame);
        Outcome::redraw().merge(outside)
    }

    /// A contact moved: its raw phase, then its pan (a drag, else a
    /// scroll) once past the slop.
    fn contact_moved(&mut self, frame: &Frame, ev: &TouchEvent) -> Outcome {
        let id = PointerId::Contact(ev.id);
        if let Some(f) = self.raw_of(frame, id) {
            f(ev);
        }
        let (x, y) = (ev.x, ev.y);
        let others_pan = self.pointers.iter().any(|p| p.id != id && p.panning);
        let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) else {
            return Outcome::default();
        };
        let began = p.gesture.as_mut().and_then(|g| g.moved(x, y)) == Some(Gesture::PanStart);
        let mut delta = (p.last.0 - x, p.last.1 - y);
        p.last = (x, y);
        if p.long {
            // Held into a long press: it only goes to where it lifts.
            return Outcome::default();
        }
        let (start, kind) = (p.start, p.kind);
        // Where it went down, as the frame has it now.
        let hit = p.hit.as_ref().and_then(|h| h.find(frame));
        let mut out = Outcome::default();
        if began {
            p.pressing = false;
            self.write_pressed(frame);
            let drags = hit.and_then(|h| Reach::Gesture.target(frame, h, |b| b.handlers.drag.is_some()));
            // A scroll container inside the dragged node that can still
            // scroll the way the finger goes takes the pan first (a list
            // in a drawer scrolls until its edge).
            let pan = (start.0 - x, start.1 - y);
            let drags = drags.filter(|&d| {
                !hit.is_some_and(|h| {
                    frame
                        .ancestors(h)
                        .take_while(|&i| i != d)
                        .any(|i| can_scroll_toward(frame, i, pan))
                })
            });
            // Captured by another pointer: it drags nothing there.
            let free = drags.filter(|&d| !self.captured(frame, d, id));
            if drags.is_some() && free.is_none() {
                return Outcome::redraw();
            }
            match free.and_then(|i| Some((i, Grip::of(frame, &self.shown(frame), i, drag_of)?))) {
                Some((i, grip)) => {
                    let on_control = hit.is_some_and(|h| {
                        let chain: Vec<usize> = frame.ancestors(h).collect();
                        on_control(frame, &chain, i)
                    });
                    let (sx, sy) = self.shown(frame).to_node(i, start);
                    let ev = DragEvent {
                        x: sx,
                        y: sy,
                        bounds: grip.shown_box,
                        phase: DragPhase::Start,
                        pointer: id,
                        kind,
                        on_control,
                    };
                    let claimed = claiming(|| (grip.handler)(&ev));
                    if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
                        p.on_control = on_control;
                        p.drag = Some(grip);
                        p.claimed |= claimed;
                    }
                }
                // The first contact to pan scrolls; one beside it does not
                // scroll the same content again.
                None if !others_pan => {
                    if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
                        p.panning = true;
                    }
                    // The pan scrolls from where it went down.
                    delta = (start.0 - x, start.1 - y);
                }
                None => {}
            }
            out = Outcome::redraw();
        }
        let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) else {
            return out;
        };
        let on_control = p.on_control;
        let panning = p.panning;
        let press = p.press;
        // Lost since (gone or disabled): cancelled before this.
        let shown = Shown::new(frame, self.shown.as_deref(), &self.hit_keys);
        let drag = p.drag.as_mut().and_then(|g| {
            let i = g.live(frame, &shown, drag_of)?;
            Some((i, std::rc::Rc::clone(&g.handler), g.shown_box))
        });
        if let Some((i, f, bounds)) = drag {
            let shown = self.shown(frame);
            let (fx, fy) = shown.to_node(i, (x, y));
            let seen = shown.local(i, (x, y));
            let ev = DragEvent {
                x: fx,
                y: fy,
                bounds,
                phase: DragPhase::Move,
                pointer: id,
                kind,
                on_control,
            };
            let claimed = claiming(|| f(&ev));
            if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
                p.seen = seen;
                p.claimed |= claimed;
            }
            out = out.merge(Outcome::redraw());
        } else if panning && let Some(h) = hit {
            // A pan is one gesture from its contact down to its lift: its
            // moves are the fingers', the press its identity.
            let step = crate::compositor::WheelStep {
                point: (x, y),
                time: crate::clock::current_nanos(),
                precise: true,
                phase: crate::compositor::WheelPhase::Changed,
                gesture: Some(press),
            };
            out = out.merge(self.scroll_from(frame, h, step, delta.0, delta.1, id));
        }
        out
    }

    /// A contact lifted: what it turned out to be (the end of its drag, a
    /// release after its long press, a tap), then its raw `Ended`.
    fn contact_up(&mut self, frame: &Frame, text: &mut TextSystem, ev: &TouchEvent) -> Outcome {
        let id = PointerId::Contact(ev.id);
        let raw = self.raw_of(frame, id);
        let Some(k) = self.pointers.iter().position(|p| p.id == id) else {
            return Outcome::default();
        };
        let mut p = self.pointers.remove(k);
        self.write_pressed(frame);
        let (x, y) = (ev.x, ev.y);
        let gesture = p.gesture.as_mut().map(|g| g.up(x, y));
        let out = if let Some(grip) = p.drag.as_mut() {
            if let Some(i) = grip.live(frame, &self.shown(frame), drag_of) {
                let (fx, fy) = self.shown(frame).to_node(i, (x, y));
                (grip.handler)(&DragEvent {
                    x: fx,
                    y: fy,
                    bounds: grip.shown_box,
                    phase: DragPhase::End,
                    pointer: p.id,
                    kind: p.kind,
                    on_control: p.on_control,
                });
            }
            Outcome::redraw()
        } else if p.long {
            // A long press lifted elsewhere: a release there (a context
            // menu's item picked by the same contact).
            let pressed_at = p.hit.as_ref().and_then(|h| h.find(frame));
            match self.locate(frame, x, y) {
                (Some(h), ..) => Outcome::redraw().merge(self.release_at(frame, h, x, y, pressed_at)),
                _ => Outcome::redraw(),
            }
        } else if let Some(Gesture::Tap { x, y }) = gesture {
            self.tap(frame, text, ev, (x, y), (p.down.as_ref(), p.hit.as_ref()))
        } else {
            Outcome::redraw()
        };
        if let Some(f) = raw {
            f(ev);
        }
        out
    }

    /// The contact of `ev` tapped at `at`, having gone down on the node at
    /// `down` (gone since, or not). A tap is one gesture, recognised as it
    /// lifts and hit tested once, as Chrome's `GestureTap`: its focus,
    /// caret, mouse down, drag start and end, and click all go to the node
    /// under the tap point in the frame as it is now (above the top modal
    /// layer, by the reach rules), when that is the node it went down on or
    /// one above it: one that took the place of what it went down on, or
    /// came over it, gets nothing (Chrome sends the tap's click to what
    /// the down and up targets share; a child removed under the finger
    /// leaves the tap to its parent). It is a press and release of that
    /// contact's own: no other pointer's press, drag or claim is touched.
    fn tap(
        &mut self,
        frame: &Frame,
        text: &mut TextSystem,
        ev: &TouchEvent,
        (x, y): (f32, f32),
        (down, held): (Option<&NodePath>, Option<&Held>),
    ) -> Outcome {
        let gone = held.is_none();
        let (h, ..) = self.locate(frame, x, y);
        let on_its_own = h
            .and_then(|h| frame.nodes.get(h))
            .is_some_and(|n| down.is_some_and(|d| covers(&n.path, d, gone)));
        if !on_its_own {
            return Outcome::redraw();
        }
        // A pen was asked as it went down.
        let outside = if ev.kind == PointerKind::Pen {
            Outcome::default()
        } else {
            self.dismiss_outside(frame, h, self.outside_at(x, y, ev.kind, PointerButton::Primary))
        };
        let id = PointerId::Contact(ev.id);
        self.pointers.push(Pointer::new(id, ev.kind, (x, y)));
        let pressed = self.press_primary(frame, text, id, x, y);
        // Its click is the contact's: from where it went down, while that
        // is there; gone, from what is left above it, under the tap.
        if let Some(p) = self.pointers.iter_mut().find(|p| p.id == id) {
            p.down = down.cloned();
            if let Some(held) = held {
                p.hit = Some(held.clone());
            }
        }
        let up = self.release(frame, id, x, y);
        up.merge(outside).merge(pressed)
    }

    /// The system took a contact away: its raw phase, and what it dragged
    /// lets go, with its box as the frame has it now.
    fn contact_cancelled(&mut self, frame: &Frame, ev: &TouchEvent) -> Outcome {
        let id = PointerId::Contact(ev.id);
        if let Some(f) = self.raw_of(frame, id) {
            f(ev);
        }
        let Some(k) = self.pointers.iter().position(|p| p.id == id) else {
            return Outcome::default();
        };
        let mut p = self.pointers.remove(k);
        self.write_pressed(frame);
        match p.drag.as_mut() {
            Some(grip) => self
                .cancel_drag(frame, grip, (ev.x, ev.y), p.id, p.kind, p.on_control)
                .merge(Outcome::redraw()),
            None => Outcome::redraw(),
        }
    }

    /// Run the long presses due by now: on each contact still down whose
    /// press is, the nearest `on_long_press` where it went down, else
    /// `on_secondary_click`.
    fn long_presses(&mut self, frame: &Frame) -> Outcome {
        if !self.pointers.iter().any(|p| p.gesture.is_some_and(|g| g.waiting())) {
            return Outcome::default();
        }
        let now = clock_now();
        let mut out = Outcome::default();
        for k in 0..self.pointers.len() {
            let Some(p) = self.pointers.get_mut(k) else { break };
            let Some(Gesture::LongPress { x, y }) = p.gesture.as_mut().and_then(|g| g.due(now)) else {
                continue;
            };
            p.pressing = false;
            let hit = p.hit.as_ref().and_then(|h| h.find(frame));
            self.write_pressed(frame);
            // The node it went down on, as the frame has it now: gone
            // since, or below a modal layer opened since, nothing runs,
            // and its lift is no release (an item of that modal layer
            // under it then is not picked by a contact it never saw).
            let h = hit.filter(|&h| !self.layers.blocked(h));
            let long = h.and_then(|h| {
                let i = Reach::Gesture.target(frame, h, |b| b.handlers.long_press.is_some())?;
                Some((i, frame.behaviour(i)?.handlers.long_press.clone()?))
            });
            let ran = match long {
                Some((i, f)) => {
                    let ((fx, fy), bounds) = self.shown(frame).event(i, (x, y));
                    f(&ClickEvent {
                        x: fx,
                        y: fy,
                        bounds,
                        modifiers: self.modifiers,
                    });
                    Outcome::by(format!("long press ({fx:.0},{fy:.0}) -> {}", label(frame, i)))
                }
                None => self.secondary_at(frame, h, x, y),
            };
            // Held into a long press only when one ran (its own handler or
            // the secondary click it falls back to): then its lift is a
            // release, as an item of the menu that opened is picked.
            if ran.handled_by.is_some()
                && let Some(p) = self.pointers.get_mut(k)
            {
                p.long = true;
            }
            out = out.merge(ran);
        }
        out
    }

    /// After a layout pass (ADR-0024): read the frame's layers, apply the
    /// focus scopes' rules (focus back to where it was as a layer closes,
    /// into one that opens, kept in a trapped one), ask the layers focus
    /// came outside of to close, run due long presses, and bring the nodes'
    /// focus flags up to date. Whether anything moved or ran: another pass
    /// is due.
    pub fn settle(&mut self, tree: &LayoutTree) -> bool {
        let frame = tree.frame();
        self.take_in(frame);
        let fields = self.settle_fields(tree);
        self.layers.publish();
        let settled = self.focus.settle(frame, &self.layers);
        let mut ran = fields;
        for spec in settled.outside {
            ran |= spec.ask(&DismissReason::FocusOutside).is_some();
        }
        ran |= self.long_presses(frame).redraw;
        // A node disabled by this pass lets go of the drag or the touch it
        // held, now rather than at the next move.
        ran |= self.cancel_lost(frame).redraw;
        // Focus a layer rule or a post hook moved (a menu stepping to its
        // first item once laid out) shows in this frame (unchanged: no-op).
        self.sync_focus(tree);
        settled.moved || ran
    }

    /// Take what placing text fields said: the offsets they were laid out
    /// at, where clamped or moved to show their caret, and the reveals
    /// they took, kept by each field's handle (its scroll lives as long as
    /// it does). Whether any.
    /// What it keeps for text fields (room to settle them).
    #[cfg(test)]
    pub(crate) fn field_bookkeeping(&self) -> usize {
        self.fields.capacity()
    }

    fn settle_fields(&mut self, tree: &LayoutTree) -> bool {
        tree.swap_fields(&mut self.fields);
        for f in &self.fields {
            f.field.settled(f.pos, f.applied);
        }
        !self.fields.is_empty()
    }

    /// What the input at `i` shows (its text, shared with the frame, and
    /// its caret there), its text style and box, and how it takes edits.
    fn input_geometry(
        frame: &Frame,
        i: usize,
    ) -> Option<(std::sync::Arc<str>, ResolvedText, Bounds, usize, InputMode, bool)> {
        let node = frame.nodes.get(i)?;
        match &node.content {
            Content::Input {
                edit,
                text,
                multiline,
                mode,
                ..
            } => Some((
                std::sync::Arc::clone(&edit.text),
                text.clone(),
                laid_content_box(node),
                edit.caret,
                *mode,
                *multiline,
            )),
            _ => None,
        }
    }

    /// How the input at `i` takes edits, and whether it takes lines: what
    /// it does, none of where it is.
    fn input_mode(frame: &Frame, i: usize) -> Option<(InputMode, bool)> {
        match &frame.nodes.get(i)?.content {
            Content::Input { mode, multiline, .. } => Some((*mode, *multiline)),
            _ => None,
        }
    }

    /// The scroll model of field `i` (where the UI holds its offset, and
    /// how far it goes), which autoscroll steps: the scroll machinery's
    /// state, not where anything shows.
    fn field_scroll(frame: &Frame, i: usize) -> Option<crate::layout::Scroll> {
        laid_scroll(frame.nodes.get(i)?)
    }

    /// Queue autoscroll's next step a frame from now, in place of any queued.
    fn queue_autoscroll(&mut self) {
        if let Some(t) = self.autoscroll_timer.take() {
            crate::clock::cancel(t);
        }
        self.autoscroll_timer = Some(crate::clock::after(AUTOSCROLL_TICK, || {}));
    }

    /// Stop autoscrolling: the press released, the field blurred or the
    /// pointer back inside; its queued wakeup cancelled.
    fn stop_autoscroll(&mut self) {
        self.autoscrolled = None;
        if let Some(t) = self.autoscroll_timer.take() {
            crate::clock::cancel(t);
        }
    }

    /// Whether autoscroll has a wakeup queued.
    #[cfg(test)]
    pub(crate) fn autoscroll_queued(&self) -> bool {
        self.autoscroll_timer.is_some_and(crate::clock::is_pending)
    }

    /// A drag selecting in a text field, held with the pointer past its
    /// edge, scrolls it that way on a timer, faster the farther past, and
    /// extends the selection to what comes under the pointer, as Chrome
    /// autoscrolls; whether it did. Run as each frame starts.
    pub fn autoscroll(&mut self, frame: &Frame, text: &mut TextSystem) -> bool {
        let field = self
            .pointers
            .iter()
            .find(|p| p.id == PointerId::Mouse && p.pressing)
            .and_then(|p| p.selecting.as_ref())
            .and_then(|s| s.find(frame));
        let (Some(i), Some((x, y))) = (field, self.pointer) else {
            self.stop_autoscroll();
            return false;
        };
        let (Some(node), Some(true)) = (
            frame.nodes.get(i),
            frame
                .behaviour(i)
                .and_then(|b| b.field)
                .map(|f| Some(f.focus.id()) == focus::focused()),
        ) else {
            // Blurred under the drag: it stops.
            self.stop_autoscroll();
            return false;
        };
        let Some(scroll) = Self::field_scroll(frame, i) else {
            return false;
        };
        // Past the edge of the field as the frame on screen shows it, and
        // the text the selection reaches into scrolled as it shows: what
        // the compositor scrolled since the commit included.
        let shown = self.shown(frame);
        let (px, py) = shown.to_node(i, (x, y));
        let Some(field) = shown.field(i) else {
            self.stop_autoscroll();
            return false;
        };
        let a = field.area;
        let past = |p: f32, lo: f32, hi: f32| {
            if p < lo {
                p - lo
            } else if p > hi {
                p - hi
            } else {
                0.0
            }
        };
        let (dx, dy) = (past(px, a.x, a.right()), past(py, a.y, a.bottom()));
        if (dx, dy) == (0.0, 0.0) || scroll.max == (0.0, 0.0) {
            self.stop_autoscroll();
            return false;
        }
        let along = if node.look.text.direction == crate::style::Direction::Rtl {
            -dx
        } else {
            dx
        };
        // Not past an edge on an axis: no step along it (`signum` is 1 at 0).
        let sign = |d: f32| {
            if d > 0.0 {
                1.0
            } else if d < 0.0 {
                -1.0
            } else {
                0.0
            }
        };
        // At its limit every way it is pulled: no wakeup until something
        // moves, from the first tick on.
        let room = |d: f32, o: i64, end: i64| (d > 0.0 && o < end) || (d < 0.0 && o > 0);
        if !room(along, scroll.pos.0, scroll.end.0) && !room(dy, scroll.pos.1, scroll.end.1) {
            self.stop_autoscroll();
            return false;
        }
        let now = clock_now();
        let Some(dt) = self
            .autoscrolled
            .replace(now)
            .map(|t| now.saturating_sub(t).as_secs_f32().min(0.25))
        else {
            // Past the edge just now: its first step in a frame's time.
            self.queue_autoscroll();
            return false;
        };
        let step = |d: f32| sign(d) * (AUTOSCROLL_BASE + d.abs() * AUTOSCROLL_PER_PX) * dt;
        // Exactly ([`crate::compositor::motion`]), on the grid.
        let next = motion::clamp2(
            motion::add2(scroll.pos, motion::units2((step(along), step(dy)))),
            scroll.end,
        );
        if next == scroll.pos {
            // At its limit that way: no wakeup until something moves.
            if let Some(t) = self.autoscroll_timer.take() {
                crate::clock::cancel(t);
            }
            return false;
        }
        // The selection reaches what the scroll brings under the pointer,
        // kept in the box: the text as shown, moved by the step.
        let by = motion::sub2(next, scroll.pos);
        let moved = motion::px2(by);
        let rtl = node.look.text.direction == crate::style::Direction::Rtl;
        let scroll_by = (
            field.scrolled.0 + if rtl { moved.0 } else { -moved.0 },
            field.scrolled.1 - moved.1,
        );
        let it = input_text(
            text,
            &field.content,
            &field.text,
            a,
            field.multiline,
            field.focused,
            scroll_by,
        );
        let (hx, hy) = (px.clamp(a.x, a.right()), py.clamp(a.y, a.bottom()));
        let at = text.hit(&field.content, &it.t, it.width, hx - it.origin.0, hy - it.origin.1);
        let offsets = field.offsets;
        // Again in a frame's time.
        self.queue_autoscroll();
        self.scroll_by(frame, i, by);
        let Some(f) = frame.behaviour(i).and_then(|b| b.field) else {
            return true;
        };
        f.edit.update(|e| {
            let offset = offsets.value_offset(e.text(), at);
            e.place(offset, true);
        });
        true
    }

    fn place_caret(&self, frame: &Frame, text: &mut TextSystem, i: usize, x: f32, y: f32, extend: bool) {
        let Some(field) = frame.behaviour(i).and_then(|b| b.field) else {
            return;
        };
        // Hit what the frame the event was routed in showed ([`Shown::field`]):
        // its text, its type and its box as that frame showed them, not as
        // laid out since; a press on a field not yet focused finds its
        // text as cut short (over its ellipsis: the end), not the text its
        // focus will expand to; scrolled as it shows, the compositor's
        // scroll since the commit included. A field that frame showed no
        // text of (made since) is placed nothing. The offset hit falls in
        // the value as that frame showed it too (masked, a composition in
        // it: [`crate::edit::Offsets`]).
        let Some(shown) = self.shown(frame).field(i) else {
            return;
        };
        let it = input_text(
            text,
            &shown.content,
            &shown.text,
            shown.area,
            shown.multiline,
            shown.focused,
            shown.scrolled,
        );
        let at = text.hit(&shown.content, &it.t, it.width, x - it.origin.0, y - it.origin.1);
        field.edit.update(|e| {
            let offset = shown.offsets.value_offset(e.text(), at);
            e.place(offset, extend);
        });
    }

    /// A held key the platform sent again: as [`Self::key`], but Space
    /// presses the focused control no more.
    pub fn key_repeated(&mut self, frame: &Frame, text: &mut TextSystem, ev: &KeyEvent) -> Outcome {
        self.repeating = true;
        let out = self.key(frame, text, ev);
        self.repeating = false;
        out
    }

    /// A key went down: to the focused node and its ancestors, then
    /// Escape to the top layer that takes it, then the top layer's root.
    pub fn key(&mut self, frame: &Frame, text: &mut TextSystem, ev: &KeyEvent) -> Outcome {
        self.take_in(frame);
        acts_by(Modality::Keyboard);
        self.modifiers = ev.modifiers;
        let focused = focus::focused()
            .and_then(|id| focused_node(frame, id))
            .filter(|&i| !self.layers.blocked(i));
        let own = focused.and_then(|i| frame.behaviour(i)?.handlers.key_down.clone().map(|f| (i, f)));
        if ev.is(NamedKey::Tab) {
            // The focused node's own handler may take Tab before focus
            // moves, as a DOM `keydown` listener may prevent it (a tags
            // input adding its draft on Tab keeps focus); its ancestors
            // never see it.
            if let Some((i, f)) = &own
                && f(ev)
            {
                return Outcome::by(format!("key {:?} -> {}", ev.key, label(frame, *i)));
            }
            return self.tab(frame, ev.modifiers.shift);
        }
        // The focused node's own handler sees the key before its input edits
        // with it, as a DOM `keydown` listener does before the default action
        // (Enter sends from a textarea rather than breaking the line).
        if let Some((i, f)) = &own
            && f(ev)
        {
            return Outcome::by(format!("key {:?} -> {}", ev.key, label(frame, *i)));
        }
        if let Some(i) = focused
            && let Some(out) = self.edit(frame, text, i, ev)
        {
            if let Some(f) = frame.behaviour(i).and_then(|b| b.field) {
                f.reveal();
            }
            return out;
        }
        let asked = own.map(|(i, _)| i);
        let chain = |start: usize| -> Option<Outcome> {
            // A disabled node's handler does not run, nor one not shown.
            for i in frame
                .ancestors(start)
                .filter(|&i| Some(i) != asked && !disabled(frame, i) && frame.shown(i))
            {
                if let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.key_down.clone())
                    && f(ev)
                {
                    return Some(Outcome::by(format!("key {:?} -> {}", ev.key, label(frame, i))));
                }
            }
            None
        };
        // A control inside that takes Escape (a filter clearing its text)
        // keeps it from the layer.
        if let Some(out) = focused.and_then(chain) {
            return out;
        }
        if ev.is(NamedKey::Escape)
            && !self.composing_now()
            && let Some(out) = self.escape(frame)
        {
            return out;
        }
        // The top layer shown, and the floor if shown: a root not shown
        // takes no key, nor keeps one from the roots below it.
        let top_root = frame
            .layers
            .iter()
            .rev()
            .map(|l| l.root)
            .find(|&r| frame.shown(r) && !self.layers.blocked(r));
        let focused_layer = focused.and_then(|i| frame.nodes.get(i)).map(|n| n.layer);
        let floor = Some(self.layers.floor).filter(|&f| frame.shown(f));
        let start = match top_root {
            Some(root) if frame.nodes.get(root).map(|n| n.layer) != focused_layer => Some(root),
            _ if focused.is_none() => floor,
            _ => None,
        };
        if let Some(out) = start.and_then(chain) {
            return out;
        }
        // A key no one on the way took goes to the window's listeners.
        for (i, b) in frame.behaviour.iter().enumerate() {
            if let Some(f) = &b.handlers.window_key
                && frame.shown(i)
                && !b.disabled
                && !self.layers.blocked(i)
                && f(ev)
            {
                return Outcome::by(format!("window key {:?} -> {}", ev.key, label(frame, i)));
            }
        }
        // Enter and Space press the focused control; a held Space only
        // once (a browser presses a button on Space's release).
        let space = ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ");
        let activates = ev.is(NamedKey::Enter) || (space && !self.repeating);
        if activates
            && let Some(i) = focused
            && self.press(frame, i, ev.modifiers)
        {
            return Outcome::by(format!("key {:?} pressed {}", ev.key, label(frame, i)));
        }
        Outcome::default()
    }

    /// Run the click handler of the node at `i` as a click at its centre
    /// does, for a press that is not the pointer's; whether it had one (an
    /// unavailable node's does not run).
    fn press(&self, frame: &Frame, i: usize, modifiers: Modifiers) -> bool {
        let Some(f) = frame
            .behaviour(i)
            .filter(|b| Reach::Activation.runs_on(b))
            .and_then(|b| b.handlers.click.clone())
        else {
            return false;
        };
        // At the middle of its box as the frame on screen shows it.
        let b = self.shown(frame).bounds(i);
        f(&ClickEvent {
            x: b.x + b.width / 2.0,
            y: b.y + b.height / 2.0,
            bounds: b,
            modifiers,
        });
        true
    }

    /// Do what assistive technology asks of a node: press it, focus it, or
    /// set a text input's value.
    pub fn act(&mut self, frame: &Frame, text: &mut TextSystem, request: &ActionRequest) -> Outcome {
        self.take_in(frame);
        let Some(i) = access::target(frame, request.target_node).filter(|&i| !self.layers.blocked(i)) else {
            return Outcome::default();
        };
        if request.action == Action::ScrollIntoView {
            return if self.scroll_into_view(frame, i) {
                Outcome::by(format!("ScrollIntoView -> {}", label(frame, i)))
            } else {
                Outcome::default()
            };
        }
        if disabled(frame, i) {
            return Outcome::default();
        }
        let Some(b) = frame.behaviour(i) else {
            return Outcome::default();
        };
        let done = match (request.action, &request.data) {
            (Action::Click, _) => self.press(frame, i, Modifiers::default()),
            (Action::Focus, _) => b.focus.is_some_and(|h| {
                focus::set_focus(Some(h.id()), true);
                self.state.focused = Some(h.id());
                true
            }),
            // A value in a range steps as its keys step it.
            (Action::Increment | Action::Decrement, _) if b.semantics.range.is_some() => {
                let key = if request.action == Action::Increment {
                    crate::events::NamedKey::ArrowUp
                } else {
                    crate::events::NamedKey::ArrowDown
                };
                b.handlers.key_down.as_ref().is_some_and(|h| {
                    h(&KeyEvent {
                        key: crate::events::Key::Named(key),
                        text: None,
                        modifiers: Modifiers::default(),
                    })
                })
            }
            (Action::SetValue, Some(ActionData::Value(v))) => b.field.as_ref().is_some_and(|f| {
                f.set_text(v.as_ref());
                f.reveal();
                true
            }),
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(sel))) => {
                let shown = access::selection_in(frame, i, sel, text);
                let mode = Self::input_mode(frame, i).map(|g| g.0);
                match (b.field.as_ref(), shown, mode) {
                    (Some(f), Some((anchor, caret)), Some(mode)) => {
                        f.edit.update(|e| {
                            let (anchor, caret) = (mode.value_offset(e, anchor), mode.value_offset(e, caret));
                            e.end_composition();
                            e.anchor = anchor;
                            e.caret = caret;
                        });
                        f.reveal();
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if done {
            Outcome::by(format!("{:?} -> {}", request.action, label(frame, i)))
        } else {
            Outcome::default()
        }
    }

    /// Scroll the containers around node `i` until it shows, as a screen
    /// reader asks (`ScrollIntoView`): a virtual list moves to the row it
    /// is in (built or only pinned); any other container by as little as
    /// shows it. Whether anything scrolled.
    fn scroll_into_view(&mut self, frame: &Frame, i: usize) -> bool {
        let Some(b) = frame.nodes.get(i).map(laid_bounds) else {
            return false;
        };
        let mut child = i;
        let mut scrolled = false;
        for a in frame.ancestors(i).skip(1) {
            let Some(node) = frame.nodes.get(a) else { break };
            let (Some(scroll), handle) = (laid_scroll(node), frame.behaviour(a).and_then(|b| b.scroll.as_ref())) else {
                child = a;
                continue;
            };
            let key = frame
                .nodes
                .get(child)
                .and_then(|c| c.path.0.last())
                .and_then(|seg| seg.element_key());
            if let (Some(h), Some(k)) = (handle, key)
                && let Some(moved) = h.reveal(k)
            {
                scrolled |= moved;
                child = a;
                continue;
            }
            let c = laid_content_box(node);
            let delta = |lo: f32, hi: f32, start: f32, end: f32| {
                if end - start > hi - lo || start < lo {
                    start - lo
                } else if end > hi {
                    end - hi
                } else {
                    0.0
                }
            };
            // Exactly ([`crate::compositor::motion`]): by what it is off by,
            // on the grid, so an axis it is not off along does not move.
            let by = motion::units2((
                delta(c.x, c.right(), b.x, b.right()),
                delta(c.y, c.bottom(), b.y, b.bottom()),
            ));
            let next = motion::clamp2(motion::add2(scroll.pos, by), scroll.end);
            if next != scroll.pos {
                self.scroll_by(frame, a, motion::sub2(next, scroll.pos));
                scrolled = true;
            }
            child = a;
        }
        scrolled
    }

    /// Apply what an input method did to the focused text input.
    pub fn ime(&mut self, frame: &Frame, ev: &ImeEvent) -> Outcome {
        let Some(i) = focus::focused().and_then(|id| focused_node(frame, id)) else {
            return Outcome::default();
        };
        let Some((mode, multiline)) = Self::input_mode(frame, i) else {
            // Not a text input: the nearest `on_ime` takes it.
            if let Some((at, f)) = Self::ime_handler(frame, i) {
                f(ev);
                return Outcome::by(format!("ime -> {}", label(frame, at)));
            }
            return Outcome::default();
        };
        let Some(field) = frame.behaviour(i).and_then(|b| b.field).filter(|_| mode.composes()) else {
            return Outcome::default();
        };
        // In place: the value is not copied to apply it.
        let mut composing = false;
        field.edit.update(|e| {
            match ev {
                ImeEvent::Preedit(p) => e.compose(p.clone()),
                ImeEvent::Commit(s) if multiline => e.commit(s),
                ImeEvent::Commit(s) => e.commit(&s.replace(['\n', '\r'], " ")),
            }
            composing = e.preedit().is_some();
        });
        self.composing = composing.then_some(field);
        field.reveal();
        Outcome::by(format!("ime -> edit {}", label(frame, i)))
    }

    /// Where an input method may type now: the focused input's caret when
    /// that input takes composed text. A composition left in an input that
    /// lost focus is dropped, as a browser ends it on blur.
    pub fn ime_target(&mut self, frame: &Frame, text: &mut TextSystem) -> Option<ImeTarget> {
        let focused = focus::focused();
        // A field gone (its scope disposed) is dropped, never touched.
        if let Some(field) = self
            .composing
            .take_if(|f| !f.is_alive() || Some(f.focus.id()) != focused)
            && field.is_alive()
        {
            field.edit.update(crate::edit::TextEdit::end_composition);
        }
        let id = focused?;
        let i = focused_node(frame, id)?;
        // Borrowed apart from the kept text below.
        let shown = Shown::new(frame, self.shown.as_deref(), &self.hit_keys);
        let Some((mode, _)) = Self::input_mode(frame, i) else {
            // A node an input method types into through `on_ime`: its
            // candidates go by the focused node.
            Self::ime_handler(frame, i)?;
            let area = shown.rect_on_screen(i, shown.bounds(i));
            return Some(ImeTarget { field: id, area });
        };
        if !mode.composes() {
            return None;
        }
        // The caret the frame on screen shows, in its text as it shows it,
        // scrolled as it shows (the compositor's scroll included): the
        // candidate window goes where the caret is drawn, and moves when a
        // frame that moved it does. A field focused since that frame,
        // which showed none of its text, has it by the field's box.
        let Some(field) = shown.field(i) else {
            let area = shown.rect_on_screen(i, shown.bounds(i));
            return Some(ImeTarget { field: id, area });
        };
        let area = field.area;
        // What its text alone decides (where the text sits in the box,
        // the caret and its selection or composition in the text) is the
        // record's, kept while the screen shows that record: an unchanged
        // field is not laid out again on each wake (ADR-0015). Where the
        // box is and how far it scrolled come from the frame each time.
        let kept = self
            .ime_text
            .as_ref()
            .filter(|k| k.focus == id && field.record.as_ref().is_some_and(|r| r.ptr_eq(&k.record)));
        let (rel, bounds) = match kept {
            Some(k) => (k.rel, k.caret),
            None => {
                let (rel, caret) = ime_text_of(text, &field);
                self.ime_text = field.record.clone().map(|record| ImeText {
                    record,
                    focus: id,
                    rel,
                    caret,
                });
                (rel, caret)
            }
        };
        let origin = (area.x + field.scrolled.0 + rel.0, area.y + field.scrolled.1 + rel.1);
        let inflate = 3.0 * 0.6 * field.text.size;
        let left = (origin.0 + bounds.x - inflate).max(area.x);
        let right = (origin.0 + bounds.right() + inflate).min(area.right()).max(left);
        let laid = Bounds {
            x: left,
            y: origin.1 + bounds.y,
            width: right - left,
            height: bounds.height,
        };
        Some(ImeTarget {
            field: id,
            area: shown.rect_on_screen(i, laid),
        })
    }

    /// The nearest `on_ime` of the focused node `i` (itself first).
    fn ime_handler(frame: &Frame, i: usize) -> Option<(usize, crate::element::Handler<ImeEvent>)> {
        frame
            .ancestors(i)
            .find_map(|a| frame.behaviour(a).and_then(|b| b.handlers.ime.clone()).map(|f| (a, f)))
    }

    /// Move focus by Tab (Shift+Tab when `back`) through the focus scopes
    /// ([`FocusManager::tab`]).
    fn tab(&mut self, frame: &Frame, back: bool) -> Outcome {
        let Some(id) = FocusManager::tab(frame, &self.layers, back) else {
            return Outcome::default();
        };
        focus::set_focus(Some(id), true);
        self.state.focused = Some(id);
        let at = focused_node(frame, id).map(|i| label(frame, i)).unwrap_or_default();
        Outcome::by(format!("tab -> focus {at}"))
    }

    /// Whether an input method is composing in the focused field.
    fn composing_now(&self) -> bool {
        self.composing.is_some_and(|f| {
            f.is_alive() && Some(f.focus.id()) == focus::focused() && f.edit.with_untracked(|e| e.preedit().is_some())
        })
    }

    /// Escape to the top layer that takes it (above the top modal one):
    /// it alone is asked, whatever it answers.
    fn escape(&mut self, frame: &Frame) -> Option<Outcome> {
        let e = self.layers.live().find(|e| e.spec.policy.escape)?;
        let (spec, node) = (std::rc::Rc::clone(&e.spec), e.node);
        let answer = spec.ask(&DismissReason::Escape).unwrap_or(Dismiss::Veto);
        Some(Outcome::by(format!("Escape -> {answer:?} {}", label(frame, node))))
    }

    /// Apply `ev` to the text input at `i`; `None` when the input does not take this key.
    fn edit(&mut self, frame: &Frame, text: &mut TextSystem, i: usize, ev: &KeyEvent) -> Option<Outcome> {
        let field = frame.behaviour(i).and_then(|b| b.field)?;
        let (content, t, area, caret_at, mode, multiline) = Self::input_geometry(frame, i)?;
        let shift = ev.modifiers.shift;
        if mode.read_only && Self::changes_value(ev, multiline) {
            return None;
        }
        // A password is never copied out, as browsers keep it.
        let copyable = mode.input_type != InputType::Password;
        // Where the caret goes by `step` through the text as shown (a
        // password's bullets, a composition), by its graphemes, words and
        // lines as laid out, in the value's offsets.
        let it = input_text(text, &content, &t, area, multiline, true, (0.0, 0.0));
        let word = ev.modifiers.word();
        // ⌘ with an arrow goes to the line's or the text's end on macOS.
        let line = cfg!(target_os = "macos") && ev.modifiers.command;
        // What the key does, read off the edit as it is (borrowed, its text
        // never copied), then done to it in place: a caret moved writes
        // the caret alone.
        let op = field.edit.with_untracked(|e| {
            let mut to = |s: Step| mode.value_offset(e, text.step(&content, &it.t, it.width, caret_at, s));
            let collapse = !shift && !word && !line && e.has_selection();
            Some(match &ev.key {
                Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::Delete) if e.has_selection() => {
                    EditOp::Insert(Cow::Borrowed(""))
                }
                Key::Named(NamedKey::Backspace) => EditOp::DeleteTo(to(if line {
                    Step::LineStart
                } else if word {
                    Step::DeleteWordBack
                } else {
                    Step::DeleteBack
                })),
                Key::Named(NamedKey::Delete) => EditOp::DeleteTo(to(if line {
                    Step::LineEnd
                } else if word {
                    Step::DeleteWordForward
                } else {
                    Step::DeleteForward
                })),
                Key::Named(NamedKey::ArrowLeft) if collapse => EditOp::Place(e.selection().0, false),
                Key::Named(NamedKey::ArrowRight) if collapse => EditOp::Place(e.selection().1, false),
                Key::Named(NamedKey::ArrowLeft) => EditOp::Place(
                    to(if line {
                        Step::LineStart
                    } else if word {
                        Step::WordLeft
                    } else {
                        Step::Left
                    }),
                    shift,
                ),
                Key::Named(NamedKey::ArrowRight) => EditOp::Place(
                    to(if line {
                        Step::LineEnd
                    } else if word {
                        Step::WordRight
                    } else {
                        Step::Right
                    }),
                    shift,
                ),
                Key::Named(NamedKey::Home) if ev.modifiers.primary() => EditOp::Place(0, shift),
                Key::Named(NamedKey::End) if ev.modifiers.primary() => EditOp::Place(e.text().len(), shift),
                Key::Named(NamedKey::Home) => EditOp::Place(to(Step::LineStart), shift),
                Key::Named(NamedKey::End) => EditOp::Place(to(Step::LineEnd), shift),
                Key::Named(NamedKey::ArrowUp) if multiline && line => EditOp::Place(0, shift),
                Key::Named(NamedKey::ArrowDown) if multiline && line => EditOp::Place(e.text().len(), shift),
                Key::Named(NamedKey::ArrowUp) if multiline => EditOp::Place(to(Step::Up), shift),
                Key::Named(NamedKey::ArrowDown) if multiline => EditOp::Place(to(Step::Down), shift),
                Key::Named(NamedKey::Enter) if multiline => EditOp::Insert(Cow::Borrowed("\n")),
                Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("a") => EditOp::SelectAll,
                Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("c") => {
                    if copyable {
                        crate::clipboard::copy(e.selected_text());
                    }
                    EditOp::Keep
                }
                Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("x") => {
                    if copyable && crate::clipboard::copy(e.selected_text()) {
                        EditOp::Insert(Cow::Borrowed(""))
                    } else {
                        EditOp::Keep
                    }
                }
                Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("v") => {
                    let pasted = crate::clipboard::paste().unwrap_or_default();
                    EditOp::Insert(Cow::Owned(if multiline {
                        pasted
                    } else {
                        pasted.replace(['\n', '\r'], " ")
                    }))
                }
                _ => EditOp::Insert(Cow::Borrowed(typed(ev)?)),
            })
        })?;
        let changes = field.edit.with_untracked(|e| match &op {
            EditOp::Keep => false,
            EditOp::Place(at, extend) => *at != e.caret || (!extend && *at != e.anchor),
            EditOp::SelectAll => e.anchor != 0 || e.caret != e.text().len(),
            EditOp::DeleteTo(at) => e.has_selection() || *at != e.caret,
            EditOp::Insert(s) => !s.is_empty() || e.has_selection(),
        });
        if changes {
            field.edit.update(|e| match op {
                EditOp::Keep => {}
                EditOp::Place(at, extend) => e.place(at, extend),
                EditOp::SelectAll => e.select_all(),
                EditOp::DeleteTo(at) => e.delete_to(at),
                EditOp::Insert(s) => e.insert(&s),
            });
        }
        Some(Outcome::by(format!("key {:?} -> edit {}", ev.key, label(frame, i))))
    }

    /// Whether `ev` would change an input's value (which a read-only one
    /// leaves to its ancestors) rather than move its caret or copy.
    fn changes_value(ev: &KeyEvent, multiline: bool) -> bool {
        match &ev.key {
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => true,
            Key::Named(NamedKey::Enter) => multiline,
            Key::Character(c) if ev.modifiers.primary() => c.eq_ignore_ascii_case("x") || c.eq_ignore_ascii_case("v"),
            _ => typed(ev).is_some(),
        }
    }
}

/// What a key does to a text input's edit ([`InputState::edit`]).
enum EditOp<'k> {
    /// Nothing (a copy).
    Keep,
    /// The caret to a byte, the selection extended or not.
    Place(usize, bool),
    SelectAll,
    /// The selection, or from the caret to a byte, deleted.
    DeleteTo(usize),
    /// The selection replaced by this.
    Insert(Cow<'k, str>),
}

/// The text `ev` types into an input, if it types any.
fn typed(ev: &KeyEvent) -> Option<&str> {
    ev.text
        .as_deref()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .filter(|_| !ev.modifiers.primary() && !ev.modifiers.control)
}

/// The smallest box holding `a` and `b`.
fn union(a: Bounds, b: Bounds) -> Bounds {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Bounds {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}

/// The frame node holding focus `id`.
fn focused_node(frame: &Frame, id: FocusId) -> Option<usize> {
    focus::node_of(frame, id)
}

/// How often a drag held past a text field's edge scrolls it: a frame.
const AUTOSCROLL_TICK: std::time::Duration = std::time::Duration::from_millis(16);
/// How fast it scrolls (logical pixels a second): a base, and more for
/// each pixel the pointer is past the edge, as Chrome speeds up.
const AUTOSCROLL_BASE: f32 = 60.0;
const AUTOSCROLL_PER_PX: f32 = 12.0;

/// Now on the frame clock.
fn clock_now() -> std::time::Duration {
    std::time::Duration::try_from_secs_f32(crate::clock::current_seconds().max(0.0)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::TextField;
    use crate::element::{IntoElement, text_input};
    use crate::inspect::OUTSIDE;
    use crate::layout::layout;
    use crate::styled::Styled;

    fn key(c: &str, primary: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Character(c.into()),
            text: Some(c.to_owned()),
            modifiers: Modifiers {
                command: primary,
                control: primary,
                ..Modifiers::default()
            },
        }
    }

    fn named(k: NamedKey, word: bool, shift: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Named(k),
            text: None,
            modifiers: Modifiers {
                shift,
                alt: word && cfg!(target_os = "macos"),
                control: word && !cfg!(target_os = "macos"),
                ..Modifiers::default()
            },
        }
    }

    type Log = std::rc::Rc<std::cell::RefCell<Vec<String>>>;

    /// `d` with a handler of every kind the pointer and the keyboard run,
    /// each writing `who` and the kind to `log`.
    fn every_handler(d: crate::element::Div, who: &'static str, log: &Log) -> crate::element::Div {
        let put = |what: &'static str| {
            let log = log.clone();
            move || log.borrow_mut().push(format!("{who} {what}"))
        };
        let (click, down, drag, touch, long, secondary) = (
            put("click"),
            put("mouse down"),
            put("drag"),
            put("touch"),
            put("long press"),
            put("secondary click"),
        );
        d.on_click(move |_| click())
            .on_mouse_down(move |_| down())
            .on_drag(move |e| {
                if e.phase == DragPhase::Start {
                    drag();
                }
            })
            .on_touch(move |e| {
                if e.phase == TouchPhase::Started {
                    touch();
                }
            })
            .on_long_press(move |_| long())
            .on_secondary_click(move |_| secondary())
    }

    fn taken(log: &Log) -> Vec<String> {
        std::mem::take(&mut *log.borrow_mut())
    }

    fn finger(phase: TouchPhase, x: f32, y: f32) -> TouchEvent {
        TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        }
    }

    /// Hold a finger at `(x, y)` past the long-press time and lift it.
    fn hold(ui: &mut InputState, frame: &Frame, text: &mut TextSystem, x: f32, y: f32) {
        ui.touch(frame, text, &finger(TouchPhase::Started, x, y));
        let later = crate::clock::current_seconds() + crate::gesture::LONG_PRESS.as_secs_f32() + 0.1;
        crate::clock::advance_to(std::time::Duration::from_secs_f32(later));
        ui.long_presses(frame);
        ui.touch(frame, text, &finger(TouchPhase::Ended, x, y));
    }

    /// The root owns every gesture; an unavailable child (with a child of
    /// its own) at y 0..40, a disabled one at 40..80, the root's own box at
    /// 80..100. Each child has every handler too.
    fn blocked_tree(log: &Log, focus: crate::focus::FocusHandle) -> crate::element::Element {
        let unavailable = every_handler(crate::element::div(), "unavailable", log)
            .unavailable(true)
            .track_focus(focus)
            .width(100.0)
            .height(40.0)
            .child(crate::element::div().width(100.0).height(20.0));
        let disabled = every_handler(crate::element::div(), "disabled", log)
            .disabled(true)
            .width(100.0)
            .height(40.0)
            .child(crate::element::div().width(100.0).height(20.0));
        every_handler(crate::element::div(), "root", log)
            .width(400.0)
            .height(100.0)
            .child(unavailable)
            .child(disabled)
            .into_element()
    }

    #[test]
    fn an_unavailable_node_blocks_activation_but_passes_gestures_to_its_ancestors() {
        let log = Log::default();
        let focus = crate::focus::FocusHandle::new();
        let root = blocked_tree(&log, focus);
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );

        // A press and a click inside it: the root's press and drag run, as
        // gestures; no click, on it or through it; its own press handlers
        // do not, but the press focuses it.
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.mouse_up(&frame, 10.0, 10.0);
        assert_eq!(taken(&log), ["root mouse down", "root drag"]);
        assert_eq!(focus::focused(), Some(focus.id()), "a press focuses it");
        // On itself, beside its child: the same.
        ui.mouse_down(&frame, &mut text, 10.0, 30.0);
        ui.mouse_up(&frame, 10.0, 30.0);
        assert_eq!(taken(&log), ["root mouse down", "root drag"]);

        // A secondary click opens the root's context menu.
        ui.button_down(&frame, 10.0, 10.0, PointerButton::Secondary);
        assert_eq!(taken(&log), ["root secondary click"]);

        // A tap: the root's touch, press and drag; still no click.
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["root touch", "root mouse down", "root drag"]);

        // A long press: the root's, as its secondary click is.
        hold(&mut ui, &frame, &mut text, 10.0, 10.0);
        assert_eq!(taken(&log), ["root touch", "root long press"]);

        // A pan drags the root from it.
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 60.0, 10.0));
        assert_eq!(taken(&log), ["root touch", "root drag"]);

        // Enter, Space and a screen reader's Click activate nothing.
        focus.focus();
        ui.sync_focus(&crate::layout::LayoutTree::default());
        for k in [NamedKey::Enter, NamedKey::Space] {
            ui.key(&frame, &mut text, &named(k, false, false));
        }
        let i = focused_node(&frame, focus.id()).expect("the unavailable node");
        let id = access::id_of(&frame.nodes[i].path);
        ui.act(
            &frame,
            &mut text,
            &ActionRequest {
                action: Action::Click,
                target_tree: accesskit::TreeId::ROOT,
                target_node: id,
                data: None,
            },
        );
        assert_eq!(
            taken(&log),
            Vec::<String>::new(),
            "no activation on an unavailable node"
        );

        // Beside it, the root takes all of it, its click too.
        ui.mouse_down(&frame, &mut text, 10.0, 90.0);
        ui.mouse_up(&frame, 10.0, 90.0);
        assert_eq!(taken(&log), ["root mouse down", "root drag", "root click"]);
    }

    #[test]
    fn a_disabled_node_runs_nothing_but_passes_gestures_to_its_ancestors() {
        let log = Log::default();
        let root = blocked_tree(&log, crate::focus::FocusHandle::new());
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = laid_out(&root, &ui, &mut text);
        focus::set_focus(None, false);
        // On it (y 50) and on its child, disabled with it (y 70).
        for y in [50.0, 70.0] {
            ui.mouse_down(&frame, &mut text, 10.0, y);
            ui.mouse_up(&frame, 10.0, y);
            assert_eq!(
                taken(&log),
                ["root mouse down", "root drag"],
                "a press, no click, at y {y}"
            );
            ui.button_down(&frame, 10.0, y, PointerButton::Secondary);
            assert_eq!(taken(&log), ["root secondary click"]);
            ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, y));
            ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, y));
            assert_eq!(
                taken(&log),
                ["root touch", "root mouse down", "root drag"],
                "a tap, no click"
            );
            hold(&mut ui, &frame, &mut text, 10.0, y);
            assert_eq!(taken(&log), ["root touch", "root long press"]);
            ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, y));
            ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 60.0, y));
            ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 60.0, y));
            assert_eq!(taken(&log), ["root touch", "root drag"], "a pan drags the root");
        }
        assert_eq!(focus::focused(), None, "a press on a disabled node moves no focus");
    }

    /// A notched wheel's turn at `point`, now.
    fn notched(point: (f32, f32)) -> crate::compositor::WheelStep {
        crate::compositor::WheelStep {
            point,
            time: crate::clock::current_nanos(),
            ..crate::compositor::WheelStep::default()
        }
    }

    /// The frame `root` lays out to in 400 by 100.
    fn laid_out(root: &crate::element::Element, ui: &InputState, text: &mut TextSystem) -> Frame {
        layout(
            root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            text,
        )
    }

    fn click_of(frame: &Frame, i: usize) -> ActionRequest {
        ActionRequest {
            action: Action::Click,
            target_tree: accesskit::TreeId::ROOT,
            target_node: access::id_of(&frame.nodes[i].path),
            data: None,
        }
    }

    #[test]
    fn disabled_is_inherited_by_the_subtree_as_a_disabled_fieldset_disables_its_controls() {
        let log = Log::default();
        let (first, inner, last) = (
            crate::focus::FocusHandle::new(),
            crate::focus::FocusHandle::new(),
            crate::focus::FocusHandle::new(),
        );
        let put = |what: &'static str| {
            let log = log.clone();
            move || log.borrow_mut().push(what.to_owned())
        };
        let (click, key, window_key) = (put("click"), put("key"), put("window key"));
        let control = crate::element::div()
            .role(accesskit::Role::Button)
            .track_focus(inner)
            .on_click(move |_| click())
            .on_key_down(move |_| {
                key();
                true
            })
            .on_window_key(move |_| {
                window_key();
                true
            })
            .width(100.0)
            .height(40.0)
            .child(crate::element::div().width(50.0).height(20.0));
        let root = crate::element::div()
            .display(crate::style::Display::Flex)
            .width(400.0)
            .height(100.0)
            .child(crate::element::div().track_focus(first).width(50.0).height(40.0))
            .child(
                crate::element::div()
                    .disabled(true)
                    .width(100.0)
                    .height(40.0)
                    .child(control),
            )
            .child(crate::element::div().track_focus(last).width(50.0).height(40.0))
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = laid_out(&root, &ui, &mut text);
        let control = hit(&frame, 60.0, 30.0).expect("the control's child");
        let control = frame.nodes[control].parent.expect("the control");

        // Tab goes past it.
        focus::set_focus(Some(first.id()), true);
        ui.key(&frame, &mut text, &named(NamedKey::Tab, false, false));
        assert_eq!(
            focus::focused(),
            Some(last.id()),
            "Tab skips a control in a disabled subtree"
        );
        // It takes no focus, so no keys; Enter, Space and its window key run nothing.
        inner.focus();
        ui.sync_focus(&crate::layout::LayoutTree::default());
        assert_eq!(focused_node(&frame, inner.id()), None, "it holds no focus");
        for k in [NamedKey::Enter, NamedKey::Space, NamedKey::ArrowDown] {
            ui.key(&frame, &mut text, &named(k, false, false));
        }
        // A screen reader's Click and Focus do nothing; a press clicks nothing.
        ui.act(&frame, &mut text, &click_of(&frame, control));
        ui.act(
            &frame,
            &mut text,
            &ActionRequest {
                action: Action::Focus,
                ..click_of(&frame, control)
            },
        );
        ui.mouse_down(&frame, &mut text, 60.0, 30.0);
        ui.mouse_up(&frame, 60.0, 30.0);
        assert_eq!(
            taken(&log),
            Vec::<String>::new(),
            "nothing runs inside a disabled subtree"
        );

        // AccessKit reads it and what it holds as disabled, and offers no Click.
        let tree = access::tree(&frame, None, 1.0, "test", &mut text);
        let node = |i: usize| {
            let id = access::id_of(&frame.nodes[i].path);
            tree.nodes.iter().find(|(n, _)| *n == id).map(|(_, n)| n.clone())
        };
        let c = node(control).expect("the control in the tree");
        assert!(c.is_disabled(), "the control is read as disabled");
        assert!(!c.supports_action(Action::Click), "and offers no Click");
        assert!(!c.supports_action(Action::Focus), "nor Focus");
        let behaviour = frame
            .behaviour(hit(&frame, 60.0, 30.0).expect("its child"))
            .expect("behaviour");
        assert!(behaviour.disabled, "its child is disabled too");
    }

    #[test]
    fn a_drag_from_a_disabled_button_inside_a_drawer_still_drags_the_drawer() {
        let log = Log::default();
        let phases = |who: &'static str| {
            let log = log.clone();
            move |e: &DragEvent| log.borrow_mut().push(format!("{who} {:?}", e.phase))
        };
        let own = log.clone();
        let button = crate::element::div()
            .role(accesskit::Role::Button)
            .disabled(true)
            .on_click(move |_| own.borrow_mut().push("button click".to_owned()))
            .on_drag(phases("button"))
            .width(100.0)
            .height(40.0);
        let root = crate::element::div()
            .on_drag(phases("drawer"))
            .width(400.0)
            .height(100.0)
            .child(button)
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = laid_out(&root, &ui, &mut text);
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.pointer_moved(&frame, &mut text, 10.0, 60.0);
        ui.mouse_up(&frame, 10.0, 60.0);
        assert_eq!(
            taken(&log),
            ["drawer Start", "drawer Move", "drawer End"],
            "by the mouse"
        );
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 10.0, 60.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 60.0));
        assert_eq!(
            taken(&log),
            ["drawer Start", "drawer Move", "drawer End"],
            "by a finger"
        );
    }

    /// An `on_touch` box at 0..100 by 0..40, disabled or not, inside a root
    /// with an `on_touch` of its own; each writes the phases it gets.
    fn touched(log: &Log, disabled: bool) -> crate::element::Element {
        let phases = |who: &'static str| {
            let log = log.clone();
            move |e: &TouchEvent| log.borrow_mut().push(format!("{who} {:?}", e.phase))
        };
        crate::element::div()
            .on_touch(phases("root"))
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .disabled(disabled)
                    .on_touch(phases("box"))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_contact_keeps_its_touch_target_and_cancels_it_once_when_it_is_disabled() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let enabled = laid_out(&touched(&log, false), &ui, &mut text);
        let disabled = laid_out(&touched(&log, true), &ui, &mut text);

        // Captured: moved off the box, the box still gets its phases.
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Moved, 10.0, 80.0));
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Ended, 10.0, 80.0));
        assert_eq!(taken(&log), ["box Started", "box Moved", "box Ended"]);

        // Disabled while held (a number field's button held to its max):
        // the box gets `Cancelled` once, then nothing, and the root, which
        // never saw it start, gets nothing either.
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&disabled, &mut text, &finger(TouchPhase::Moved, 12.0, 10.0));
        ui.touch(&disabled, &mut text, &finger(TouchPhase::Moved, 14.0, 10.0));
        ui.touch(&disabled, &mut text, &finger(TouchPhase::Ended, 14.0, 10.0));
        assert_eq!(taken(&log), ["box Started", "box Cancelled"]);

        // The next contact goes down as if none had been.
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["box Started", "box Ended"]);
    }

    /// A box at 0..100 by 0..40 taking drags, disabled or not.
    fn dragged(log: &Log, disabled: bool) -> crate::element::Element {
        let log = log.clone();
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .disabled(disabled)
                    .on_drag(move |e| log.borrow_mut().push(format!("{:?}", e.phase)))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_drag_whose_node_is_disabled_mid_gesture_is_cancelled_once() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let enabled = laid_out(&dragged(&log, false), &ui, &mut text);
        let disabled = laid_out(&dragged(&log, true), &ui, &mut text);

        ui.mouse_down(&enabled, &mut text, 10.0, 10.0);
        ui.pointer_moved(&enabled, &mut text, 20.0, 10.0);
        ui.pointer_moved(&disabled, &mut text, 30.0, 10.0);
        ui.pointer_moved(&disabled, &mut text, 40.0, 10.0);
        ui.mouse_up(&disabled, 40.0, 10.0);
        assert_eq!(taken(&log), ["Start", "Move", "Cancel"], "by the mouse");

        ui.touch(&enabled, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&enabled, &mut text, &finger(TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&disabled, &mut text, &finger(TouchPhase::Moved, 70.0, 10.0));
        ui.touch(&disabled, &mut text, &finger(TouchPhase::Ended, 70.0, 10.0));
        assert_eq!(taken(&log), ["Start", "Move", "Cancel"], "by a finger");
    }

    /// `d` writing `who` and what it gets to `log`: clicks, drag and touch
    /// phases, long presses.
    fn logging(d: crate::element::Div, who: &'static str, log: &Log) -> crate::element::Div {
        let (click, drag, touch, long) = (log.clone(), log.clone(), log.clone(), log.clone());
        d.on_click(move |_| click.borrow_mut().push(format!("{who} click")))
            .on_drag(move |e| drag.borrow_mut().push(format!("{who} {:?}", e.phase)))
            .on_touch(move |e| touch.borrow_mut().push(format!("{who} touch {:?}", e.phase)))
            .on_long_press(move |_| long.borrow_mut().push(format!("{who} long press")))
    }

    /// A keyed box at 0..100 by 0..40 that logs as "target", after `before`
    /// keyed boxes of no size that log as "other" (inserted before it, they
    /// move its index in the frame, not its path); without it when `gone`.
    fn shifting(log: &Log, before: u64, gone: bool) -> crate::element::Element {
        let mut root = crate::element::div().width(400.0).height(100.0);
        for k in 100..100_u64.saturating_add(before) {
            root = root.child(
                logging(crate::element::div().key(k), "other", log)
                    .width(0.0)
                    .height(0.0),
            );
        }
        if !gone {
            root = root.child(
                logging(crate::element::div().key(1), "target", log)
                    .width(100.0)
                    .height(40.0),
            );
        }
        root.into_element()
    }

    /// The frame index of the box [`shifting`] keys 1.
    fn target_at(frame: &Frame) -> usize {
        let key = crate::geometry::PathSeg::Key(1);
        frame
            .nodes
            .iter()
            .position(|n| n.path.0.last() == Some(&key))
            .expect("the target")
    }

    /// Three frames of [`shifting`]: the target first, after a sibling
    /// inserted before it (its old index is the sibling's), and gone.
    fn shifted(log: &Log, ui: &InputState, text: &mut TextSystem) -> (Frame, Frame, Frame) {
        let first = laid_out(&shifting(log, 0, false), ui, text);
        let moved = laid_out(&shifting(log, 1, false), ui, text);
        let gone = laid_out(&shifting(log, 1, true), ui, text);
        let (was, now) = (target_at(&first), target_at(&moved));
        assert_ne!(was, now, "the target moved in the frame");
        assert_eq!(
            moved.nodes[was].path.0.last(),
            Some(&crate::geometry::PathSeg::Key(100)),
            "where it was is another node that takes every gesture"
        );
        (first, moved, gone)
    }

    #[test]
    fn a_drag_follows_its_node_when_a_rebuild_moves_it_in_the_frame() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (first, moved, _) = shifted(&log, &ui, &mut text);

        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.pointer_moved(&first, &mut text, 20.0, 10.0);
        ui.pointer_moved(&moved, &mut text, 30.0, 10.0);
        ui.mouse_up(&moved, 30.0, 10.0);
        assert_eq!(
            taken(&log),
            [
                "target Start",
                "target Move",
                "target Move",
                "target End",
                "target click"
            ],
            "by the mouse"
        );

        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&first, &mut text, &finger(TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&moved, &mut text, &finger(TouchPhase::Moved, 70.0, 10.0));
        ui.touch(&moved, &mut text, &finger(TouchPhase::Ended, 70.0, 10.0));
        assert_eq!(
            taken(&log),
            [
                "target touch Started",
                "target touch Moved",
                "target Start",
                "target Move",
                "target touch Moved",
                "target Move",
                "target End",
                "target touch Ended",
            ],
            "by a finger"
        );
    }

    #[test]
    fn a_drag_whose_node_goes_mid_gesture_is_cancelled_once_through_the_handler_it_had() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (first, _, gone) = shifted(&log, &ui, &mut text);

        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.pointer_moved(&first, &mut text, 20.0, 10.0);
        ui.pointer_moved(&gone, &mut text, 30.0, 10.0);
        ui.pointer_moved(&gone, &mut text, 40.0, 10.0);
        ui.mouse_up(&gone, 40.0, 10.0);
        assert_eq!(
            taken(&log),
            ["target Start", "target Move", "target Cancel"],
            "by the mouse"
        );

        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&first, &mut text, &finger(TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&gone, &mut text, &finger(TouchPhase::Moved, 70.0, 10.0));
        ui.touch(&gone, &mut text, &finger(TouchPhase::Moved, 80.0, 10.0));
        ui.touch(&gone, &mut text, &finger(TouchPhase::Ended, 80.0, 10.0));
        assert_eq!(
            taken(&log),
            [
                "target touch Started",
                "target touch Moved",
                "target Start",
                "target Move",
                "target Cancel",
                "target touch Cancelled",
            ],
            "by a finger"
        );

        // The next gesture starts as if none had been.
        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.mouse_up(&first, 10.0, 10.0);
        assert_eq!(taken(&log), ["target Start", "target End", "target click"]);
    }

    #[test]
    fn a_press_clicks_and_holds_the_node_it_went_down_on_when_a_rebuild_moves_it() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (first, moved, gone) = shifted(&log, &ui, &mut text);

        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.mouse_up(&moved, 10.0, 10.0);
        assert_eq!(taken(&log), ["target Start", "target End", "target click"], "a click");

        // A tap: the touch went down on the target, which moved in the
        // frame before it lifted.
        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&moved, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(
            taken(&log),
            [
                "target touch Started",
                "target Start",
                "target End",
                "target click",
                "target touch Ended"
            ],
            "a tap"
        );

        // A long press runs on the node the finger went down on.
        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        let later = crate::clock::current_seconds() + crate::gesture::LONG_PRESS.as_secs_f32() + 0.1;
        crate::clock::advance_to(std::time::Duration::from_secs_f32(later));
        ui.long_presses(&moved);
        ui.touch(&moved, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(
            taken(&log),
            ["target touch Started", "target long press", "target touch Ended"],
            "a long press"
        );

        // Pressed, then gone: the release over where it was clicks nothing.
        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.mouse_up(&gone, 10.0, 10.0);
        assert_eq!(taken(&log), ["target Start", "target Cancel"], "gone");
    }

    #[test]
    fn a_press_on_an_unavailable_box_focuses_what_it_hands_focus_to() {
        let (first, second) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let ran = std::rc::Rc::new(std::cell::Cell::new(false));
        let own = ran.clone();
        let root = crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .unavailable(true)
                    .press_focuses(move || (!second.is_focused()).then_some(first))
                    .on_mouse_down(move |_| own.set(true))
                    .display(crate::style::Display::Flex)
                    .width(300.0)
                    .height(40.0)
                    .child(crate::element::div().track_focus(first).width(50.0).height(40.0))
                    .child(crate::element::div().track_focus(second).width(50.0).height(40.0)),
            )
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        focus::set_focus(None, false);
        ui.mouse_down(&frame, &mut text, 200.0, 20.0);
        ui.mouse_up(&frame, 200.0, 20.0);
        assert_eq!(focus::focused(), Some(first.id()), "beside its parts, the first");
        assert!(!ran.get(), "its own press handler does not run");
        ui.mouse_down(&frame, &mut text, 75.0, 20.0);
        ui.mouse_up(&frame, 75.0, 20.0);
        assert_eq!(focus::focused(), Some(second.id()), "on a part, that part");
        ui.mouse_down(&frame, &mut text, 200.0, 20.0);
        assert_eq!(focus::focused(), Some(second.id()), "handed none, focus stays");
    }

    /// `d` writing `who` and what it gets to `log`: a mouse down, clicks,
    /// drag and touch phases, long presses.
    fn tapped(d: crate::element::Div, who: &'static str, log: &Log) -> crate::element::Div {
        let down = log.clone();
        logging(d, who, log).on_mouse_down(move |_| down.borrow_mut().push(format!("{who} mouse down")))
    }

    /// Boxes of 100 by 40 stacked from the top, keyed and focusable, each
    /// [`tapped`] as its name.
    fn column(log: &Log, boxes: &[(u64, &'static str, crate::focus::FocusHandle)]) -> crate::element::Element {
        let mut root = crate::element::div().width(400.0).height(100.0);
        for &(k, who, f) in boxes {
            root = root.child(
                tapped(crate::element::div().key(k), who, log)
                    .track_focus(f)
                    .width(100.0)
                    .height(40.0),
            );
        }
        root.into_element()
    }

    #[test]
    fn a_tap_goes_whole_to_the_node_under_it_as_it_lifts_when_it_went_down_there() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let first = laid_out(&column(&log, &[(1, "A", a)]), &ui, &mut text);
        // A replaced by B before the finger lifts.
        let replaced = laid_out(&column(&log, &[(2, "B", b)]), &ui, &mut text);
        // B came to the top and A moved below it.
        let swapped = laid_out(&column(&log, &[(2, "B", b), (1, "A", a)]), &ui, &mut text);
        let path_a = first.nodes[hit(&first, 10.0, 10.0).expect("A")].path.clone();

        // Replaced: what the finger went down on has gone, so its lift is
        // no tap, and B, made in its place, takes nothing of it. A gets
        // only the cancel of the raw touch it captured as the finger went
        // down, once it has gone.
        focus::set_focus(None, false);
        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        assert!(
            ui.state().active.contains(&path_a),
            "the touch presses A as it goes down"
        );
        ui.touch(&replaced, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Cancelled"], "replaced");
        assert_eq!(focus::focused(), None, "no tap focuses B");
        assert!(ui.state().active.is_empty(), "nothing is left pressed");

        // Swapped: B came over A, which is still there below it. B is
        // neither A nor above it, so it takes nothing of the tap; A keeps
        // its captured raw touch phases (Chrome's `touchend` goes where
        // `touchstart` went), and its pressed look ends.
        focus::set_focus(None, false);
        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&swapped, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Ended"], "swapped");
        assert_eq!(focus::focused(), None, "B is not focused");
        assert!(ui.state().active.is_empty(), "A's pressed look ended");

        // The mouse keeps its rule: a click only when released over the
        // node pressed.
        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.mouse_up(&replaced, 10.0, 10.0);
        assert_eq!(
            taken(&log),
            ["A mouse down", "A Start", "A Cancel"],
            "replaced, by the mouse"
        );
        ui.mouse_down(&first, &mut text, 10.0, 10.0);
        ui.mouse_up(&swapped, 10.0, 10.0);
        assert_eq!(
            taken(&log),
            ["A mouse down", "A Start", "A End"],
            "swapped, by the mouse"
        );
    }

    /// A keyed slider of 100 by 40 at the top logging its drag phases and
    /// where they are, as `who`, after `before` keyed boxes of no size.
    fn slider_at(log: &Log, key: u64, who: &'static str, before: u64) -> crate::element::Element {
        let mut root = crate::element::div().width(400.0).height(100.0);
        for k in 100..100_u64.saturating_add(before) {
            root = root.child(crate::element::div().key(k).width(0.0).height(0.0));
        }
        let log = log.clone();
        root.child(
            crate::element::div()
                .key(key)
                .role(accesskit::Role::Slider)
                .on_drag(move |e| {
                    log.borrow_mut()
                        .push(format!("{who} {:?} {:.0},{:.0}", e.phase, e.x, e.y));
                })
                .width(100.0)
                .height(40.0),
        )
        .into_element()
    }

    #[test]
    fn a_slider_under_a_tap_as_it_lifts_gets_one_start_and_end_there() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let old = laid_out(&slider_at(&log, 1, "old", 0), &ui, &mut text);
        let new = laid_out(&slider_at(&log, 2, "new", 0), &ui, &mut text);
        let moved = laid_out(&slider_at(&log, 1, "old", 1), &ui, &mut text);

        // Replaced: what the finger went down on has gone, so its lift is
        // no tap, and the new slider takes nothing of it.
        ui.touch(&old, &mut text, &finger(TouchPhase::Started, 30.0, 10.0));
        ui.touch(&new, &mut text, &finger(TouchPhase::Ended, 32.0, 11.0));
        assert!(taken(&log).is_empty(), "no tap");

        // The same slider, moved in the frame: it takes it.
        ui.touch(&old, &mut text, &finger(TouchPhase::Started, 30.0, 10.0));
        ui.touch(&moved, &mut text, &finger(TouchPhase::Ended, 30.0, 10.0));
        assert_eq!(taken(&log), ["old Start 30,10", "old End 30,10"]);
    }

    #[test]
    fn a_frame_that_places_a_held_drag_again_is_no_move() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let first = laid_out(&slider_at(&log, 1, "s", 0), &ui, &mut text);
        // The same slider in the same place, its frame laid out anew.
        let again = laid_out(&slider_at(&log, 1, "s", 1), &ui, &mut text);

        ui.mouse_down(&first, &mut text, 30.0, 10.0);
        ui.rehover(&first, &mut text);
        ui.pointer_moved(&first, &mut text, 40.0, 10.0);
        ui.rehover(&again, &mut text);
        ui.rehover(&again, &mut text);
        // A move the pointer made is one, even to where it was.
        ui.pointer_moved(&again, &mut text, 40.0, 10.0);
        ui.mouse_up(&again, 40.0, 10.0);
        assert_eq!(
            taken(&log),
            ["s Start 30,10", "s Move 40,10", "s Move 40,10", "s End 40,10"]
        );
    }

    /// A keyed slider of 100 by 40 at the top of a keyed container 100 tall
    /// that scrolls (200 more below the slider), logging its drag phases
    /// and where they are in its own box.
    fn scrolled_slider(log: &Log) -> crate::element::Element {
        let log = log.clone();
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(9)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .child(
                        crate::element::div()
                            .key(1)
                            .role(accesskit::Role::Slider)
                            .on_drag(move |e| {
                                log.borrow_mut().push(format!(
                                    "{:?} {:.0},{:.0}",
                                    e.phase,
                                    e.x - e.bounds.x,
                                    e.y - e.bounds.y
                                ));
                            })
                            .width(100.0)
                            .height(40.0),
                    )
                    .child(crate::element::div().width(100.0).height(200.0)),
            )
            .into_element()
    }

    #[test]
    fn a_held_slider_scrolled_under_a_resting_pointer_moves_once_to_where_it_is_in_it() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let first = laid_out(&scrolled_slider(&log), &ui, &mut text);
        let container = first
            .nodes
            .iter()
            .find(|n| n.path.0.last() == Some(&crate::geometry::PathSeg::Key(9)))
            .expect("the container")
            .path
            .clone();

        ui.mouse_down(&first, &mut text, 30.0, 10.0);
        ui.rehover(&first, &mut text);
        assert_eq!(taken(&log), ["Start 30,10"]);

        // The wheel scrolls the container under the pointer: the frame that
        // places the slider 5 higher moves it once, 5 further down in it.
        assert!(ui.wheel(&first, 0.0, 5.0).redraw, "the wheel scrolls");
        ui.rehover(&first, &mut text);
        let scrolled = laid_out(&scrolled_slider(&log), &ui, &mut text);
        ui.rehover(&scrolled, &mut text);
        ui.rehover(&scrolled, &mut text);
        // Laid out anew, not moved: no move.
        let again = laid_out(&scrolled_slider(&log), &ui, &mut text);
        ui.rehover(&again, &mut text);
        assert_eq!(taken(&log), ["Move 30,15"], "by the wheel");

        // Scrolled by the program: the same.
        ui.state.scroll.insert((container, None), motion::units2((0.0, 12.0)));
        let jumped = laid_out(&scrolled_slider(&log), &ui, &mut text);
        ui.rehover(&jumped, &mut text);
        ui.rehover(&jumped, &mut text);
        ui.mouse_up(&jumped, 30.0, 10.0);
        assert_eq!(taken(&log), ["Move 30,22", "End 30,22"], "by the program");
    }

    /// The frame index of the node keyed `k`.
    fn keyed(frame: &Frame, k: u64) -> usize {
        let key = crate::geometry::PathSeg::Key(k);
        frame
            .nodes
            .iter()
            .position(|n| n.path.0.last() == Some(&key))
            .expect("the keyed node")
    }

    /// A container scrolling down (key 9) holding a box that takes drags
    /// (key 2, logging where they are in its own box), which holds a
    /// container of its own scrolling down (key 3) with a plain box at its
    /// top (key 4): what the compositor scrolls moves the plain box by
    /// both scrolls and the dragged box by the outer one alone.
    fn nested_scrolls(log: &Log) -> crate::element::Element {
        let log = log.clone();
        let inner = crate::element::div()
            .key(3)
            .width(100.0)
            .height(60.0)
            .overflow_y(crate::style::Overflow::Scroll)
            .child(crate::element::div().key(4).width(100.0).height(40.0))
            .child(crate::element::div().width(100.0).height(200.0));
        let dragged = crate::element::div()
            .key(2)
            .on_drag(move |e| {
                log.borrow_mut()
                    .push(format!("{:?} {:.0},{:.0}", e.phase, e.x - e.bounds.x, e.y - e.bounds.y));
            })
            .width(100.0)
            .height(300.0)
            .child(inner);
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(9)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .child(dragged),
            )
            .into_element()
    }

    /// What the compositor showed of `frame`'s scroll containers.
    fn presented(frame: &Frame, text: &mut TextSystem) -> std::sync::Arc<crate::compositor::Presented> {
        let scroll = frame
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let s = n.scroll(OUTSIDE)?;
                Some(crate::compositor::ShownScroll {
                    key: crate::compositor::NodeKey::of(&n.path),
                    owner: owner_of(frame, i),
                    offset: s.offset,
                    applied: s.applied,
                })
            })
            .collect();
        showing(
            frame,
            text,
            crate::compositor::Presented {
                scroll,
                ..crate::compositor::Presented::default()
            },
        )
    }

    /// `shown`, with what `frame` (the frame as the compositor showed it)
    /// gives input to hit: its boxes and property trees, painted.
    fn showing(
        frame: &Frame,
        text: &mut TextSystem,
        shown: crate::compositor::Presented,
    ) -> std::sync::Arc<crate::compositor::Presented> {
        let size = (400, 400);
        let scene = crate::paint::Painter::fresh(frame, text, size, 1.0);
        std::sync::Arc::new(crate::compositor::Presented {
            hits: std::sync::Arc::clone(&scene.compositing.hits),
            texts: std::sync::Arc::clone(&scene.compositing.texts),
            properties: scene.properties.clone(),
            scale: scene.scale,
            ..shown
        })
    }

    #[test]
    fn a_press_on_a_node_the_compositor_scrolled_is_no_move_until_the_pointer_moves() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // What the compositor scrolled to and shows: the outer container 10
        // down, the inner one 5 more; the UI has not taken it in yet.
        let laid = laid_out(&nested_scrolls(&log), &ui, &mut text);
        let (outer, inner) = (
            laid.nodes[keyed(&laid, 9)].path.clone(),
            laid.nodes[keyed(&laid, 3)].path.clone(),
        );
        ui.state
            .scroll
            .insert((outer.clone(), None), motion::units2((0.0, 10.0)));
        ui.state
            .scroll
            .insert((inner.clone(), None), motion::units2((0.0, 5.0)));
        let scrolled = laid_out(&nested_scrolls(&log), &ui, &mut text);
        let shown = presented(&scrolled, &mut text);
        ui.state.scroll.clear();
        let frame = laid_out(&nested_scrolls(&log), &ui, &mut text);

        // As the window does: the screen showed `shown` when the press came.
        ui.show(Some(shown.clone()));
        ui.take_in(&frame);
        let (hit, owner) = (keyed(&frame, 4), keyed(&frame, 2));
        assert_ne!(
            ui.shown(&frame).to_node(hit, (30.0, 20.0)),
            ui.shown(&frame).to_node(owner, (30.0, 20.0)),
            "the node hit and the drag's owner show shifted apart"
        );
        assert_eq!(
            ui.locate(&frame, 30.0, 20.0).0,
            Some(hit),
            "the press lands on the plain box"
        );
        ui.mouse_down(&frame, &mut text, 30.0, 20.0);
        // The frames after, as the window draws them: each compares with
        // what the screen shows then, here still `shown`.
        ui.show(Some(shown.clone()));
        ui.rehover(&frame, &mut text);
        // A layout of nothing that moves it, before the UI takes the scroll in.
        let again = laid_out(&nested_scrolls(&log), &ui, &mut text);
        ui.show(Some(shown.clone()));
        ui.rehover(&again, &mut text);
        assert_eq!(taken(&log), ["Start 30,30"], "no move until the pointer moves");
        ui.show(Some(shown.clone()));
        ui.pointer_moved(&again, &mut text, 40.0, 20.0);
        assert_eq!(taken(&log), ["Move 40,30"]);

        // The UI takes the scroll in: laid out where the screen showed it,
        // nothing moved under the pointer.
        ui.state.scroll.insert((outer, None), motion::units2((0.0, 10.0)));
        ui.state.scroll.insert((inner, None), motion::units2((0.0, 5.0)));
        let caught_up = laid_out(&nested_scrolls(&log), &ui, &mut text);
        ui.show(Some(shown.clone()));
        ui.rehover(&caught_up, &mut text);
        ui.show(Some(shown));
        ui.mouse_up(&caught_up, 40.0, 20.0);
        assert_eq!(taken(&log), ["End 40,30"], "taken in, it is where it showed");
    }

    /// A container the compositor scrolled, of 20,000 rows that each take
    /// the pointer: a hover, a click and a rehover under a frame the
    /// compositor scrolled further (the same hit table) each look at a few
    /// frame nodes and compose a few transform chains, not one per row or
    /// node. The per-frame facts a hit reads (a node's scroll shift, a
    /// turn above it) are found up its ancestor chain, and the hit table's
    /// boxes are found through a grid made once per table (RC5).
    #[test]
    fn a_hit_among_twenty_thousand_rows_visits_a_few_nodes_and_composes_a_few_chains() {
        const ROWS: u64 = 20_000;
        let clicked = std::rc::Rc::new(std::cell::Cell::new(None::<u64>));
        let rows = |clicked: &std::rc::Rc<std::cell::Cell<Option<u64>>>| {
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    crate::element::div()
                        .key(9)
                        .width(400.0)
                        .height(100.0)
                        .overflow_y(crate::style::Overflow::Scroll)
                        .children((0..ROWS).map(|k| {
                            let clicked = std::rc::Rc::clone(clicked);
                            crate::element::div()
                                .key(100 + k)
                                .width(400.0)
                                .height(1.0)
                                .on_click(move |_| clicked.set(Some(k)))
                        })),
                )
                .into_element()
        };
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let at = |frame: &Frame| {
            let list = keyed(frame, 9);
            (frame.nodes[list].path.clone(), owner_of(frame, list))
        };
        // What the compositor scrolled to and shows: the list 10 down.
        let frame = laid_out(&rows(&clicked), &ui, &mut text);
        let (list, owner) = at(&frame);
        ui.state.scroll.insert((list, owner), motion::units2((0.0, 10.0)));
        let scrolled = laid_out(&rows(&clicked), &ui, &mut text);
        let shown = presented(&scrolled, &mut text);
        assert!(
            shown.hits.len() >= 20_000,
            "every row is a hit box: {}",
            shown.hits.len()
        );
        ui.state.scroll.clear();
        let frame = laid_out(&rows(&clicked), &ui, &mut text);
        ui.show(Some(shown.clone()));
        // The hit table's grid, made once for the table at its first hit.
        ui.pointer_moved(&frame, &mut text, 200.0, 5.5);

        let cost = || (index_visits(), crate::clip::composed());
        let since = |(v, c): (u64, u64)| (index_visits() - v, crate::clip::composed() - c);
        let (visits, composed) = {
            let before = cost();
            let candidates = crate::clip::candidates();
            ui.show(Some(shown.clone()));
            ui.pointer_moved(&frame, &mut text, 200.0, 50.5);
            // Every box and index node the hit weighed, not only the frame
            // nodes it read: the logarithm of the 20,000 rows, not a walk.
            let weighed = crate::clip::candidates() - candidates;
            assert!(weighed <= 128, "a hover weighed {weighed} candidates");
            since(before)
        };
        assert!(
            visits <= 16 && composed <= 8,
            "a hover: {visits} nodes, {composed} chains"
        );

        let (visits, composed) = {
            let before = cost();
            ui.show(Some(shown.clone()));
            ui.mouse_down(&frame, &mut text, 200.0, 50.5);
            ui.show(Some(shown.clone()));
            ui.mouse_up(&frame, 200.0, 50.5);
            since(before)
        };
        assert_eq!(clicked.get(), Some(60), "the row the screen shows under the pointer");
        assert!(
            visits <= 32 && composed <= 16,
            "a click: {visits} nodes, {composed} chains"
        );

        // The compositor scrolls on, 5 more: the same hit table, another
        // frame on screen; the pointer rests.
        let further = std::sync::Arc::new(crate::compositor::Presented {
            scroll: shown
                .scroll
                .iter()
                .map(|s| crate::compositor::ShownScroll {
                    offset: (s.offset.0, s.offset.1 + 5.0),
                    applied: (s.applied.0, s.applied.1 + 5.0),
                    ..*s
                })
                .collect(),
            frame: shown.frame + 1,
            ..(*shown).clone()
        });
        let (visits, composed) = {
            let before = cost();
            ui.show(Some(further));
            ui.rehover(&frame, &mut text);
            since(before)
        };
        assert!(
            visits <= 16 && composed <= 8,
            "a rehover: {visits} nodes, {composed} chains"
        );
    }

    /// A box at 0..100 by 0..40 taking drags, logging where they are in its
    /// own box, scaled `scale` about its top-left corner.
    fn scaled(log: &Log, scale: f32) -> crate::element::Element {
        let log = log.clone();
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(1)
                    .on_drag(move |e| {
                        log.borrow_mut()
                            .push(format!("{:?} {:.0},{:.0}", e.phase, e.x - e.bounds.x, e.y - e.bounds.y));
                    })
                    .scale(scale)
                    .origin(crate::style::Origin::At([
                        crate::style::Calc::fraction(0.0),
                        crate::style::Calc::fraction(0.0),
                    ]))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_held_box_scaled_under_a_resting_pointer_moves_once_to_where_it_is_in_it() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let plain = laid_out(&scaled(&log, 1.0), &ui, &mut text);
        let doubled = laid_out(&scaled(&log, 2.0), &ui, &mut text);

        ui.mouse_down(&plain, &mut text, 30.0, 10.0);
        ui.rehover(&plain, &mut text);
        // Scaled 2x about its corner: the point under the pointer is half
        // as far into it. One move, then nothing for a frame that places
        // it again.
        ui.rehover(&doubled, &mut text);
        ui.rehover(&doubled, &mut text);
        ui.pointer_moved(&doubled, &mut text, 50.0, 30.0);
        ui.mouse_up(&doubled, 50.0, 30.0);
        assert_eq!(taken(&log), ["Start 30,10", "Move 15,5", "Move 25,15", "End 25,15"]);
    }

    /// A paragraph of two spans, "aaaa" and "bbbb", each logging its
    /// clicks, scaled `scale` about its top-left corner.
    fn two_spans(log: &Log, scale: f32) -> crate::element::Element {
        let (a, b) = (log.clone(), log.clone());
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::text("")
                    .key(1)
                    .span(crate::element::span("aaaa").on_click(move |_| a.borrow_mut().push("a".to_owned())))
                    .span(crate::element::span("bbbb").on_click(move |_| b.borrow_mut().push("b".to_owned())))
                    .scale(scale)
                    .origin(crate::style::Origin::At([
                        crate::style::Calc::fraction(0.0),
                        crate::style::Calc::fraction(0.0),
                    ])),
            )
            .into_element()
    }

    /// A click routed in frame A, where a paragraph shows unscaled, lands
    /// on the span A showed under it, though the frame laid out since (B)
    /// scales the paragraph twice: the point is mapped into the node by
    /// A's property trees, the frame it was hit in, never by B's
    /// transform (which would put it half as far in, on the other span).
    #[test]
    fn a_click_routed_in_a_frame_maps_into_the_span_that_frame_showed() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let plain = laid_out(&two_spans(&log, 1.0), &ui, &mut text);
        let doubled = laid_out(&two_spans(&log, 2.0), &ui, &mut text);
        let p = keyed(&plain, 1);
        let spans = &plain.behaviour(p).expect("the paragraph's behaviour").spans;
        let b = spans[1].boxes[0];
        let (x, y) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
        assert!(
            spans[0].boxes[0].contains(x / 2.0, y / 2.0),
            "B's scale would put the point on the first span"
        );
        // The screen showed A as the press came; the UI laid out B since.
        ui.show(Some(presented(&plain, &mut text)));
        ui.take_in(&doubled);
        ui.mouse_down(&doubled, &mut text, x, y);
        ui.mouse_up(&doubled, x, y);
        assert_eq!(taken(&log), ["b"], "the span A showed under the click");
    }

    /// A keyed button of 100 by 40 at the top, scaled by `scale` about its
    /// left edge, logging its drag phases (where they are in it, and which
    /// pointer) and its clicks.
    fn scaled_button(log: &Log, scale: f32) -> crate::element::Element {
        let (dragged, clicked) = (log.clone(), log.clone());
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(1)
                    .on_drag(move |e| {
                        dragged.borrow_mut().push(format!(
                            "{:?} {:.0},{:.0}",
                            e.phase,
                            e.x - e.bounds.x,
                            e.y - e.bounds.y
                        ));
                    })
                    .on_click(move |_| clicked.borrow_mut().push("click".to_owned()))
                    .scale(scale)
                    .origin(crate::style::Origin::At([
                        crate::style::Calc::fraction(0.0),
                        crate::style::Calc::fraction(0.0),
                    ]))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_press_lands_where_the_scale_shows_the_node_and_a_scale_of_0_takes_none() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let doubled = laid_out(&scaled_button(&log, 2.0), &ui, &mut text);
        let folded = laid_out(&scaled_button(&log, 0.0), &ui, &mut text);

        // Scaled 2x about its left edge it shows 200 wide: a press 150 in,
        // past the box as laid out, is on it, 75 into it, as its events say.
        ui.mouse_down(&doubled, &mut text, 150.0, 10.0);
        ui.mouse_up(&doubled, 150.0, 10.0);
        assert_eq!(taken(&log), ["Start 75,5", "End 75,5", "click"]);

        // Scaled to nothing, where it was laid out takes no press.
        ui.mouse_down(&folded, &mut text, 30.0, 10.0);
        ui.mouse_up(&folded, 30.0, 10.0);
        assert!(taken(&log).is_empty(), "a node scaled to 0 is hit by nothing");
        let free = hit(&folded, 30.0, 10.0)
            .and_then(|i| folded.nodes.get(i))
            .map(|n| n.path.clone());
        assert_ne!(free, Some(path_at(&doubled, 10.0, 10.0)), "nor by the free hit test");
    }

    /// Two keyed boxes, one above the other, 200 by 50 each, logging their drag
    /// phases and raw touches with the pointer: taking several pointers
    /// when `multi`, captured together when `group` is given.
    fn captures(log: &Log, multi: bool, group: Option<crate::events::CaptureGroup>) -> crate::element::Element {
        let boxed = |k: u64, who: &'static str| {
            let (dragged, touched) = (log.clone(), log.clone());
            let d = crate::element::div()
                .key(k)
                .on_drag(move |e| {
                    dragged
                        .borrow_mut()
                        .push(format!("{who} {:?} {:?}", e.pointer, e.phase));
                })
                .on_touch(move |e| touched.borrow_mut().push(format!("{who} touch {} {:?}", e.id, e.phase)))
                .width(200.0)
                .height(50.0);
            let d = if multi { d.multi_pointer() } else { d };
            match group {
                Some(g) => d.capture_group(g),
                None => d,
            }
        };
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(boxed(1, "A"))
            .child(boxed(2, "B"))
            .into_element()
    }

    #[test]
    fn a_wheel_at_a_point_goes_to_what_is_there_wherever_the_mouse_is() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let wheeled = |who: &'static str| {
            let log = log.clone();
            crate::element::div()
                .on_wheel(move |e| {
                    log.borrow_mut().push(format!("{who} {:.0}", e.dy));
                    true
                })
                .width(200.0)
                .height(50.0)
        };
        let root = crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(wheeled("A"))
            .child(wheeled("B"))
            .into_element();
        let frame = laid_out(&root, &ui, &mut text);
        // The mouse rests on A; the platform's wheel came at B.
        ui.pointer_moved(&frame, &mut text, 10.0, 10.0);
        ui.wheel_at(&frame, notched((10.0, 60.0)), (0.0, 40.0), None);
        ui.wheel(&frame, 0.0, 5.0);
        assert_eq!(taken(&log), ["B 40", "A 5"]);
    }

    #[test]
    fn a_raw_touch_and_a_drag_of_a_node_are_one_capture_whoever_takes_it_first() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let lone = laid_out(&captures(&log, false, None), &ui, &mut text);
        // A finger rests on A (its raw touch, no drag yet, not moved): the
        // mouse pressing A drags nothing.
        ui.touch(&lone, &mut text, &contact(1, TouchPhase::Started, 10.0, 10.0));
        ui.mouse_down(&lone, &mut text, 20.0, 10.0);
        ui.mouse_up(&lone, 20.0, 10.0);
        ui.touch(&lone, &mut text, &contact(1, TouchPhase::Cancelled, 10.0, 10.0));
        assert_eq!(taken(&log), ["A touch 1 Started", "A touch 1 Cancelled"]);
        // The mouse drags A: a finger pressing it gets no raw touch.
        ui.mouse_down(&lone, &mut text, 20.0, 10.0);
        ui.touch(&lone, &mut text, &contact(2, TouchPhase::Started, 10.0, 10.0));
        ui.touch(&lone, &mut text, &contact(2, TouchPhase::Cancelled, 10.0, 10.0));
        ui.mouse_up(&lone, 20.0, 10.0);
        assert_eq!(taken(&log), ["A Mouse Start", "A Mouse End"]);
        // Across a capture group: a finger on A, the mouse on B.
        let grouped = laid_out(
            &captures(&log, false, Some(crate::events::CaptureGroup::new())),
            &ui,
            &mut text,
        );
        ui.touch(&grouped, &mut text, &contact(3, TouchPhase::Started, 10.0, 10.0));
        ui.mouse_down(&grouped, &mut text, 20.0, 60.0);
        ui.mouse_up(&grouped, 20.0, 60.0);
        ui.touch(&grouped, &mut text, &contact(3, TouchPhase::Cancelled, 10.0, 10.0));
        assert_eq!(taken(&log), ["A touch 3 Started", "A touch 3 Cancelled"]);
    }

    #[test]
    fn a_late_wheel_skips_a_node_disabled_or_hidden_since_the_frame_it_came_under() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // A focused number field's input steps on the wheel.
        let field = |disabled: bool, hidden: bool| {
            let log = log.clone();
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    crate::element::div()
                        .key(1)
                        .disabled(disabled)
                        .invisible(hidden)
                        .on_wheel(move |_| {
                            log.borrow_mut().push("step".to_owned());
                            true
                        })
                        .width(200.0)
                        .height(50.0),
                )
                .into_element()
        };
        let shown = laid_out(&field(false, false), &ui, &mut text);
        let target = crate::compositor::HitTarget {
            key: crate::compositor::NodeKey::of(&path_at(&shown, 10.0, 10.0)),
            owner: None,
        };
        ui.wheel_at(&shown, notched((10.0, 10.0)), (0.0, 40.0), Some(target));
        assert_eq!(taken(&log), ["step"], "enabled, it steps");
        for (disabled, hidden) in [(true, false), (false, true)] {
            let now = laid_out(&field(disabled, hidden), &ui, &mut text);
            ui.wheel_at(&now, notched((10.0, 10.0)), (0.0, 40.0), Some(target));
            assert!(taken(&log).is_empty(), "disabled {disabled}, hidden {hidden}: no step");
        }
    }

    /// A wheel the render thread found over a control, dispatched once a
    /// modal layer opened over it: the control below the modal takes
    /// nothing, as it takes no other input.
    #[test]
    fn a_late_wheel_skips_a_node_a_modal_layer_opened_over_since() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let page = |modal: bool| {
            let log = log.clone();
            let root = crate::element::div().width(400.0).height(100.0).child(
                crate::element::div()
                    .key(1)
                    .on_wheel(move |_| {
                        log.borrow_mut().push("step".to_owned());
                        true
                    })
                    .width(200.0)
                    .height(50.0),
            );
            if modal {
                root.child(crate::element::deferred(
                    crate::element::Positioning::At { x: 300.0, y: 0.0 },
                    crate::element::div()
                        .layer(crate::layer::Layer::new().modal(true))
                        .width(100.0)
                        .height(100.0),
                ))
            } else {
                root
            }
            .into_element()
        };
        let shown = laid_out(&page(false), &ui, &mut text);
        let target = crate::compositor::HitTarget {
            key: crate::compositor::NodeKey::of(&path_at(&shown, 10.0, 10.0)),
            owner: None,
        };
        let now = laid_out(&page(true), &ui, &mut text);
        ui.wheel_at(&now, notched((10.0, 10.0)), (0.0, 40.0), Some(target));
        assert!(taken(&log).is_empty(), "below the modal: no step");
    }

    /// A trackpad's gesture latched to a list, going on over a modal layer
    /// opened since: the list below the modal no longer takes it.
    #[test]
    fn a_gesture_latched_to_a_list_a_modal_opened_over_scrolls_it_no_more() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let page = |modal: bool| {
            let root = crate::element::div().width(400.0).height(100.0).child(
                crate::element::div()
                    .key(2)
                    .width(100.0)
                    .height(60.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .child(crate::element::div().width(100.0).height(200.0)),
            );
            if modal {
                root.child(crate::element::deferred(
                    crate::element::Positioning::At { x: 200.0, y: 0.0 },
                    crate::element::div()
                        .layer(crate::layer::Layer::new().modal(true))
                        .width(100.0)
                        .height(100.0),
                ))
            } else {
                root
            }
            .into_element()
        };
        let laid = laid_out(&page(false), &ui, &mut text);
        let list = laid.nodes[keyed(&laid, 2)].path.clone();
        let offset = |ui: &InputState| {
            ui.state
                .scroll
                .get(&(list.clone(), None))
                .map_or(0.0, |o| motion::px(o.1))
        };
        let pad = |point: (f32, f32), began: bool| crate::compositor::WheelStep {
            point,
            time: crate::clock::current_nanos(),
            precise: true,
            phase: if began {
                crate::compositor::WheelPhase::Began
            } else {
                crate::compositor::WheelPhase::Changed
            },
            gesture: None,
        };
        ui.wheel_at(&laid, pad((50.0, 20.0), true), (0.0, 30.0), None);
        assert_eq!(offset(&ui), 30.0, "the list took the gesture");
        let modal = laid_out(&page(true), &ui, &mut text);
        ui.wheel_at(&modal, pad((250.0, 50.0), false), (0.0, 30.0), None);
        assert_eq!(offset(&ui), 30.0, "below the modal: no further");
    }

    /// The compositor's scroll of list A arriving after B replaced it at
    /// the same path (another instance) is A's: B takes none of it; a
    /// delta naming B is B's.
    #[test]
    fn a_scroll_delta_for_a_list_another_instance_replaced_moves_nothing() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let list = |who: &'static str| {
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    tapped(crate::element::div().key(2), who, &log)
                        .width(100.0)
                        .height(60.0)
                        .overflow_y(crate::style::Overflow::Scroll)
                        .child(crate::element::div().width(100.0).height(200.0)),
                )
                .into_element()
        };
        let (a, b) = (crate::reactive::Scope::new(), crate::reactive::Scope::new());
        let first = laid_out(&a.run(|| list("A")), &ui, &mut text);
        let i = keyed(&first, 2);
        let (key, owner_a) = (
            crate::compositor::NodeKey::of(&first.nodes[i].path),
            owner_of(&first, i),
        );
        let replaced = laid_out(&b.run(|| list("B")), &ui, &mut text);
        let j = keyed(&replaced, 2);
        let path = replaced.nodes[j].path.clone();
        let owner_b = owner_of(&replaced, j);
        assert!(owner_a.is_some() && owner_a != owner_b, "two instances at one path");
        let delta = |owner| crate::compositor::ScrollDelta {
            key,
            owner,
            delta: motion::units2((0.0, 30.0)),
            reach: (0.0, 0.0),
            steps: 1,
            ..Default::default()
        };
        ui.take_scroll(&replaced, &[delta(owner_a)]);
        assert_eq!(
            ui.state.scroll.get(&(path.clone(), owner_b)),
            None,
            "B took none of A's"
        );
        ui.take_scroll(&replaced, &[delta(owner_b)]);
        assert_eq!(
            ui.state.scroll.get(&(path, owner_b)),
            Some(&motion::units2((0.0, 30.0))),
            "B's is B's"
        );
    }

    /// The last frame showed a list at 0 and the compositor scrolled it 30
    /// on from there; the program scrolls it to 200 before the UI takes
    /// that in (ADR-0032: the UI took in before the input that jumped
    /// whatever wheels came before it, so this is of wheels after the
    /// jump). The jump is an assignment, the take a displacement on it:
    /// the list is at 230, from the jump, not 30 nor 200.
    #[test]
    fn the_compositors_scroll_taken_after_a_jump_moves_from_it() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let list = |h: &ScrollHandle| {
            crate::element::div()
                .child(
                    crate::element::div()
                        .key(2)
                        .width(100.0)
                        .height(60.0)
                        .overflow_y(crate::style::Overflow::Scroll)
                        .track_scroll(h)
                        .child(crate::element::div().width(100.0).height(400.0)),
                )
                .into_element()
        };
        // The compositor's scroll of the list `handle` shows in `frame`.
        let scrolled = |frame: &Frame| {
            let i = keyed(frame, 2);
            let s = frame.nodes[i].scroll(OUTSIDE).expect("a scroll container");
            assert_eq!(s.offset, (0.0, 0.0), "shown at 0");
            crate::compositor::ScrollDelta {
                key: crate::compositor::NodeKey::of(&frame.nodes[i].path),
                owner: owner_of(frame, i),
                delta: motion::units2((0.0, 30.0)),
                reach: (0.0, 0.0),
                steps: 1,
                ..Default::default()
            }
        };
        // Alone, it moves the list.
        let handle = ScrollHandle::new();
        let frame = laid_out(&list(&handle), &ui, &mut text);
        let delta = scrolled(&frame);
        ui.take_scroll(&frame, &[delta]);
        assert_eq!(handle.offset(), (0.0, 30.0), "the compositor's scroll is taken");
        // After the jump, it moves from it.
        let handle = ScrollHandle::new();
        let frame = laid_out(&list(&handle), &ui, &mut text);
        let delta = scrolled(&frame);
        handle.scroll_to((0.0, 200.0));
        ui.take_scroll(&frame, &[delta]);
        assert_eq!(handle.offset(), (0.0, 230.0), "from the jump");
        let next = laid_out(&list(&handle), &ui, &mut text);
        let s = next.nodes[keyed(&next, 2)].scroll(OUTSIDE).expect("a scroll container");
        assert_eq!(s.offset, (0.0, 230.0), "and is laid out");
        // Taken twice before a layout: each lands once, in order.
        ui.take_scroll(&next, &[scrolled_from(&next, 2, 10.0)]);
        ui.take_scroll(&next, &[scrolled_from(&next, 2, 10.0)]);
        assert_eq!(handle.offset(), (0.0, 250.0), "two takes, each once");
    }

    /// The compositor's scroll of the container keyed `key` in `frame`, by
    /// `dy`.
    fn scrolled_from(frame: &Frame, key: u64, dy: f32) -> crate::compositor::ScrollDelta {
        let i = keyed(frame, key);
        crate::compositor::ScrollDelta {
            key: crate::compositor::NodeKey::of(&frame.nodes[i].path),
            owner: owner_of(frame, i),
            delta: motion::units2((0.0, dy)),
            reach: (0.0, 0.0),
            steps: 0,
            ..Default::default()
        }
    }

    /// The last frame showed a text field at 0 and the compositor scrolled
    /// it 30 on from there; before the UI takes that in, an edit brought
    /// the field's caret into view (a jump, the UI's own write, an
    /// assignment). What the UI takes after lands on top of it, as on a
    /// program's scroll (ADR-0032); an offset only clamped is no jump.
    #[test]
    fn the_compositors_scroll_taken_after_a_reveal_moves_from_it() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let field = TextField::new("a long value that runs well past the end of its narrow box, and on");
        let root = crate::element::div()
            .child(text_input(&field).width(60.0))
            .into_element();
        let delta = |frame: &Frame| {
            let i = frame
                .nodes
                .iter()
                .position(|n| matches!(n.content, Content::Input { .. }))
                .expect("the field");
            let s = frame.nodes[i].scroll(OUTSIDE).expect("a field scrolls");
            assert!(s.max.0 > 100.0, "its text runs past its box");
            crate::compositor::ScrollDelta {
                key: crate::compositor::NodeKey::of(&frame.nodes[i].path),
                owner: owner_of(frame, i),
                delta: motion::units2((30.0, 0.0)),
                reach: (0.0, 0.0),
                steps: 1,
                ..Default::default()
            }
        };
        let frame = laid_out(&root, &ui, &mut text);
        let d = delta(&frame);
        let before = field.scroll_generation();
        field.reveal();
        let asked = field.view.with_untracked(|v| v.reveal);
        field.settled(motion::units2((12.0, 0.0)), asked);
        assert_eq!(
            field.scroll_generation(),
            before.wrapping_add(1),
            "the reveal is a jump"
        );
        ui.take_scroll(&frame, &[d]);
        assert_eq!(field.offset(), (42.0, 0.0), "from the reveal");
        // Only clamped (no reveal): no jump, and a take lands from there.
        let frame = laid_out(&root, &ui, &mut text);
        let d = delta(&frame);
        let before = field.scroll_generation();
        field.settled(motion::units2((10.0, 0.0)), asked);
        assert_eq!(field.scroll_generation(), before, "a clamp is no jump");
        ui.take_scroll(&frame, &[d]);
        assert_eq!(field.offset(), (40.0, 0.0), "the delta lands");
    }

    /// A notched wheel's gesture that took a list to its end turns on
    /// against it, through the UI's own wheel: nothing moves, but the list
    /// was scrolled (its overlay scrollbar shows, [`ScrollHandle::wheels`]).
    #[test]
    fn the_uis_wheel_against_an_edge_counts_as_a_scroll() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let handle = ScrollHandle::new();
        let list = crate::element::div()
            .child(
                crate::element::div()
                    .width(100.0)
                    .height(60.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .track_scroll(&handle)
                    .child(crate::element::div().width(100.0).height(400.0)),
            )
            .into_element();
        let frame = laid_out(&list, &ui, &mut text);
        assert!(ui.wheel_at(&frame, notched((10.0, 10.0)), (0.0, 500.0), None).redraw);
        assert_eq!(handle.offset(), (0.0, 340.0), "at its end");
        let wheels = handle.wheels();
        let frame = laid_out(&list, &ui, &mut text);
        let out = ui.wheel_at(&frame, notched((10.0, 10.0)), (0.0, 40.0), None);
        assert!(out.redraw, "taken");
        assert_eq!(handle.offset(), (0.0, 340.0), "nothing moved");
        assert_eq!(handle.wheels(), wheels + 1, "but it was scrolled");
    }

    /// The edge rule the compositor's route has, through the UI's own
    /// wheel: a fresh gesture against the end of a lone list (no container
    /// around it can move that way) is the list's, held against its edge,
    /// and counts as a scroll; a sideways step of a gesture latched to it,
    /// along the axis it does not scroll, is not its and counts for
    /// nothing.
    #[test]
    fn the_uis_wheel_takes_an_edge_by_the_compositors_rule() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let handle = ScrollHandle::new();
        handle.scroll_to((0.0, f32::MAX));
        let list = crate::element::div()
            .child(
                crate::element::div()
                    .width(100.0)
                    .height(60.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .track_scroll(&handle)
                    .child(crate::element::div().width(100.0).height(400.0)),
            )
            .into_element();
        let frame = laid_out(&list, &ui, &mut text);
        let offset = |frame: &Frame| frame.nodes.iter().find_map(|n| n.scroll(OUTSIDE)).map(|s| s.offset);
        assert_eq!(offset(&frame), Some((0.0, 340.0)), "at its end");
        let wheels = handle.wheels();
        let out = ui.wheel_at(&frame, notched((10.0, 10.0)), (0.0, 40.0), None);
        assert!(out.redraw, "a fresh wheel against the end is taken");
        assert_eq!(
            offset(&laid_out(&list, &ui, &mut text)),
            Some((0.0, 340.0)),
            "nothing moved"
        );
        assert_eq!(handle.wheels(), wheels + 1, "but it was scrolled");
        let frame = laid_out(&list, &ui, &mut text);
        let out = ui.wheel_at(&frame, notched((10.0, 10.0)), (40.0, 0.0), None);
        assert!(!out.redraw, "a sideways step is not the list's");
        assert_eq!(handle.wheels(), wheels + 1, "and counts for nothing");
    }

    /// Taking in the compositor's deltas finds each container by its key:
    /// over a frame of 2,000 nodes with one list, the first delta reads the
    /// frame once (its index), and every later one, the frame unchanged,
    /// looks at the list's node alone and allocates nothing.
    #[test]
    fn a_compositor_delta_costs_its_container_not_the_frame() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let handle = ScrollHandle::new();
        let root = crate::element::div()
            .child(
                crate::element::div()
                    .width(100.0)
                    .height(60.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .track_scroll(&handle)
                    .child(crate::element::div().width(100.0).height(40_000.0)),
            )
            .children((0..2_000).map(|_| crate::element::div().width(1.0).height(1.0)))
            .into_element();
        let frame = laid_out(&root, &ui, &mut text);
        assert!(frame.nodes.len() > 2_000);
        let i = frame
            .nodes
            .iter()
            .position(|n| laid_scroll(n).is_some())
            .expect("the list");
        let delta = crate::compositor::ScrollDelta {
            key: crate::compositor::NodeKey::of(&frame.nodes[i].path),
            owner: owner_of(&frame, i),
            delta: motion::units2((0.0, 1.0)),
            reach: (0.0, 0.0),
            steps: 1,
            ..Default::default()
        };
        let visits = || SCROLL_INDEX_VISITS.with(std::cell::Cell::get);
        let before = visits();
        assert!(ui.take_scroll(&frame, &[delta]));
        assert!(visits() - before > 2_000, "the first reads the frame");
        // Warm: what it keeps has grown.
        ui.take_scroll(&frame, &[delta]);
        for k in 0..100 {
            let before = visits();
            let (moved, allocations) = crate::heap::allocations(|| ui.take_scroll(&frame, &[delta]));
            assert!(moved, "delta {k} moves the list");
            assert_eq!(visits() - before, 1, "delta {k} looks at the list alone");
            assert_eq!(allocations, 0, "delta {k} allocates nothing");
        }
        // Each added once to where the UI holds it, no layout between.
        assert_eq!(handle.offset(), (0.0, 102.0));
        // A pass wrote a node in place since (the frame's log has news no
        // reader of it read): every reader on the delta's path, the
        // layers' stack and the containers' index, reads that node alone.
        let mut frame = frame;
        frame.changes.log.push(crate::layout::Changed {
            index: u32::try_from(i).unwrap_or(0),
            before: frame.nodes[i].bounds(OUTSIDE),
            repainted: true,
        });
        let before = visits();
        assert!(ui.take_scroll(&frame, &[delta]));
        let read = visits() - before;
        assert!(
            read <= 3,
            "a change logged costs its node, not the frame: {read} visits"
        );
    }

    /// A box that clicks ahead of 20,000 others, on the frame the screen
    /// shows: once the frame's key index is read, each press, release and
    /// move finds the node the screen shows by a lookup, not a walk of the
    /// frame (a walk from the end visits all 20,000 to find the first).
    #[test]
    fn a_click_and_a_hover_over_a_large_page_cost_a_lookup() {
        let mut text = TextSystem::new();
        let log = Log::default();
        let l = log.clone();
        let root = crate::element::div()
            .child(
                crate::element::div()
                    .width(200.0)
                    .height(60.0)
                    .on_click(move |_| l.borrow_mut().push("click".to_owned())),
            )
            .children((0..20_000).map(|_| crate::element::div().width(1.0).height(1.0)))
            .into_element();
        let frame = laid_out(&root, &InputState::default(), &mut text);
        assert!(frame.nodes.len() > 20_000);
        let mut ui = InputState::showing(&frame, std::sync::Arc::new(painted(&frame)));
        let visits = || SCROLL_INDEX_VISITS.with(std::cell::Cell::get);
        // Warm: the first reads the frame into the index.
        ui.pointer_moved(&frame, &mut text, 20.0, 15.0);
        assert!(ui.hit_shown(&frame, 20.0, 15.0).is_some(), "the box is shown");
        for k in 0..20_u8 {
            let (x, y) = (20.0 + f32::from(k), 15.0);
            let before = visits();
            ui.pointer_moved(&frame, &mut text, x, y);
            ui.mouse_down(&frame, &mut text, x, y);
            ui.mouse_up(&frame, x, y);
            let read = visits() - before;
            assert!(read <= 16, "click {k} costs lookups, not the frame: {read} visits");
        }
        assert_eq!(taken(&log).len(), 20, "each click reached the box");
    }

    /// A menu open over a page of 20,000 nodes: the layer stack reads the
    /// frame once; a write in place since, of a node no layer is, costs
    /// that node (and the layers' roots it may be), not the frame nor a
    /// walk for each layer's owner; one of the menu's root, its Tab owner
    /// asked for unchanged, costs no walk either.
    #[test]
    fn a_write_in_place_under_an_open_menu_costs_what_was_written() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let root = crate::element::div()
            .children((0..20_000).map(|_| crate::element::div().width(1.0).height(1.0)))
            .child(crate::element::deferred(
                crate::element::Positioning::At { x: 10.0, y: 10.0 },
                crate::element::div()
                    .layer(crate::layer::Layer::new())
                    .width(100.0)
                    .height(100.0),
            ))
            .into_element();
        let frame = laid_out(&root, &ui, &mut text);
        assert!(frame.nodes.len() > 20_000);
        assert_eq!(frame.layers.len(), 1, "the menu");
        let visits = || SCROLL_INDEX_VISITS.with(std::cell::Cell::get);
        let before = visits();
        ui.layers.refresh(&frame);
        assert!(visits() - before > 20_000, "the first reads the frame");
        let mut frame = frame;
        for (what, index) in [
            ("an unrelated node", 5_usize),
            ("the menu's root", frame.layers[0].root),
        ] {
            frame.changes.log.push(crate::layout::Changed {
                index: u32::try_from(index).unwrap_or(0),
                before: frame.nodes[index].bounds(OUTSIDE),
                repainted: true,
            });
            let before = visits();
            ui.layers.refresh(&frame);
            let read = visits() - before;
            assert!(read <= 2, "{what} written costs what was written: {read} visits");
            assert_eq!(ui.layers.entries.len(), 1, "the menu still open ({what})");
        }
    }

    /// A trackpad gesture the compositor hands on over a wheel handler on
    /// a page of 20,000 nodes: the first event finds the node it was over
    /// through the frame's index, read once, and every later one, the
    /// frame unchanged, looks at a constant number of nodes, not the frame.
    #[test]
    fn a_wheel_handed_on_over_a_large_frame_finds_its_node_by_an_index() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // The handler last in the frame: no walk from its start finds it
        // cheaply.
        let root = crate::element::div()
            .width(400.0)
            .children((0..20_000).map(|_| crate::element::div().width(1.0).height(1.0)))
            .child(
                crate::element::div()
                    .key(1)
                    .on_wheel(|_| true)
                    .width(200.0)
                    .height(50.0),
            )
            .into_element();
        let frame = laid_out(&root, &ui, &mut text);
        assert!(frame.nodes.len() > 20_000);
        let at = frame
            .nodes
            .last()
            .map(|n| (n.bounds(OUTSIDE).x + 10.0, n.bounds(OUTSIDE).y + 10.0))
            .unwrap_or_default();
        let target = crate::compositor::HitTarget {
            key: crate::compositor::NodeKey::of(&path_at(&frame, at.0, at.1)),
            owner: None,
        };
        let sample = |k: u64| crate::compositor::WheelStep {
            point: at,
            time: crate::clock::current_nanos().saturating_add(k),
            precise: true,
            phase: if k == 0 {
                crate::compositor::WheelPhase::Began
            } else {
                crate::compositor::WheelPhase::Changed
            },
            gesture: Some(1),
        };
        let visits = || SCROLL_INDEX_VISITS.with(std::cell::Cell::get);
        let before = visits();
        assert!(ui.wheel_at(&frame, sample(0), (0.0, 4.0), Some(target)).redraw);
        assert!(visits() - before > 20_000, "the first reads the frame");
        for k in 1..20 {
            let before = visits();
            let out = ui.wheel_at(&frame, sample(k), (0.0, 4.0), Some(target));
            assert!(out.redraw, "sample {k} reaches the handler");
            let read = visits() - before;
            assert!(read <= 2, "sample {k} looks at {read} nodes, not the frame");
        }
    }

    /// A wheel gesture over a list when a modal layer opens over it, with
    /// a scroller of its own under the pointer: the gesture's next events,
    /// still naming the list, find it gone (below the modal) and go to
    /// what is at the pointer now, the modal's scroller.
    #[test]
    fn a_gesture_whose_target_a_modal_covered_goes_on_in_the_modals_scroller() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let scroller = |key: u64| {
            crate::element::div()
                .key(key)
                .width(100.0)
                .height(60.0)
                .overflow_y(crate::style::Overflow::Scroll)
                .child(crate::element::div().width(100.0).height(200.0))
        };
        let page = |modal: bool| {
            let root = crate::element::div().width(400.0).height(100.0).child(scroller(2));
            if modal {
                root.child(crate::element::deferred(
                    crate::element::Positioning::At { x: 0.0, y: 0.0 },
                    crate::element::div()
                        .layer(crate::layer::Layer::new().modal(true))
                        .width(400.0)
                        .height(100.0)
                        .child(scroller(3)),
                ))
            } else {
                root
            }
            .into_element()
        };
        let laid = laid_out(&page(false), &ui, &mut text);
        let i = keyed(&laid, 2);
        let list = crate::compositor::HitTarget {
            key: crate::compositor::NodeKey::of(&laid.nodes[i].path),
            owner: owner_of(&laid, i),
        };
        let pad = |began: bool| crate::compositor::WheelStep {
            point: (50.0, 20.0),
            time: crate::clock::current_nanos(),
            precise: true,
            phase: if began {
                crate::compositor::WheelPhase::Began
            } else {
                crate::compositor::WheelPhase::Changed
            },
            gesture: None,
        };
        ui.wheel_at(&laid, pad(true), (0.0, 30.0), Some(list));
        let modal = laid_out(&page(true), &ui, &mut text);
        let inner = modal.nodes[keyed(&modal, 3)].path.clone();
        ui.wheel_at(&modal, pad(false), (0.0, 30.0), Some(list));
        let offset = ui.state.scroll.get(&(inner, None)).map_or(0.0, |o| motion::px(o.1));
        assert_eq!(offset, 30.0, "the modal's scroller took the gesture on");
    }

    /// A finger panning a list when a modal layer opens over it: its next
    /// moves, captured by the contact that went down on the list, scroll
    /// nothing below the modal, neither the list it latched to nor one
    /// walked up to from where it went down.
    #[test]
    fn a_finger_pan_under_a_modal_opened_mid_gesture_scrolls_nothing() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let page = |modal: bool| {
            let root = crate::element::div().width(400.0).height(100.0).child(
                crate::element::div()
                    .key(2)
                    .width(100.0)
                    .height(60.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .child(crate::element::div().width(100.0).height(200.0)),
            );
            if modal {
                root.child(crate::element::deferred(
                    crate::element::Positioning::At { x: 200.0, y: 0.0 },
                    crate::element::div()
                        .layer(crate::layer::Layer::new().modal(true))
                        .width(100.0)
                        .height(100.0),
                ))
            } else {
                root
            }
            .into_element()
        };
        let laid = laid_out(&page(false), &ui, &mut text);
        let list = laid.nodes[keyed(&laid, 2)].path.clone();
        let offset = |ui: &InputState| {
            ui.state
                .scroll
                .get(&(list.clone(), None))
                .map_or(0.0, |o| motion::px(o.1))
        };
        ui.touch(&laid, &mut text, &contact(1, TouchPhase::Started, 50.0, 50.0));
        ui.touch(&laid, &mut text, &contact(1, TouchPhase::Moved, 50.0, 20.0));
        let panned = offset(&ui);
        assert!(panned > 0.0, "the pan scrolled the list");
        let modal = laid_out(&page(true), &ui, &mut text);
        ui.touch(&modal, &mut text, &contact(1, TouchPhase::Moved, 50.0, -10.0));
        assert_eq!(offset(&ui), panned, "below the modal: no further");
    }

    #[test]
    fn a_wheel_handed_on_with_its_node_goes_to_that_node_while_it_is_there_and_else_is_hit_tested_again() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let wheeled = |who: &'static str, k: u64| {
            let log = log.clone();
            crate::element::div()
                .key(k)
                .on_wheel(move |_| {
                    log.borrow_mut().push(who.to_owned());
                    true
                })
                .width(200.0)
                .height(50.0)
        };
        let column = |first: (&'static str, u64), second: Option<(&'static str, u64)>| {
            let mut root = crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(wheeled(first.0, first.1));
            if let Some((who, k)) = second {
                root = root.child(wheeled(who, k));
            }
            root.into_element()
        };
        let shown = laid_out(&column(("A", 1), Some(("B", 2))), &ui, &mut text);
        let a = path_at(&shown, 10.0, 10.0);
        let target = crate::compositor::HitTarget {
            key: crate::compositor::NodeKey::of(&a),
            owner: None,
        };
        // The UI's frame now has B above A: the wheel the render thread
        // found over A goes to A.
        let now = laid_out(&column(("B", 2), Some(("A", 1))), &ui, &mut text);
        ui.wheel_at(&now, notched((10.0, 10.0)), (0.0, 40.0), Some(target));
        assert_eq!(taken(&log), ["A"]);
        // A gone: the stale target is dropped, as every latch is, and the
        // wheel hit-tested again: B, under the point now.
        let gone = laid_out(&column(("B", 2), None), &ui, &mut text);
        ui.wheel_at(&gone, notched((10.0, 10.0)), (0.0, 40.0), Some(target));
        assert_eq!(taken(&log), ["B"]);
    }

    /// A container 100 px tall scrolling down (key 9, 400 px of content)
    /// holding at its top another (key 3, 60 px tall, 200 px of content).
    fn nested_lists() -> crate::element::Element {
        let inner = crate::element::div()
            .key(3)
            .width(100.0)
            .height(60.0)
            .overflow_y(crate::style::Overflow::Scroll)
            .child(crate::element::div().width(100.0).height(200.0));
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(9)
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(crate::style::Overflow::Scroll)
                    .child(inner)
                    .child(crate::element::div().width(100.0).height(340.0)),
            )
            .into_element()
    }

    #[test]
    fn a_wheel_gesture_stays_on_the_container_it_began_on_and_the_next_passes_one_at_its_end() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let laid = laid_out(&nested_lists(), &ui, &mut text);
        let (outer, inner) = (
            laid.nodes[keyed(&laid, 9)].path.clone(),
            laid.nodes[keyed(&laid, 3)].path.clone(),
        );
        let offset =
            |ui: &InputState, p: &NodePath| ui.state.scroll.get(&(p.clone(), None)).map_or(0.0, |o| motion::px(o.1));
        let wheel = |ui: &mut InputState, text: &mut TextSystem| {
            let frame = laid_out(&nested_lists(), ui, text);
            ui.wheel_at(&frame, notched((20.0, 20.0)), (0.0, 100.0), None);
        };
        // One gesture over the inner list: it scrolls to its end, and the
        // gesture stays on it there; the outer list does not move.
        for _ in 0..3 {
            wheel(&mut ui, &mut text);
        }
        assert_eq!((offset(&ui, &inner), offset(&ui, &outer)), (140.0, 0.0));
        // A new gesture: the inner list is at its end, the outer one scrolls.
        crate::clock::advance_to(std::time::Duration::from_secs(11));
        wheel(&mut ui, &mut text);
        assert_eq!((offset(&ui, &inner), offset(&ui, &outer)), (140.0, 100.0));
    }

    /// A pan is latched per contact: a wheel turned on the outer list, and
    /// a second finger going down, beside a pan that scrolled the inner
    /// list to its end end neither that pan's gesture nor take it over, so
    /// the first finger moving on scrolls nothing.
    #[test]
    fn a_second_contact_leaves_the_first_pans_latch_alone() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let laid = laid_out(&nested_lists(), &ui, &mut text);
        let (outer, inner) = (
            laid.nodes[keyed(&laid, 9)].path.clone(),
            laid.nodes[keyed(&laid, 3)].path.clone(),
        );
        let offset =
            |ui: &InputState, p: &NodePath| ui.state.scroll.get(&(p.clone(), None)).map_or(0.0, |o| motion::px(o.1));
        let touch = |ui: &mut InputState, text: &mut TextSystem, ev: TouchEvent| {
            let frame = laid_out(&nested_lists(), ui, text);
            ui.touch(&frame, text, &ev);
        };
        // The first finger pans the inner list to its end.
        touch(&mut ui, &mut text, contact(1, TouchPhase::Started, 20.0, 50.0));
        touch(&mut ui, &mut text, contact(1, TouchPhase::Moved, 20.0, -100.0));
        assert_eq!((offset(&ui, &inner), offset(&ui, &outer)), (140.0, 0.0));
        // The wheel on the outer list, below the inner one.
        let frame = laid_out(&nested_lists(), &ui, &mut text);
        ui.wheel_at(&frame, notched((20.0, 80.0)), (0.0, 30.0), None);
        assert_eq!((offset(&ui, &inner), offset(&ui, &outer)), (140.0, 30.0));
        // A second finger down; the first moves on: its pan stays on the
        // inner list, at its end, and the outer one where the wheel left it.
        touch(&mut ui, &mut text, contact(2, TouchPhase::Started, 50.0, 80.0));
        touch(&mut ui, &mut text, contact(1, TouchPhase::Moved, 20.0, -200.0));
        assert_eq!((offset(&ui, &inner), offset(&ui, &outer)), (140.0, 30.0));
    }

    /// A finger pan handled after the program's `scroll_to` (queued while
    /// the UI ran it): writes are ordered as the UI makes them (ADR-0032,
    /// I5), no clock between them, so the pan moves the list from where
    /// the jump put it, and the jump refuses nothing.
    #[test]
    fn a_pan_handled_after_a_jump_moves_from_it() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let h = ScrollHandle::new();
        let list = |h: &ScrollHandle| {
            crate::element::div()
                .child(
                    crate::element::div()
                        .key(2)
                        .width(100.0)
                        .height(60.0)
                        .overflow_y(crate::style::Overflow::Scroll)
                        .track_scroll(h)
                        .child(crate::element::div().width(100.0).height(400.0)),
                )
                .into_element()
        };
        h.scroll_to((0.0, 100.0));
        for ev in [
            contact(1, TouchPhase::Started, 20.0, 50.0),
            contact(1, TouchPhase::Moved, 20.0, 20.0),
        ] {
            let frame = laid_out(&list(&h), &ui, &mut text);
            ui.touch(&frame, &mut text, &ev);
        }
        assert!(h.offset().1 > 100.0, "from where the program put it: {:?}", h.offset());
    }

    /// Two lists side by side, each 100 px wide and 60 px tall (200 px of
    /// content), keys 1 and 2, whose wheel handlers decline: the UI
    /// scrolls them.
    fn two_declining_lists() -> crate::element::Element {
        let list = |k: u64| {
            crate::element::div()
                .key(k)
                .position(crate::style::Position::Absolute)
                .left(if k == 1 { 0.0 } else { 100.0 })
                .top(0.0)
                .width(100.0)
                .height(60.0)
                .overflow_y(crate::style::Overflow::Scroll)
                .on_wheel(|_| false)
                .child(crate::element::div().width(100.0).height(200.0))
        };
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(list(1))
            .child(list(2))
            .into_element()
    }

    /// The UI latches a gesture as the compositor does: a notched wheel
    /// moved onto another list, however soon and however little, begins
    /// another gesture there; a trackpad's, resting past the notched
    /// timeout, is one until its fingers lift.
    #[test]
    fn a_wheel_the_ui_scrolls_is_latched_by_the_compositors_rules() {
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        crate::clock::script(std::time::Duration::from_secs(10));
        let laid = laid_out(&two_declining_lists(), &ui, &mut text);
        let (a, b) = (
            laid.nodes[keyed(&laid, 1)].path.clone(),
            laid.nodes[keyed(&laid, 2)].path.clone(),
        );
        let offsets = |ui: &InputState| {
            let of = |p: &NodePath| ui.state.scroll.get(&(p.clone(), None)).map_or(0.0, |o| motion::px(o.1));
            (of(&a), of(&b))
        };
        let wheel = |ui: &mut InputState, text: &mut TextSystem, step: crate::compositor::WheelStep| {
            let frame = laid_out(&two_declining_lists(), ui, text);
            ui.wheel_at(&frame, step, (0.0, 30.0), None);
        };
        // A notched wheel on A by its edge, then 10 ms later 8 px on, over B.
        wheel(&mut ui, &mut text, notched((96.0, 20.0)));
        crate::clock::advance_to(std::time::Duration::from_millis(10_010));
        wheel(&mut ui, &mut text, notched((104.0, 20.0)));
        assert_eq!(offsets(&ui), (30.0, 30.0), "(A, B): the second turn scrolls B");
        // A trackpad's fingers down on A, resting 600 ms, moving on over B:
        // A's gesture still.
        let pad = |point: (f32, f32), began: bool| crate::compositor::WheelStep {
            point,
            time: crate::clock::current_nanos(),
            precise: true,
            phase: if began {
                crate::compositor::WheelPhase::Began
            } else {
                crate::compositor::WheelPhase::Changed
            },
            gesture: None,
        };
        crate::clock::advance_to(std::time::Duration::from_secs(12));
        wheel(&mut ui, &mut text, pad((50.0, 20.0), true));
        crate::clock::advance_to(std::time::Duration::from_millis(12_600));
        wheel(&mut ui, &mut text, pad((150.0, 20.0), false));
        assert_eq!(offsets(&ui), (90.0, 30.0), "(A, B): the trackpad's gesture stays on A");
    }

    /// Two contacts and the mouse on box A in turn, as
    /// [`captures`] lays it out.
    fn three_pointers_on_a(ui: &mut InputState, frame: &Frame, text: &mut TextSystem) {
        ui.touch(frame, text, &contact(1, TouchPhase::Started, 10.0, 10.0));
        ui.touch(frame, text, &contact(2, TouchPhase::Started, 10.0, 30.0));
        ui.touch(frame, text, &contact(1, TouchPhase::Moved, 60.0, 10.0));
        ui.touch(frame, text, &contact(2, TouchPhase::Moved, 60.0, 30.0));
        ui.touch(frame, text, &contact(2, TouchPhase::Ended, 60.0, 30.0));
        ui.mouse_down(frame, text, 100.0, 20.0);
        ui.touch(frame, text, &contact(1, TouchPhase::Cancelled, 60.0, 10.0));
        ui.mouse_up(frame, 100.0, 20.0);
    }

    #[test]
    fn a_node_is_captured_by_the_first_pointer_unless_it_takes_several() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();

        // One pointer captures A: the second contact and the mouse, which
        // press it while the first holds it, get nothing of it, neither
        // drag nor raw touch.
        let single = laid_out(&captures(&log, false, None), &ui, &mut text);
        three_pointers_on_a(&mut ui, &single, &mut text);
        assert_eq!(
            taken(&log),
            [
                "A touch 1 Started",
                "A touch 1 Moved",
                "A Contact(1) Start",
                "A Contact(1) Move",
                "A touch 1 Cancelled",
                "A Contact(1) Cancel",
            ]
        );

        // Taking several pointers, each gets its own drag and touches.
        let multi = laid_out(&captures(&log, true, None), &ui, &mut text);
        three_pointers_on_a(&mut ui, &multi, &mut text);
        assert_eq!(
            taken(&log),
            [
                "A touch 1 Started",
                "A touch 2 Started",
                "A touch 1 Moved",
                "A Contact(1) Start",
                "A Contact(1) Move",
                "A touch 2 Moved",
                "A Contact(2) Start",
                "A Contact(2) Move",
                "A Contact(2) End",
                "A touch 2 Ended",
                "A Mouse Start",
                "A touch 1 Cancelled",
                "A Contact(1) Cancel",
                "A Mouse End",
            ]
        );
    }

    #[test]
    fn a_pointer_captures_every_node_of_a_capture_group() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // A and B are one group: a finger dragging A holds B too, so a
        // second finger on B gets nothing of it; apart, B is its own.
        for (group, b_gets) in [(Some(crate::events::CaptureGroup::new()), false), (None, true)] {
            let frame = laid_out(&captures(&log, false, group), &ui, &mut text);
            ui.touch(&frame, &mut text, &contact(1, TouchPhase::Started, 10.0, 10.0));
            ui.touch(&frame, &mut text, &contact(1, TouchPhase::Moved, 60.0, 10.0));
            ui.touch(&frame, &mut text, &contact(2, TouchPhase::Started, 10.0, 60.0));
            ui.touch(&frame, &mut text, &contact(2, TouchPhase::Moved, 60.0, 60.0));
            ui.touch(&frame, &mut text, &contact(2, TouchPhase::Ended, 60.0, 60.0));
            ui.touch(&frame, &mut text, &contact(1, TouchPhase::Ended, 60.0, 10.0));
            let b = taken(&log).into_iter().filter(|l| l.starts_with('B')).count();
            assert_eq!(b > 0, b_gets, "group {group:?}: B got {b} events");
        }
    }

    #[test]
    fn a_drag_of_a_node_gone_with_its_component_calls_nothing_of_it() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // The handler reads a signal its component owns: called once the
        // component is disposed, it would read a disposed signal.
        let scope = crate::reactive::Scope::new();
        let tree = scope.run(|| {
            let held = Signal::new(0_u32);
            let dragged = log.clone();
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    crate::element::div()
                        .key(1)
                        .on_drag(move |e| {
                            held.set(held.get_untracked().saturating_add(1));
                            dragged.borrow_mut().push(format!("{:?}", e.phase));
                        })
                        .width(100.0)
                        .height(40.0),
                )
                .into_element()
        });
        let frame = laid_out(&tree, &ui, &mut text);
        let gone = laid_out(
            &crate::element::div().width(400.0).height(100.0).into_element(),
            &ui,
            &mut text,
        );
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        assert_eq!(taken(&log), ["Start"]);
        // The component goes, and its node with it: the drag is let go of
        // without calling into it.
        scope.dispose();
        ui.rehover(&gone, &mut text);
        ui.mouse_up(&gone, 10.0, 10.0);
        assert!(taken(&log).is_empty(), "nothing of a disposed component runs");
    }

    /// A box keyed 1 at the top left, 100 by 30, [`tapped`] as `who`.
    fn keyed_one(log: &Log, who: &'static str) -> crate::element::Element {
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(tapped(crate::element::div().key(1), who, log).width(100.0).height(30.0))
            .into_element()
    }

    #[test]
    fn a_node_another_instance_made_at_its_path_in_one_rebuild_is_another_node() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // Two owners, as two components of different types at the same
        // place: the second's box has the first's path.
        let (a, b) = (crate::reactive::Scope::new(), crate::reactive::Scope::new());
        let first = laid_out(&a.run(|| keyed_one(&log, "A")), &ui, &mut text);
        let replaced = laid_out(&b.run(|| keyed_one(&log, "B")), &ui, &mut text);
        // Pressed on A, released on B, which replaced it in one rebuild:
        // A's drag is cancelled, and B gets no end, nor a click, nor A's.
        ui.mouse_down(&first, &mut text, 20.0, 15.0);
        ui.mouse_up(&replaced, 20.0, 15.0);
        assert_eq!(taken(&log), ["A mouse down", "A Start", "A Cancel"]);
        // The same by touch: the tap clicks nothing.
        ui.touch(&first, &mut text, &finger(TouchPhase::Started, 20.0, 15.0));
        ui.touch(&replaced, &mut text, &finger(TouchPhase::Ended, 20.0, 15.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Cancelled"]);
    }

    #[test]
    fn a_mouse_press_whose_node_went_clicks_nothing_where_it_is_released() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let both = laid_out(&family(&log, &[1, 2]), &ui, &mut text);
        let without_a = laid_out(&family(&log, &[2]), &ui, &mut text);
        // Down on A, A removed, up on the parent where A was: Blink forgets
        // the mouse down node as it is removed, and clicks nothing (a
        // finger's tap clicks the parent left under it).
        ui.mouse_down(&both, &mut text, 20.0, 15.0);
        ui.rehover(&without_a, &mut text);
        ui.mouse_up(&without_a, 150.0, 60.0);
        assert_eq!(taken(&log), ["A mouse down", "A Start", "A Cancel"]);
    }

    /// A component (by `which`: "A", or "B", a component of another type)
    /// holding the box keyed 1 [`keyed_one`] lays out; each drag event
    /// logs whether the component was alive as it ran, and the drag's
    /// start sets a timer 100 ms out, which logs.
    fn swapped(log: &Log, which: Signal<&'static str>) -> crate::element::Element {
        fn logged(log: &Log, who: &'static str) -> crate::element::Element {
            let (dragged, timed) = (log.clone(), log.clone());
            let mine = Signal::new(0_u32);
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    crate::element::div()
                        .key(1)
                        .on_drag(move |e| {
                            if e.phase == DragPhase::Start {
                                let timed = timed.clone();
                                crate::clock::after(std::time::Duration::from_millis(100), move || {
                                    timed.borrow_mut().push(format!("{who} timer"));
                                });
                            }
                            dragged
                                .borrow_mut()
                                .push(format!("{who} {:?} alive {}", e.phase, mine.is_alive()));
                        })
                        .width(100.0)
                        .height(30.0),
                )
                .into_element()
        }
        let (a, b) = (log.clone(), log.clone());
        match which.get() {
            "A" => crate::component::component((), move |_| move || logged(&a, "A")).into_element(),
            _ => crate::component::component((0_u8,), move |_| move || logged(&b, "B")).into_element(),
        }
    }

    #[test]
    fn a_keyed_component_moved_among_its_siblings_keeps_its_drag() {
        use crate::component::component;
        let log = Log::default();
        let swapped = Signal::new(false);
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(
            move || {
                let row = |who: &'static str, key: u64| {
                    let l = l.clone();
                    // Its root unkeyed: the component's key keeps its place.
                    component(who, move |cx| {
                        move || {
                            let (who, l) = (cx.props().get_untracked(), l.clone());
                            crate::element::div()
                                .width(100.0)
                                .height(30.0)
                                .on_drag(move |e| l.borrow_mut().push(format!("{who} {:?}", e.phase)))
                                .into_element()
                        }
                    })
                    .key(crate::element::ElementKey::from(key))
                    .into_element()
                };
                let (a, b) = (row("A", 1), row("B", 2));
                let rows = if swapped.get() { [b, a] } else { [a, b] };
                crate::element::div()
                    .width(400.0)
                    .height(100.0)
                    .display(crate::style::Display::Flex)
                    .flex_direction(crate::style::FlexDirection::Column)
                    .children(rows)
                    .into_element()
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("a frame");
        s.hover(20.0, 15.0);
        s.send(crate::window::Input::MouseDown { x: 20.0, y: 15.0 });
        s.hover(30.0, 15.0);
        // B moves above A mid-drag: A is the same node, and keeps it.
        s.act("swap", || swapped.set(true));
        s.settle().expect("a frame");
        s.hover(40.0, 15.0);
        s.send(crate::window::Input::MouseUp { x: 40.0, y: 15.0 });
        let got = taken(&log);
        assert!(!got.contains(&"A Cancel".to_owned()), "{got:?}");
        assert_eq!(got.first().map(String::as_str), Some("A Start"), "{got:?}");
        assert_eq!(got.last().map(String::as_str), Some("A End"), "{got:?}");
        assert!(got.iter().all(|e| e.starts_with('A')), "{got:?}");
    }

    /// A box that clicks, made by component A, is on screen; B, another
    /// component, replaces it at the same path in a commit not yet
    /// presented. A click on what the screen shows is A's: B, which the
    /// screen never showed there, is not clicked.
    #[test]
    fn a_click_on_a_box_another_instance_replaced_since_it_was_shown_reaches_neither() {
        use crate::component::component;
        let log = Log::default();
        let which = Signal::new("A");
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(
            move || {
                let clickable = |who: &'static str, l: Log| {
                    crate::element::div()
                        .width(200.0)
                        .height(60.0)
                        .on_click(move |_| l.borrow_mut().push(format!("{who} click")))
                        .into_element()
                };
                let (a, b) = (l.clone(), l.clone());
                match which.get() {
                    "A" => component((), move |_| move || clickable("A", a.clone())).into_element(),
                    _ => component((0_u8,), move |_| move || clickable("B", b.clone())).into_element(),
                }
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("A presented");
        s.act("B replaces A", || which.set("B"));
        s.click(20.0, 15.0);
        let got = taken(&log);
        assert!(got.is_empty(), "B is not clicked: {got:?}");
    }

    #[test]
    fn a_press_on_a_bare_node_whose_component_was_replaced_clicks_nothing() {
        use crate::component::component;
        let log = Log::default();
        let which = Signal::new("A");
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(
            move || {
                // A box that clicks, holding a bare one where it is pressed:
                // the same paths in A and in B, components of two types.
                let clickable = |who: &'static str, l: Log| {
                    crate::element::div()
                        .width(200.0)
                        .height(60.0)
                        .on_click(move |_| l.borrow_mut().push(format!("{who} click")))
                        .child(crate::element::div().width(100.0).height(30.0))
                        .into_element()
                };
                let (a, b) = (l.clone(), l.clone());
                match which.get() {
                    "A" => component((), move |_| move || clickable("A", a.clone())).into_element(),
                    _ => component((0_u8,), move |_| move || clickable("B", b.clone())).into_element(),
                }
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("a frame");
        s.hover(20.0, 15.0);
        s.send(crate::window::Input::MouseDown { x: 20.0, y: 15.0 });
        s.act("replace", || which.set("B"));
        s.settle().expect("a frame");
        s.send(crate::window::Input::MouseUp { x: 20.0, y: 15.0 });
        assert!(taken(&log).is_empty(), "neither A's box nor B's clicks");
    }

    #[test]
    fn a_parents_callback_a_child_calls_runs_in_the_parent_and_outlives_the_child() {
        use crate::component::{Callback, component};
        let log = Log::default();
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(
            move || {
                let l = l.clone();
                component((), move |_| {
                    let shown = Signal::new(true);
                    move || {
                        let timed = l.clone();
                        // Made in the parent's render: it sets the parent's
                        // timer, and hides the child that calls it.
                        let act = Callback::new(move |()| {
                            let timed = timed.clone();
                            crate::clock::after(std::time::Duration::from_millis(100), move || {
                                timed.borrow_mut().push("parent timer".to_owned());
                            });
                            shown.set(false);
                        });
                        let child = component(act, |cx| {
                            move || {
                                let act = cx.props().get_untracked();
                                crate::element::div()
                                    .width(100.0)
                                    .height(30.0)
                                    .on_click(move |_| act.call(()))
                                    .into_element()
                            }
                        });
                        crate::element::div()
                            .width(400.0)
                            .height(100.0)
                            .child(if shown.get() {
                                child.into_element()
                            } else {
                                crate::element::div().into_element()
                            })
                            .into_element()
                    }
                })
                .into_element()
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("a frame");
        s.hover(20.0, 15.0);
        s.send(crate::window::Input::MouseDown { x: 20.0, y: 15.0 });
        s.send(crate::window::Input::MouseUp { x: 20.0, y: 15.0 });
        s.settle().expect("a frame");
        // The child that called it is gone; the parent's timer is not.
        s.wait(std::time::Duration::from_millis(200));
        s.settle().expect("a frame");
        assert_eq!(taken(&log), ["parent timer"]);
    }

    #[test]
    fn a_binds_on_change_a_child_writes_runs_in_the_parent_and_outlives_the_child() {
        use crate::component::{Bind, component};
        let log = Log::default();
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(
            move || {
                let l = l.clone();
                component((), move |_| {
                    let shown = Signal::new(true);
                    move || {
                        let timed = l.clone();
                        // Given in the parent's render: on a write it sets
                        // the parent's timer, and hides the child that wrote.
                        let bind = Bind::value(0_i32).on_change(move |v: i32| {
                            let timed = timed.clone();
                            crate::clock::after(std::time::Duration::from_millis(100), move || {
                                timed.borrow_mut().push(format!("parent timer {v}"));
                            });
                            shown.set(false);
                        });
                        let child = component(bind, |cx| {
                            move || {
                                let bind = cx.props().get_untracked();
                                crate::element::div()
                                    .width(100.0)
                                    .height(30.0)
                                    .on_click(move |_| bind.emit(1))
                                    .into_element()
                            }
                        });
                        crate::element::div()
                            .width(400.0)
                            .height(100.0)
                            .child(if shown.get() {
                                child.into_element()
                            } else {
                                crate::element::div().into_element()
                            })
                            .into_element()
                    }
                })
                .into_element()
            },
            400,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("a frame");
        s.hover(20.0, 15.0);
        s.send(crate::window::Input::MouseDown { x: 20.0, y: 15.0 });
        s.send(crate::window::Input::MouseUp { x: 20.0, y: 15.0 });
        s.settle().expect("a frame");
        // The child that wrote is gone; the parent's timer is not.
        s.wait(std::time::Duration::from_millis(200));
        s.settle().expect("a frame");
        assert_eq!(taken(&log), ["parent timer 1"]);
    }

    #[test]
    fn a_drag_whose_component_leaves_is_cancelled_while_it_is_alive_and_what_it_made_goes_with_it() {
        let log = Log::default();
        let which = Signal::new("A");
        let l = log.clone();
        let mut s = crate::app::Scenario::measuring(move || swapped(&l, which), 400, 100, 1.0).expect("a scenario");
        s.settle().expect("a frame");
        s.hover(20.0, 15.0);
        s.send(crate::window::Input::MouseDown { x: 20.0, y: 15.0 });
        s.hover(30.0, 15.0);
        assert_eq!(taken(&log), ["A Start alive true", "A Move alive true"]);
        // Replaced by a component of another type in one rebuild, its box
        // at the same path: A's drag is cancelled before A is disposed, run
        // inside it; B gets nothing of it.
        s.act("swap", || which.set("B"));
        s.settle().expect("a frame");
        s.send(crate::window::Input::MouseUp { x: 30.0, y: 15.0 });
        assert_eq!(taken(&log), ["A Cancel alive true"]);
        // The timer A's drag set went with A.
        s.wait(std::time::Duration::from_millis(200));
        s.settle().expect("a frame");
        assert!(taken(&log).is_empty(), "nothing of A runs once it is gone");
    }

    /// A parent with a click handler holding children keyed 1 and 2 (in
    /// `keys`), each 100 by 30 and clicked by its own handler, one above
    /// the other below 10 px of the parent's padding.
    fn family(log: &Log, keys: &[u64]) -> crate::element::Element {
        let clicked = log.clone();
        let mut parent = crate::element::div()
            .key(9)
            .on_click(move |_| clicked.borrow_mut().push("parent click".to_owned()))
            .padding(10.0)
            .width(200.0)
            .height(90.0);
        for &k in keys {
            parent = parent.child(
                tapped(crate::element::div().key(k), if k == 1 { "A" } else { "B" }, log)
                    .width(100.0)
                    .height(30.0),
            );
        }
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(parent)
            .into_element()
    }

    #[test]
    fn input_lands_where_its_frame_showed_a_scale_the_ui_has_since_ended() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        // The UI's frame now: the animation ended, the button at 1x and no
        // longer moving. The frame the press came under showed it at 2x.
        let rested = laid_out(&scaled_button(&log, 1.0), &ui, &mut text);
        let key = crate::compositor::NodeKey::of(&path_at(&rested, 10.0, 10.0));
        let turned = crate::style::Painted {
            scale: 2.0,
            rotate: 0.0,
            origin: (0.0, 0.0),
        };
        // The frame shown: the button at 2x, as the compositor sampled it.
        let showed = laid_out(&scaled_button(&log, 2.0), &ui, &mut text);
        ui.show(Some(showing(
            &showed,
            &mut text,
            crate::compositor::Presented {
                turned: vec![(key, turned)],
                ..crate::compositor::Presented::default()
            },
        )));
        // 150 in, past the box at 1x, on it as shown: 75 into it.
        ui.mouse_down(&rested, &mut text, 150.0, 10.0);
        ui.mouse_up(&rested, 150.0, 10.0);
        assert_eq!(taken(&log), ["Start 75,5", "End 75,5", "click"]);
    }

    #[test]
    fn a_press_released_on_a_sibling_clicks_what_they_share() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = laid_out(&family(&log, &[1, 2]), &ui, &mut text);
        // Down on A, up on B: the click goes to their nearest common
        // ancestor with a click handler, the parent, as in Chrome.
        ui.mouse_down(&frame, &mut text, 20.0, 20.0);
        ui.mouse_up(&frame, 20.0, 50.0);
        assert_eq!(taken(&log), ["A mouse down", "A Start", "A End", "parent click"]);
    }

    #[test]
    fn a_tap_whose_child_went_under_the_finger_clicks_the_parent_left_there() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let both = laid_out(&family(&log, &[1, 2]), &ui, &mut text);
        let without_a = laid_out(&family(&log, &[2]), &ui, &mut text);
        // The parent alone: both children gone.
        let a_only_gone = laid_out(
            &{
                let clicked = log.clone();
                crate::element::div()
                    .width(400.0)
                    .height(100.0)
                    .child(
                        crate::element::div()
                            .key(9)
                            .on_click(move |_| clicked.borrow_mut().push("parent click".to_owned()))
                            .padding(10.0)
                            .width(200.0)
                            .height(90.0),
                    )
                    .into_element()
            },
            &ui,
            &mut text,
        );
        // The child the finger went down on is removed and nothing takes
        // its place: the tap clicks the parent under the lift point (WPT's
        // `pointerevent_touch_target_after_pointerdown_target_removed`).
        ui.touch(&both, &mut text, &finger(TouchPhase::Started, 20.0, 20.0));
        ui.touch(&a_only_gone, &mut text, &finger(TouchPhase::Ended, 20.0, 20.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Cancelled", "parent click"]);
        // B, moved up into A's place, is a sibling that took its place: it
        // gets nothing of the tap.
        ui.touch(&both, &mut text, &finger(TouchPhase::Started, 20.0, 20.0));
        ui.touch(&without_a, &mut text, &finger(TouchPhase::Ended, 20.0, 20.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Cancelled"]);
        // A removed while the finger rests, then a new A in its place as it
        // lifts: the same path, not the node it went down on; the tap
        // clicks nothing.
        ui.touch(&both, &mut text, &finger(TouchPhase::Started, 20.0, 20.0));
        ui.touch(&a_only_gone, &mut text, &finger(TouchPhase::Moved, 20.0, 20.0));
        ui.touch(&both, &mut text, &finger(TouchPhase::Ended, 20.0, 20.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Cancelled"]);
    }

    #[test]
    fn a_disabled_press_released_on_its_enabled_parent_clicks_nothing() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let tree = |disabled: bool| {
            let clicked = log.clone();
            crate::element::div()
                .width(400.0)
                .height(100.0)
                .child(
                    crate::element::div()
                        .key(1)
                        .on_click(move |_| clicked.borrow_mut().push("parent click".to_owned()))
                        .padding(20.0)
                        .width(200.0)
                        .height(80.0)
                        .child(crate::element::div().key(2).disabled(disabled).width(50.0).height(20.0)),
                )
                .into_element()
        };
        let off = laid_out(&tree(true), &ui, &mut text);
        let on = laid_out(&tree(false), &ui, &mut text);

        // Down on the disabled child, up on the parent's padding: the
        // press reaches nothing, so neither does the click (Chrome clicks
        // the common ancestor of down and up, and a disabled down target
        // suppresses it).
        ui.mouse_down(&off, &mut text, 30.0, 30.0);
        ui.mouse_up(&off, 5.0, 5.0);
        assert!(taken(&log).is_empty(), "a disabled press clicks nothing");

        // The same with the child enabled: both reach the parent, which
        // clicks.
        ui.mouse_down(&on, &mut text, 30.0, 30.0);
        ui.mouse_up(&on, 5.0, 5.0);
        assert_eq!(taken(&log), ["parent click"]);
    }

    #[test]
    fn a_drag_handler_that_panics_leaves_the_outer_claim_as_it_was() {
        let outer = claiming(|| {
            claim_press();
            let unwound = std::panic::catch_unwind(|| claiming(|| panic!("a handler panics")));
            assert!(unwound.is_err());
        });
        assert!(outer, "the outer claim survives a nested handler's panic");
    }

    #[test]
    fn a_drag_event_dispatched_inside_another_keeps_each_ones_claim() {
        // The outer handler claims, then a nested one runs: the outer
        // claim stands, and the nested one's is its own.
        let mut inner = None;
        let outer = claiming(|| {
            claim_press();
            inner = Some(claiming(|| {}));
        });
        assert!(outer, "the outer claim survives the nested dispatch");
        assert_eq!(inner, Some(false), "the nested handler claimed nothing");
        // And a nested claim is not the outer handler's.
        let outer = claiming(|| {
            inner = Some(claiming(claim_press));
        });
        assert!(!outer, "the nested claim is not the outer's");
        assert_eq!(inner, Some(true));
    }

    /// A keyed box of 100 by 40 below `before` px, logging pointer moves
    /// and hover as "A".
    fn pointed(log: &Log, before: f32) -> crate::element::Element {
        let (moved, hovered) = (log.clone(), log.clone());
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(crate::element::div().width(100.0).height(before))
            .child(
                crate::element::div()
                    .key(1)
                    .on_pointer_move(move |e| moved.borrow_mut().push(format!("A move {:.0},{:.0}", e.x, e.y)))
                    .on_hover(move |on| hovered.borrow_mut().push(format!("A hover {on}")))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn boxes_moving_under_a_resting_pointer_change_hover_but_are_no_pointer_move() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let first = laid_out(&pointed(&log, 0.0), &ui, &mut text);

        ui.pointer_moved(&first, &mut text, 30.0, 10.0);
        assert_eq!(taken(&log), ["A hover true", "A move 30,10"]);
        // The frame its handler asked for places it again: nothing.
        ui.rehover(&first, &mut text);
        let again = laid_out(&pointed(&log, 0.0), &ui, &mut text);
        ui.rehover(&again, &mut text);
        assert_eq!(taken(&log), Vec::<String>::new(), "placed again");
        // It moves away from under the pointer and back: hover alone.
        let away = laid_out(&pointed(&log, 50.0), &ui, &mut text);
        ui.rehover(&away, &mut text);
        let back = laid_out(&pointed(&log, 0.0), &ui, &mut text);
        ui.rehover(&back, &mut text);
        assert_eq!(taken(&log), ["A hover false", "A hover true"], "moved under it");
        // A move the pointer makes is one.
        ui.pointer_moved(&back, &mut text, 31.0, 10.0);
        assert_eq!(taken(&log), ["A move 31,10"]);
    }

    #[test]
    fn a_tap_while_the_mouse_drags_leaves_the_mouse_press_whole() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let frame = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);
        let path_a = frame.nodes[hit(&frame, 10.0, 10.0).expect("A")].path.clone();

        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        assert_eq!(taken(&log), ["A mouse down", "A Start"]);
        // A finger taps B: the tap is whole, and the mouse's press is not
        // touched.
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 50.0));
        assert_eq!(
            taken(&log),
            [
                "B touch Started",
                "B mouse down",
                "B Start",
                "B End",
                "B click",
                "B touch Ended"
            ]
        );
        assert!(ui.state().active.contains(&path_a), "A shows pressed again");
        // The mouse rests where it was: a frame is no move of A's drag.
        ui.rehover(&frame, &mut text);
        assert_eq!(taken(&log), Vec::<String>::new(), "no move to the tap");
        ui.pointer_moved(&frame, &mut text, 20.0, 10.0);
        ui.mouse_up(&frame, 20.0, 10.0);
        assert_eq!(taken(&log), ["A Move", "A End", "A click"]);
    }

    fn contact(id: u64, phase: TouchPhase, x: f32, y: f32) -> TouchEvent {
        TouchEvent {
            id,
            kind: PointerKind::Touch,
            phase,
            x,
            y,
        }
    }

    /// The path of the node under `(x, y)`.
    fn path_at(frame: &Frame, x: f32, y: f32) -> NodePath {
        frame.nodes[hit(frame, x, y).expect("a node")].path.clone()
    }

    /// A box at 0..100 by 0..40 clicked as "A", and below it at 40..80 a
    /// box "P" whose drag claims its press once it moves, as a carousel
    /// dragged past its slop does.
    fn claimed(log: &Log) -> crate::element::Element {
        let (click, drag) = (log.clone(), log.clone());
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .key(1)
                    .on_click(move |_| click.borrow_mut().push("A click".to_owned()))
                    .width(100.0)
                    .height(40.0),
            )
            .child(
                crate::element::div()
                    .key(2)
                    .on_drag(move |e| {
                        drag.borrow_mut().push(format!("P {:?}", e.phase));
                        if e.phase == DragPhase::Move {
                            claim_press();
                        }
                    })
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_finger_pan_that_claims_its_press_leaves_the_mouse_click_alone() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = laid_out(&claimed(&log), &ui, &mut text);

        // The mouse holds A while a finger drags P, which claims the
        // finger's press: the mouse's release on A still clicks it.
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 10.0, 80.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 80.0));
        ui.mouse_up(&frame, 10.0, 10.0);
        assert_eq!(taken(&log), ["P Start", "P Move", "P End", "A click"]);

        // The finger lifts after the mouse is up: the same.
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 10.0, 80.0));
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.mouse_up(&frame, 10.0, 10.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 80.0));
        assert_eq!(taken(&log), ["P Start", "P Move", "A click", "P End"]);

        // The mouse's own drag claims its own press: no click on release
        // over P, and a finger tapping A meanwhile still clicks it.
        ui.mouse_down(&frame, &mut text, 10.0, 50.0);
        ui.pointer_moved(&frame, &mut text, 10.0, 60.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        ui.mouse_up(&frame, 10.0, 60.0);
        assert_eq!(taken(&log), ["P Start", "P Move", "A click", "P End"]);
    }

    #[test]
    fn each_pointer_shows_its_own_press_whatever_the_others_do() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let frame = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);
        let (path_a, path_b) = (path_at(&frame, 10.0, 10.0), path_at(&frame, 10.0, 50.0));

        // A finger rests on B; the mouse presses A and lets go: B is still
        // pressed by the finger.
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        assert!(ui.state().active.contains(&path_a) && ui.state().active.contains(&path_b));
        ui.mouse_up(&frame, 10.0, 10.0);
        assert!(!ui.state().active.contains(&path_a), "the mouse let go of A");
        assert!(
            ui.state().active.contains(&path_b),
            "the finger still presses B after the mouse's release"
        );
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 10.0, 50.0));
        assert!(ui.state().active.is_empty());
        assert_eq!(
            taken(&log),
            [
                "B touch Started",
                "A mouse down",
                "A Start",
                "A End",
                "A click",
                "B mouse down",
                "B Start",
                "B End",
                "B click",
                "B touch Ended"
            ]
        );

        // The mouse holds A while a finger pans over B: A stays pressed,
        // and the pan scrolls or drags nothing of the mouse's.
        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 60.0, 50.0));
        assert!(ui.state().active.contains(&path_a), "the mouse still presses A");
        assert!(!ui.state().active.contains(&path_b), "the pan ended B's pressed look");
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 60.0, 50.0));
        assert!(ui.state().active.contains(&path_a));
        ui.mouse_up(&frame, 10.0, 10.0);
        assert_eq!(
            taken(&log),
            [
                "A mouse down",
                "A Start",
                "B touch Started",
                "B touch Moved",
                "B Start",
                "B Move",
                "B End",
                "B touch Ended",
                "A End",
                "A click"
            ]
        );
    }

    #[test]
    fn a_second_finger_is_a_pointer_of_its_own_and_neither_is_a_tap() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let frame = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);
        let (path_a, path_b) = (path_at(&frame, 10.0, 10.0), path_at(&frame, 10.0, 50.0));

        // Two fingers down, on A and on B: each presses its own, each gets
        // its own raw phases, and as one lifts the other is still pressed.
        // Neither is a tap (Chrome's `CancelTaps` on a second pointer).
        ui.touch(&frame, &mut text, &contact(1, TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &contact(2, TouchPhase::Started, 10.0, 50.0));
        assert!(ui.state().active.contains(&path_a) && ui.state().active.contains(&path_b));
        ui.touch(&frame, &mut text, &contact(2, TouchPhase::Ended, 10.0, 50.0));
        assert!(ui.state().active.contains(&path_a), "the first finger still presses A");
        assert!(!ui.state().active.contains(&path_b));
        // Nor a long press, held long after.
        let later = crate::clock::current_seconds() + crate::gesture::LONG_PRESS.as_secs_f32() + 0.1;
        crate::clock::advance_to(std::time::Duration::from_secs_f32(later));
        ui.long_presses(&frame);
        ui.touch(&frame, &mut text, &contact(1, TouchPhase::Ended, 10.0, 10.0));
        assert!(ui.state().active.is_empty());
        assert_eq!(
            taken(&log),
            ["A touch Started", "B touch Started", "B touch Ended", "A touch Ended"]
        );

        // Each drags its own node at once, its moves its own.
        ui.touch(&frame, &mut text, &contact(1, TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &contact(2, TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &contact(1, TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&frame, &mut text, &contact(2, TouchPhase::Moved, 60.0, 50.0));
        ui.touch(&frame, &mut text, &contact(1, TouchPhase::Ended, 60.0, 10.0));
        ui.touch(&frame, &mut text, &contact(2, TouchPhase::Ended, 60.0, 50.0));
        assert_eq!(
            taken(&log),
            [
                "A touch Started",
                "B touch Started",
                "A touch Moved",
                "A Start",
                "A Move",
                "B touch Moved",
                "B Start",
                "B Move",
                "A End",
                "A touch Ended",
                "B End",
                "B touch Ended"
            ]
        );

        // After both lifted, a single finger taps again.
        ui.touch(&frame, &mut text, &contact(3, TouchPhase::Started, 10.0, 10.0));
        ui.touch(&frame, &mut text, &contact(3, TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(
            taken(&log),
            [
                "A touch Started",
                "A mouse down",
                "A Start",
                "A End",
                "A click",
                "A touch Ended"
            ]
        );
    }

    #[test]
    fn a_node_removed_and_made_again_mid_contact_cancels_only_its_pointer_once_for_good() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let both = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);
        // A goes (B moves up into its place), then comes back with the same key.
        let without = laid_out(&column(&log, &[(2, "B", b)]), &ui, &mut text);
        let again = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);
        let path_a = path_at(&both, 10.0, 10.0);

        // The mouse holds A and a finger drags B; A goes while both are
        // down: the mouse's drag is cancelled once, the finger's goes on.
        ui.mouse_down(&both, &mut text, 10.0, 10.0);
        ui.touch(&both, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&both, &mut text, &finger(TouchPhase::Moved, 60.0, 50.0));
        ui.touch(&without, &mut text, &finger(TouchPhase::Moved, 70.0, 10.0));
        assert!(!ui.state().active.contains(&path_a), "a node gone is pressed no more");
        // A made again with the same key is another node: the mouse does
        // not press it again, drag it, or click it on release.
        ui.touch(&again, &mut text, &finger(TouchPhase::Moved, 80.0, 50.0));
        ui.rehover(&again, &mut text);
        assert!(
            !ui.state().active.contains(&path_a),
            "made again, it is not pressed by the press it lost"
        );
        ui.touch(&again, &mut text, &finger(TouchPhase::Ended, 80.0, 50.0));
        ui.pointer_moved(&again, &mut text, 20.0, 10.0);
        ui.mouse_up(&again, 20.0, 10.0);
        assert_eq!(
            taken(&log),
            [
                "A mouse down",
                "A Start",
                "B touch Started",
                "B touch Moved",
                "B Start",
                "B Move",
                "A Cancel",
                "B touch Moved",
                "B Move",
                "B touch Moved",
                "B Move",
                "B End",
                "B touch Ended"
            ]
        );
        assert!(ui.state().active.is_empty());

        // The same with the finger's: B goes and comes back while the
        // mouse holds A; only the finger's raw touch and drag are cancelled.
        let without_b = laid_out(&column(&log, &[(1, "A", a)]), &ui, &mut text);
        ui.mouse_down(&both, &mut text, 10.0, 10.0);
        ui.touch(&both, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&both, &mut text, &finger(TouchPhase::Moved, 60.0, 50.0));
        ui.rehover(&without_b, &mut text);
        ui.rehover(&both, &mut text);
        ui.touch(&both, &mut text, &finger(TouchPhase::Moved, 70.0, 50.0));
        ui.touch(&both, &mut text, &finger(TouchPhase::Ended, 70.0, 50.0));
        ui.mouse_up(&both, 10.0, 10.0);
        assert_eq!(
            taken(&log),
            [
                "A mouse down",
                "A Start",
                "B touch Started",
                "B touch Moved",
                "B Start",
                "B Move",
                "B Cancel",
                "B touch Cancelled",
                "A End",
                "A click"
            ]
        );
    }

    /// A box taking drags and touches, 40 tall, `before` px down, logging
    /// its drag phases with the top of the box they carry.
    fn moving_box(log: &Log, before: f32) -> crate::element::Element {
        let log = log.clone();
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(crate::element::div().width(100.0).height(before))
            .child(
                crate::element::div()
                    .key(1)
                    .on_drag(move |e| log.borrow_mut().push(format!("{:?} {:.0}", e.phase, e.bounds.y)))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_touch_cancel_carries_the_box_its_node_has_now() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let top = laid_out(&moving_box(&log, 0.0), &ui, &mut text);
        let lower = laid_out(&moving_box(&log, 30.0), &ui, &mut text);

        ui.touch(&top, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&top, &mut text, &finger(TouchPhase::Moved, 10.0, 30.0));
        // The box moved under the resting finger (no move of it), then the
        // system took the contact away.
        ui.touch(&lower, &mut text, &finger(TouchPhase::Cancelled, 10.0, 30.0));
        assert_eq!(taken(&log), ["Start 0", "Move 0", "Cancel 30"]);

        // A drag cancelled by its node going reports where it last was.
        let gone = laid_out(
            &crate::element::div().width(400.0).height(100.0).into_element(),
            &ui,
            &mut text,
        );
        ui.touch(&top, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&top, &mut text, &finger(TouchPhase::Moved, 10.0, 30.0));
        ui.rehover(&lower, &mut text);
        ui.rehover(&gone, &mut text);
        ui.touch(&gone, &mut text, &finger(TouchPhase::Ended, 10.0, 30.0));
        assert_eq!(taken(&log), ["Start 0", "Move 0", "Move 30", "Cancel 30"]);
    }

    #[test]
    fn losing_window_focus_cancels_every_pointer_once() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let (a, b) = (crate::focus::FocusHandle::new(), crate::focus::FocusHandle::new());
        let frame = laid_out(&column(&log, &[(1, "A", a), (2, "B", b)]), &ui, &mut text);

        ui.mouse_down(&frame, &mut text, 10.0, 10.0);
        ui.pointer_moved(&frame, &mut text, 20.0, 10.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Started, 10.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 60.0, 50.0));
        assert!(!ui.state().active.is_empty());
        let out = ui.blur(&frame);
        assert!(out.redraw);
        assert!(ui.state().active.is_empty(), "nothing shows pressed");
        // What comes after the loss of capture does nothing.
        ui.pointer_moved(&frame, &mut text, 30.0, 10.0);
        ui.mouse_up(&frame, 30.0, 10.0);
        ui.touch(&frame, &mut text, &finger(TouchPhase::Moved, 70.0, 50.0));
        ui.touch(&frame, &mut text, &finger(TouchPhase::Ended, 70.0, 50.0));
        assert!(!ui.blur(&frame).redraw, "a second loss has nothing to cancel");
        assert_eq!(
            taken(&log),
            [
                "A mouse down",
                "A Start",
                "A Move",
                "B touch Started",
                "B touch Moved",
                "B Start",
                "B Move",
                "A Cancel",
                "B Cancel",
                "B touch Cancelled"
            ]
        );
    }

    /// A box at 0..100 by 0..40 taking drags and touches, unavailable or not.
    fn held_box(log: &Log, unavailable: bool) -> crate::element::Element {
        let (drag, touch) = (log.clone(), log.clone());
        crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .unavailable(unavailable)
                    .on_drag(move |e| drag.borrow_mut().push(format!("drag {:?}", e.phase)))
                    .on_touch(move |e| touch.borrow_mut().push(format!("touch {:?}", e.phase)))
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
    }

    #[test]
    fn a_gesture_whose_node_is_made_unavailable_mid_gesture_is_cancelled_once() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let available = laid_out(&held_box(&log, false), &ui, &mut text);
        let unavailable = laid_out(&held_box(&log, true), &ui, &mut text);

        ui.mouse_down(&available, &mut text, 10.0, 10.0);
        ui.pointer_moved(&available, &mut text, 20.0, 10.0);
        ui.pointer_moved(&unavailable, &mut text, 30.0, 10.0);
        ui.pointer_moved(&unavailable, &mut text, 40.0, 10.0);
        ui.mouse_up(&unavailable, 40.0, 10.0);
        assert_eq!(taken(&log), ["drag Start", "drag Move", "drag Cancel"], "by the mouse");

        ui.touch(&available, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.touch(&available, &mut text, &finger(TouchPhase::Moved, 60.0, 10.0));
        ui.touch(&unavailable, &mut text, &finger(TouchPhase::Moved, 70.0, 10.0));
        ui.touch(&unavailable, &mut text, &finger(TouchPhase::Moved, 80.0, 10.0));
        ui.touch(&unavailable, &mut text, &finger(TouchPhase::Ended, 80.0, 10.0));
        assert_eq!(
            taken(&log),
            [
                "touch Started",
                "touch Moved",
                "drag Start",
                "drag Move",
                "drag Cancel",
                "touch Cancelled"
            ],
            "by a finger"
        );

        // Settling a pass that made it unavailable cancels it then, not at
        // the next move.
        ui.touch(&available, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        ui.cancel_lost(&unavailable);
        ui.touch(&unavailable, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["touch Started", "touch Cancelled"]);
    }

    /// A box at 0..100 by 0..40 with a long press, as "A"; below it at
    /// 40..80 an item picked on release, or, with `modal`, a modal layer
    /// over the window whose item at 0..100 by 0..40 is picked on release.
    fn long_pressed(log: &Log, modal: bool) -> crate::element::Element {
        let item = |who: &'static str| {
            let log = log.clone();
            crate::element::div()
                .on_release(move |_| log.borrow_mut().push(format!("{who} release")))
                .width(100.0)
                .height(40.0)
        };
        let root = crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(logging(crate::element::div(), "A", log).width(100.0).height(40.0));
        if modal {
            root.child(crate::element::deferred(
                crate::element::Positioning::At { x: 0.0, y: 0.0 },
                crate::element::div()
                    .layer(crate::layer::Layer::new().modal(true))
                    .width(400.0)
                    .height(100.0)
                    .child(item("modal item")),
            ))
        } else {
            root.child(item("item"))
        }
        .into_element()
    }

    #[test]
    fn a_long_press_that_ran_nothing_does_not_release_where_it_lifts() {
        let log = Log::default();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let plain = laid_out(&long_pressed(&log, false), &ui, &mut text);
        let modal = laid_out(&long_pressed(&log, true), &ui, &mut text);
        let later = || {
            let t = crate::clock::current_seconds() + crate::gesture::LONG_PRESS.as_secs_f32() + 0.1;
            crate::clock::advance_to(std::time::Duration::from_secs_f32(t));
        };

        // A long press that ran: its lift over an item picks it.
        ui.touch(&plain, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        later();
        ui.long_presses(&plain);
        ui.touch(&plain, &mut text, &finger(TouchPhase::Ended, 10.0, 60.0));
        assert_eq!(
            taken(&log),
            ["A touch Started", "A long press", "item release", "A touch Ended"]
        );

        // A modal layer opened over it meanwhile: the long press runs
        // nothing below it, so its lift over the modal's item picks nothing.
        let over = hit(&modal, 10.0, 10.0).and_then(|i| modal.behaviour(i));
        assert!(
            over.is_some_and(|b| b.handlers.release.is_some()),
            "the modal's item is under it"
        );
        ui.touch(&plain, &mut text, &finger(TouchPhase::Started, 10.0, 10.0));
        later();
        ui.take_in(&modal);
        ui.long_presses(&modal);
        ui.touch(&modal, &mut text, &finger(TouchPhase::Ended, 10.0, 10.0));
        assert_eq!(taken(&log), ["A touch Started", "A touch Ended"]);
    }

    #[test]
    fn keys_move_and_delete_by_graphemes_and_words_as_laid_out() {
        let field = TextField::new("hello wide world e\u{301}");
        let root = text_input(&field).width(300.0).into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        field.focus.focus();
        let mut press = |k: NamedKey, word: bool, shift: bool| {
            ui.sync_focus(&crate::layout::LayoutTree::default());
            let frame = layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                &mut text,
            );
            ui.key(&frame, &mut text, &named(k, word, shift));
            field.edit.get()
        };
        // Backspace takes the accent alone, as browsers do.
        assert_eq!(press(NamedKey::Backspace, false, false).text(), "hello wide world e");
        assert_eq!(press(NamedKey::ArrowLeft, true, false).caret, 17);
        assert_eq!(press(NamedKey::ArrowLeft, true, false).caret, 11);
        let e = press(NamedKey::ArrowLeft, true, true);
        assert_eq!(e.selected_text(), "wide ");
        assert_eq!(
            press(NamedKey::ArrowRight, false, false).caret,
            11,
            "an arrow collapses a selection"
        );
        assert_eq!(press(NamedKey::Backspace, true, false).text(), "hello world e");
        assert_eq!(press(NamedKey::Home, false, false).caret, 0);
        assert_eq!(press(NamedKey::End, false, false).caret, "hello world e".len());
    }

    #[test]
    fn a_password_shows_bullets_everywhere_and_cannot_be_cut() {
        let field = TextField::new("");
        let root = text_input(&field)
            .input_type(InputType::Password)
            .width(200.0)
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        field.focus.focus();
        for c in ["s", "3", "c"] {
            ui.sync_focus(&crate::layout::LayoutTree::default());
            let frame = layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                &mut text,
            );
            ui.key(&frame, &mut text, &key(c, false));
        }
        ui.sync_focus(&crate::layout::LayoutTree::default());
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        ui.key(&frame, &mut text, &key("a", true));
        ui.key(&frame, &mut text, &key("x", true));
        assert_eq!(field.text(), "s3c", "a cut takes nothing out of a password");
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let dump = crate::inspect::dump(&frame.nodes);
        assert!(dump.contains("•••") && !dump.contains("s3c"), "{dump}");
    }

    #[test]
    fn an_input_method_composes_at_the_caret_and_types_only_what_it_commits() {
        let (field, other) = (TextField::new("xin chào "), TextField::new(""));
        let root = crate::element::div()
            .child(text_input(&field).width(200.0))
            .child(text_input(&other).width(200.0))
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        let frame = |ui: &mut InputState, text: &mut TextSystem| {
            ui.sync_focus(&crate::layout::LayoutTree::default());
            layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                text,
            )
        };
        let preedit = |s: &str| {
            ImeEvent::Preedit(crate::edit::Preedit {
                text: s.to_owned(),
                cursor: Some((s.len(), s.len())),
            })
        };
        field.focus.focus();
        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("viet"));
        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("việt"));
        assert_eq!(field.text(), "xin chào ", "a composition is not yet the value");
        let f = frame(&mut ui, &mut text);
        let dump = crate::inspect::dump(&f.nodes);
        assert!(dump.contains("xin chào việt"), "{dump}");
        let target = ui.ime_target(&f, &mut text).expect("a target");
        assert_eq!(target.field, field.focus.id());
        // The area holds the composition and about three characters either
        // side, inside the input (Parley's `PlainEditor::ime_cursor_area`).
        let i = focused_node(&f, field.focus.id()).expect("the input");
        let (_, t, input, ..) = InputState::input_geometry(&f, i).expect("an input");
        let before = text.measure("xin chào ", &t, None).0;
        let whole = text.measure("xin chào việt", &t, None).0;
        let inflate = 3.0 * 0.6 * t.size;
        assert!((target.area.x - (input.x + before - inflate)).abs() < 1.0, "{target:?}");
        assert!(
            (target.area.right() - (input.x + whole + inflate).min(input.right())).abs() < 1.0,
            "{target:?}"
        );
        ui.ime(&f, &ImeEvent::Commit("việt".to_owned()));
        assert_eq!(field.text(), "xin chào việt");

        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("nam"));
        other.focus.focus();
        let f = frame(&mut ui, &mut text);
        assert_eq!(
            ui.ime_target(&f, &mut text).map(|t| t.field),
            Some(other.focus.id()),
            "the input method follows focus"
        );
        assert_eq!(
            field.edit.with(|e| e.preedit().cloned()),
            None,
            "focus leaving drops the composition"
        );
        assert_eq!(field.text(), "xin chào việt");
    }

    /// The focus rules settle a frame once (by its epoch and change count):
    /// a trim between two settles, which `lay_out` leaves when its last
    /// pass settles nothing, gives the log the length it had at the last
    /// settle, yet the frame changed, and the rules still apply to it.
    #[test]
    fn a_trim_between_settles_hides_no_change_from_the_focus_rules() {
        use crate::element::{Positioning, deferred, div};
        use crate::focus::{FocusHandle, FocusScope};
        use crate::layer::Layer;

        let (held, first) = (FocusHandle::new(), FocusHandle::new());
        // Many boxes on the page that move every pass (an animation), and a
        // trapped layer: `held` is in it until `moved` takes it to the page.
        let build = |step: u16, moved: bool| {
            let stop = |on: bool| {
                let d = div().width(50.0).height(20.0);
                if on { d.track_focus(held) } else { d }
            };
            let boxes = (0..60).map(|_| div().width(10.0 + f32::from(step % 7)).height(4.0));
            let region = div()
                .layer(Layer::new().focus_scope(FocusScope::trapped()))
                .child(div().track_focus(first).width(50.0).height(20.0))
                .child(stop(!moved));
            div()
                .child(div().children(boxes).child(stop(moved)))
                .child(deferred(Positioning::At { x: 200.0, y: 100.0 }, region))
                .into_element()
        };
        let mut tree = crate::layout::LayoutTree::new();
        let mut text = TextSystem::new();
        let mut motion = crate::motion::Motion::default();
        let mut ui = InputState::default();
        let mut step = 0;
        let mut pass = |tree: &mut crate::layout::LayoutTree, ui: &InputState, moved: bool| {
            step += 1;
            let f = tree.layout(&build(step, moved), 400.0, 300.0, ui.state(), &mut motion, &mut text);
            (f.epoch(), f.changes.end(), f.changes.log.len())
        };
        pass(&mut tree, &ui, false);
        held.focus();
        ui.settle(&tree);
        assert_eq!(focus::focused(), Some(held.id()), "held inside the trapped layer");
        // Settle after each pass until one trims: the last settle saw the
        // log as a trim leaves it.
        let mut last = pass(&mut tree, &ui, false);
        ui.settle(&tree);
        loop {
            let now = pass(&mut tree, &ui, false);
            ui.settle(&tree);
            assert_eq!(now.0, last.0, "the boxes move in place");
            let trimmed = now.1.saturating_sub(now.2) > last.1.saturating_sub(last.2);
            last = now;
            if trimmed {
                break;
            }
        }
        // Two passes, no settle between: the second moves `held` out of the
        // layer, in place, and trims the log back to the same length.
        pass(&mut tree, &ui, false);
        let now = pass(&mut tree, &ui, true);
        assert_eq!((now.0, now.2), (last.0, last.2), "epoch and log length repeat");
        assert!(now.1 > last.1, "the change count does not");
        ui.settle(&tree);
        assert_eq!(
            focus::focused(),
            Some(first.id()),
            "focus outside the trapped layer is pulled back in"
        );
    }

    #[test]
    fn an_input_methods_area_under_a_turned_box_is_where_its_caret_is_drawn() {
        // A field in a box turned a quarter about its centre: the area an
        // input method is told is where the caret shows on screen, mapped
        // as everything is: hit testing its centre finds the caret.
        let field = TextField::new("hello");
        let root = crate::element::div()
            .width(400.0)
            .height(100.0)
            .child(
                crate::element::div()
                    .width(200.0)
                    .height(40.0)
                    .rotate(90.0)
                    .child(text_input(&field).width(120.0)),
            )
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = InputState::default();
        field.focus.focus();
        ui.sync_focus(&crate::layout::LayoutTree::default());
        let f = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        ui.take_in(&f);
        let i = focused_node(&f, field.focus.id()).expect("the input");
        let target = ui.ime_target(&f, &mut text).expect("a target");
        let (content, t, area, caret, _, multiline) = InputState::input_geometry(&f, i).expect("an input");
        let it = input_text(&mut text, &content, &t, area, multiline, true, (0.0, 0.0));
        let c = text.caret(&content, &it.t, it.width, caret);
        let (cx, cy) = (it.origin.0 + c.x, it.origin.1 + c.y + c.height / 2.0);
        let mid = (
            target.area.x + target.area.width / 2.0,
            target.area.y + target.area.height / 2.0,
        );
        let (hx, hy) = ui.shown(&f).to_node(i, mid);
        assert!(
            (hy - cy).abs() < 1.5 && hx >= area.x - 1.0 && hx <= cx + 1.0,
            "the area {target:?} maps back to ({hx}, {hy}), the caret at ({cx}, {cy})"
        );
        assert!(target.area.height > target.area.width, "turned: {target:?}");
    }
}
