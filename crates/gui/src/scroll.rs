//! A scroll container whose offset code holds: [`ScrollHandle`] says where the
//! container and its children were laid out and how far it scrolls, moves it,
//! and runs a hook after each layout that may ask for the frame to be laid out
//! again (the DOM's `scrollTop`, `getBoundingClientRect` and `ResizeObserver`
//! in one, as GPUI's `ScrollHandle`). A container without one keeps its offset
//! in the frame's interaction state, moved only by the wheel.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::element::ElementKey;
use crate::engine::{ForkSlab, SlabId};
use crate::geometry::Bounds;
use crate::style::Edges;

/// A scroll container as last laid out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollLayout {
    /// Where the container sits in the tree.
    pub path: crate::geometry::NodePath,
    /// The border box in window pixels.
    pub bounds: Bounds,
    pub padding: Edges<f32>,
    /// The offset shown, within `0..=max` on each axis.
    pub offset: (f32, f32),
    /// The largest offset on each axis.
    pub max: (f32, f32),
    /// The children, in order.
    pub children: Vec<ScrollChild>,
    /// The offset exactly ([`crate::compositor::motion`]).
    pub(crate) pos: Exact,
    /// The largest offset exactly.
    pub(crate) end: Exact,
}

use crate::compositor::motion::{self, Exact};

/// A child of a scroll container.
#[derive(Clone, Debug, PartialEq)]
pub struct ScrollChild {
    pub key: Option<ElementKey>,
    /// Its border box from the container's top-left corner, unscrolled.
    pub bounds: Bounds,
    /// The focused node is it or inside it.
    pub focus_within: bool,
}

/// What the render built the container's children from, as it handed it
/// ([`crate::Div::track_scroll_built`]).
pub(crate) type Built = Rc<dyn std::any::Any>;
type Hook = Rc<dyn Fn(&ScrollLayout, Option<&Built>) -> bool>;
type Reveal = Rc<dyn Fn(ElementKey) -> bool>;
type Take = Rc<dyn Fn(&Built) -> bool>;
type Wheeled = Rc<dyn Fn()>;

/// The two kinds of UI write to a scroll offset (ADR-0032). Every write
/// is an assignment in program order: the last one made wins, and none is
/// refused. The writer says which it makes: where a write comes from (a
/// layout hook, code between layouts) does not tell, since a hook may
/// flush a program's pending `scroll_to` and code between layouts may
/// splice rows under the place shown.
///
/// The compositor's scroll is a displacement the UI takes in before it
/// handles any input that came after it (ADR-0032, I2): what it took is in
/// the offset a write starts from, and what it had not taken yet lands on
/// top of the place written, as Chromium's main thread's scroll offset is
/// a base its compositor's delta rides on (`cc/trees/property_tree.cc`,
/// `PushScrollUpdatesFromMainThread`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Move {
    /// The UI chose a new place to show (a program's `scroll_to`, a text
    /// field's caret brought into view): a new generation, which tells the
    /// compositor to stop what it moves by itself there (a fling, a
    /// prediction past the last sample), as Chromium's clobbering commit
    /// does (`commit_state.h`, `scrollers_clobbering_active_value`); its
    /// reach goes. What the reader's own input moved is never dropped.
    Jump,
    /// The UI moved the content under the same place shown (a virtual
    /// list building another window of rows, a clamp, a list held at its
    /// end, the UI's own wheel): the offset changes by what moved, nothing
    /// else, and a gesture and its fling go on (as Chrome's scroll
    /// anchoring adjustments do not stop a fling).
    Rebase,
}

/// A scroll handle's state, in [`HANDLES`].
struct State {
    /// Bumped when a layout differs from the last: what a render read
    /// through [`ScrollHandle::layout`] renders again.
    version: crate::Signal<u64>,
    /// Bumped when the offset is set other than by a layout: the layout
    /// that placed the container reads it, so a wheel or `scroll_to`
    /// places that container again and nothing else (ADR-0013).
    moved: crate::Signal<u64>,
    /// Where the next layout puts the container, clamped then, exactly
    /// ([`motion`]): every motion adds to it in whole units.
    pos: Exact,
    /// Held at its end on each axis ([`ScrollHandle::keep_at`]): each
    /// layout puts it at that layout's end, until it is moved.
    end: (bool, bool),
    /// The container as last laid out, its children with it: replaced
    /// whole by a layout that differs (in place while live holds it alone),
    /// never written in part, so a check's copy shares it.
    laid: Option<Rc<ScrollLayout>>,
    /// The reader scrolled it since the last [`ScrollHandle::take_wheeled`]
    /// (moved or held against an edge).
    wheeled: bool,
    /// How many times the reader scrolled it (its wheel, the UI's or the
    /// compositor's), never reset: what shows an overlay scrollbar.
    wheels: u64,
    /// What was added to the offset ([`ScrollHandle::add`]: the reader's
    /// scroll) since it was last put or taken ([`ScrollHandle::take_added`]):
    /// a virtual list holding a jump not yet shown lays it on top of the
    /// jump's anchor (ADR-0032 I5), not over it.
    added: Exact,
    /// Bumped by each jump ([`Move::Jump`]): a commit with another than
    /// the last tells the compositor to stop what it moves by itself there
    /// (ADR-0032, I4). A count, compared only with itself.
    generation: u64,
    hook: Option<Hook>,
    /// Takes in what the render built the children from, after each layout
    /// pass that laid them out ([`ScrollHandle::on_built`]).
    take: Option<Take>,
    /// Brings the child keyed so into view (a virtual list's row, built or not).
    reveal: Option<Reveal>,
    /// Told each time the reader scrolls it, moved or not
    /// ([`ScrollHandle::on_wheel`]).
    on_wheel: Option<Wheeled>,
    /// How far the compositor may scroll it, at its speed, before the
    /// commit after this frame's shows (ADR-0015): what a virtual list
    /// builds beyond its viewport that way.
    reach: (f32, f32),
}

// A logged value (ADR-0034 S7, audited): `&State` changes nothing; every
// callback is called, never replaced, through it.
crate::engine::fork_value_audited!(State);
impl crate::engine::parts_seal::Sealed for State {}
impl crate::engine::Parts for State {
    /// Each callback and the layout on its own box (S9, review r17): two
    /// callbacks whose captures panic as they drop are two faults, never
    /// one unwind into another. Shared ones (live holds them too) are let
    /// go of at once.
    /// Every field is named (the drain lint): one added states how it drops.
    fn parts(self, out: &mut Vec<Box<dyn std::any::Any>>) {
        let Self {
            version,
            moved,
            pos,
            end,
            laid,
            wheeled,
            wheels,
            added,
            generation,
            hook,
            take,
            reveal,
            on_wheel,
            reach,
        } = self;
        laid.parts(out);
        hook.parts(out);
        take.parts(out);
        reveal.parts(out);
        on_wheel.parts(out);
        // Data: nothing to drop.
        let _data = (version, moved, pos, end, wheeled, wheels, added, generation, reach);
    }
}
impl crate::engine::sealed::Sealed for State {}
impl crate::engine::ForkClone for State {
    /// O(1): the last layout and its children are shared (a count bumped),
    /// and so is each callback, by identity (`Rc`: a count bumped, never a
    /// closure copied, so the copy holds no callback live does not); the
    /// rest is `Copy`. Every field is named, so one added states its cost.
    fn fork_clone(&self) -> Self {
        let Self {
            version,
            moved,
            pos,
            end,
            laid,
            wheeled,
            wheels,
            added,
            generation,
            hook,
            take,
            reveal,
            on_wheel,
            reach,
        } = self;
        Self {
            version: *version,
            moved: *moved,
            pos: *pos,
            end: *end,
            laid: laid.as_ref().map(Rc::clone),
            wheeled: *wheeled,
            wheels: *wheels,
            added: *added,
            generation: *generation,
            hook: hook.as_ref().map(Rc::clone),
            take: take.as_ref().map(Rc::clone),
            reveal: reveal.as_ref().map(Rc::clone),
            on_wheel: on_wheel.as_ref().map(Rc::clone),
            reach: *reach,
        }
    }
}

crate::engine::engine_local! {
    /// Every scroll handle's state, by its id (ADR-0028, rule 4: a handle
    /// is an id): what a cold layout writes through a handle is the fork's.
    [Fork(log)] static HANDLES: RefCell<ForkSlab<State>> = RefCell::default();
}

/// Holds one scroll container's offset (see [`crate::Div::track_scroll`]):
/// a `Copy` handle, owned by the scope it is made in (made outside any,
/// it lives until [`Self::dispose`]). Once its owner ended it is inert: it
/// reads as never laid out, and writes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScrollHandle(SlabId);

impl Default for ScrollHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for ScrollHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollHandle")
            .field("offset", &self.read(|s| motion::px2(s.pos)))
            .finish_non_exhaustive()
    }
}

impl ScrollHandle {
    #[must_use]
    pub fn new() -> Self {
        let state = State {
            version: crate::reactive::detached(|| crate::Signal::labeled("scroll layout", 0)),
            moved: crate::reactive::detached(|| crate::Signal::labeled("scroll offset", 0)),
            pos: (0, 0),
            end: (false, false),
            laid: None,
            wheeled: false,
            wheels: 0,
            added: (0, 0),
            generation: 0,
            hook: None,
            take: None,
            reveal: None,
            on_wheel: None,
            reach: (0.0, 0.0),
        };
        let me = Self(crate::engine::allocate(&HANDLES, state));
        crate::engine::owned_by_scope(move || me.dispose());
        me
    }

    /// Let its state go now: it is inert from here on.
    pub fn dispose(self) {
        // `try_with`: a holder may drop it as the thread's state is torn
        // down, when the state is gone already.
        if let Ok(Some(s)) = HANDLES.try_with(|h| h.with_mut(|h| h.remove(self.0))) {
            s.version.dispose_quietly();
            s.moved.dispose_quietly();
        }
    }

    /// Whether its owner has not ended.
    #[must_use]
    pub fn is_alive(self) -> bool {
        HANDLES.with_borrow(|h| h.contains(self.0))
    }

    /// How many scroll handles this thread holds (tests: what a component
    /// made goes with it).
    #[doc(hidden)]
    #[must_use]
    pub fn live() -> usize {
        HANDLES.with_borrow(|h| h.len())
    }

    /// Start it afresh, as a handle never laid out: at the top, read as
    /// not laid out until its next layout (which a layer's placement
    /// scrolls as it opens): a jump ([`Move::Jump`]). What one handle,
    /// kept by its owner, does in place of a new one made for each opening.
    ///
    /// Reset: the offset (to the top), the ends reached, the last layout,
    /// whether the reader scrolled it, the reach the compositor may scroll
    /// (none), and the generation (a new one). Kept: its hooks (the layout
    /// hook and [`Self::on_wheel`]), a take or reveal asked for and not yet
    /// laid out, and the scrolls counted ([`Self::wheels`]), which never go
    /// back. Both signals it bumps are told.
    #[track_caller]
    pub fn reset(&self) {
        self.used();
        let signals = self.write(|s| {
            Self::jumped(s);
            s.pos = (0, 0);
            s.end = (false, false);
            s.laid = None;
            s.wheeled = false;
            (s.version, s.moved)
        });
        if let Some((version, moved)) = signals {
            version.update(|v| *v = v.wrapping_add(1));
            moved.update(|m| *m = m.wrapping_add(1));
        }
    }

    /// Read its state; `None` once it is gone. `f` reaches no handle.
    fn read<R>(self, f: impl FnOnce(&State) -> R) -> Option<R> {
        HANDLES.with_borrow(|h| h.read(self.0, f))
    }

    /// Write its state (nothing once it is gone). `f` reaches no handle.
    fn write<R>(self, f: impl FnOnce(&mut State) -> R) -> Option<R> {
        HANDLES.with_borrow(|h| h.write(self.0, f))
    }

    /// Its owner's code uses it: gone, that is a bug, loud in a debug build
    /// ([`crate::engine::check_alive`]). Every `pub` method but `dispose`
    /// and `is_alive` asks this; the engine's own (`pub(crate)`) do not.
    #[track_caller]
    #[inline]
    fn used(self) {
        #[cfg(debug_assertions)]
        crate::engine::check_alive(self.is_alive(), "scroll handle");
    }

    /// The offset shown, or asked for since the last layout.
    #[must_use]
    #[track_caller]
    pub fn offset(&self) -> (f32, f32) {
        self.used();
        let Some((version, pos)) = self.read(|s| (s.version, s.pos)) else {
            return (0.0, 0.0);
        };
        version.with(|_| ());
        motion::px2(pos)
    }

    /// Scroll to `offset`; the next layout clamps it to what the content
    /// allows. Always a jump ([`Move::Jump`]), even to the offset this
    /// side holds: the compositor may show it elsewhere (a fling it runs),
    /// and a program asking for a place gets that place.
    #[track_caller]
    pub fn scroll_to(&self, offset: (f32, f32)) {
        self.used();
        self.put(offset, Move::Jump);
    }

    /// Put it at `offset` (`f32::MAX` on an axis: its end) as `kind` says,
    /// as a virtual list places its window: its offset is the window's, so
    /// an offset equal to the one it has may be another place (two rows
    /// far apart at the same height in their windows), and a
    /// [`Move::Jump`] there is a new generation all the same. Held at its
    /// end already, it stays at the end of whatever layout places it next
    /// (decided by the next layout, not the last one this reads: content
    /// added since has moved the end, and the end it is held at moves
    /// with it); a [`Move::Rebase`] there is no move at all.
    pub(crate) fn keep_at(&self, offset: (f32, f32), kind: Move) {
        let Some((at, end)) = self.read(|s| (s.pos, s.laid.as_ref().map(|l| l.end))) else {
            return;
        };
        // Exactly ([`motion`]): deep in a long list an `f32` of pixels
        // tells no two nearby places apart.
        let held = |want: f32, at: i64, end: Option<i64>| {
            motion::units(want) == at || (want == f32::MAX && end.is_some_and(|e| at >= e))
        };
        if !(held(offset.0, at.0, end.map(|e| e.0)) && held(offset.1, at.1, end.map(|e| e.1))) {
            self.put(offset, kind);
            return;
        }
        // Held: an axis held at its end is placed at each layout's end
        // ([`Self::wanted`]) until something moves it.
        self.write(|s| {
            if kind == Move::Jump {
                Self::jumped(s);
            }
            s.end = (offset.0 == f32::MAX, offset.1 == f32::MAX);
        });
        if kind == Move::Jump {
            // The commit carries the new generation.
            self.moved();
        }
    }

    /// The one way the UI side writes the offset, as a jump or a rebase
    /// ([`Move`]), an assignment in program order (ADR-0032, I5): a
    /// rebase to where it is writes nothing. A place written in pixels
    /// lands on the grid ([`motion::units`]): a jump to 0.1 px is a whole
    /// number of units, and motion from there is exact. Whether it wrote.
    fn put(&self, offset: (f32, f32), kind: Move) -> bool {
        let pos = motion::units2(offset);
        let wrote = self.write(|s| {
            if s.pos == pos && kind == Move::Rebase {
                return false;
            }
            if kind == Move::Jump {
                Self::jumped(s);
            }
            s.pos = pos;
            s.end = (false, false);
            s.added = (0, 0);
            true
        });
        if wrote != Some(true) {
            return false;
        }
        self.moved();
        true
    }

    /// A jump: a new generation, and how far the compositor may scroll it,
    /// measured from where it was, goes.
    fn jumped(s: &mut State) {
        s.reach = (0.0, 0.0);
        s.generation = s.generation.wrapping_add(1);
        s.added = (0, 0);
    }

    /// What the reader's scroll added since the offset was last put or
    /// this was last called, cleared ([`State::added`]).
    pub(crate) fn take_added(&self) -> (f32, f32) {
        if self.read(|s| s.added).is_none_or(|a| a == (0, 0)) {
            return (0.0, 0.0);
        }
        motion::px2(self.write(|s| std::mem::take(&mut s.added)).unwrap_or_default())
    }

    /// Where the UI holds it now, exactly, as a write starts from: its end
    /// when held there (the range last laid out).
    fn pos_now(s: &State) -> Exact {
        let end = s.laid.as_ref().map_or((i64::MAX, i64::MAX), |l| l.end);
        (
            if s.end.0 { end.0 } else { s.pos.0 },
            if s.end.1 { end.1 } else { s.pos.1 },
        )
    }

    /// Where the UI holds it now, as a write starts from: its end when held
    /// there (the range last laid out), untracked.
    pub(crate) fn offset_now(&self) -> (f32, f32) {
        self.read(|s| motion::px2(Self::pos_now(s))).unwrap_or_default()
    }

    /// Move it by `delta` units ([`motion`]), the compositor's scroll the UI
    /// took in (ADR-0032, I2) or its own wheel's: added exactly to where
    /// the UI has it now (what it took before, a jump made since), clamped
    /// to the range last laid out; a container held at its end it moves
    /// off it. Whether it moved.
    pub(crate) fn add(&self, delta: Exact) -> bool {
        if delta == (0, 0) {
            return false;
        }
        let moved = self.write(|s| {
            let end = s.laid.as_ref().map_or((i64::MAX, i64::MAX), |l| l.end);
            let from = Self::pos_now(s);
            let to = motion::clamp2(motion::add2(from, delta), end);
            // What moved it, clamped first (Opus r9 nit): a push past an
            // edge adds nothing to what a list lays on its anchor.
            s.added = motion::add2(s.added, motion::sub2(to, from));
            if to == s.pos && s.end == (false, false) {
                return false;
            }
            s.pos = to;
            s.end = (false, false);
            true
        });
        if moved == Some(true) {
            self.moved();
            return true;
        }
        false
    }

    /// The reader scrolled it (its wheel, the UI's or the compositor's),
    /// whether or not that moved it: a wheel held against its edge shows
    /// its overlay scrollbar as one that moves it does (Chrome's overlay
    /// scrollbars flash on every scroll the container takes). Counted
    /// apart from the offset written, so a wheel that writes nothing still
    /// counts, and its owner is told at once ([`Self::on_wheel`]), with no
    /// layout.
    fn wheeled(&self, steps: u32) {
        if steps == 0 {
            return;
        }
        let told = self
            .write(|s| {
                s.wheeled = true;
                s.wheels = s.wheels.wrapping_add(u64::from(steps));
                s.on_wheel.clone()
            })
            .flatten();
        if let Some(f) = told {
            f();
        }
    }

    /// Run `f` each time the reader scrolls it (its wheel, the UI's or the
    /// compositor's, a finger's pan), moved or held against its edge: what
    /// shows an overlay scrollbar of the owner's own. A wheel against its
    /// edge places nothing again, so this is how the owner hears of it.
    pub(crate) fn on_wheel(&self, f: impl Fn() + 'static) {
        let f: Wheeled = Rc::new(f);
        let _was = self.write(|s| s.on_wheel.replace(f));
    }

    fn moved(&self) {
        if let Some(moved) = self.read(|s| s.moved) {
            moved.update(|m| *m = m.wrapping_add(1));
        }
    }

    /// The offset the next layout puts it at, read by that layout: a set
    /// offset places the container again. Exactly ([`motion`]); an axis
    /// held at its end is `i64::MAX`, which the layout clamps to its end.
    pub(crate) fn wanted(&self) -> Exact {
        let Some((moved, at, end)) = self.read(|s| (s.moved, s.pos, s.end)) else {
            return (0, 0);
        };
        moved.with(|_| ());
        (if end.0 { i64::MAX } else { at.0 }, if end.1 { i64::MAX } else { at.1 })
    }

    /// How many times the offset was set other than by the compositor's scroll.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.read(|s| s.generation).unwrap_or(0)
    }

    /// The container as last laid out; `None` before its first layout.
    #[must_use]
    #[track_caller]
    pub fn layout(&self) -> Option<ScrollLayout> {
        self.used();
        let (version, laid) = self.read(|s| (s.version, s.laid.as_deref().cloned()))?;
        version.with(|_| ());
        laid
    }

    /// Whether the reader scrolled it with the wheel since the last call
    /// (moved or held against an edge).
    #[track_caller]
    pub fn take_wheeled(&self) -> bool {
        self.used();
        self.write(|s| std::mem::take(&mut s.wheeled)).unwrap_or(false)
    }

    /// How many times the reader has scrolled it: a change since last read
    /// means they scrolled it (an overlay scrollbar shows).
    #[must_use]
    #[track_caller]
    pub fn wheels(&self) -> u64 {
        self.used();
        self.read(|s| s.wheels).unwrap_or(0)
    }

    /// Run `f` after each layout of the container. It returns whether it
    /// changed what the frame is built from (a signal the view reads, the
    /// offset), and then the frame is laid out again before it is painted.
    #[track_caller]
    pub fn on_layout(&self, f: impl Fn(&ScrollLayout) -> bool + 'static) {
        self.used();
        // A caller's closure: reached only through the gate, so a fork
        // never runs it (ADR-0028, rule 5) and asks for no pass again.
        let hook: Hook = Rc::new(move |l: &ScrollLayout, _: Option<&Built>| {
            crate::engine::call_out(crate::engine::Site::ScrollLayout, || f(l)).unwrap_or(false)
        });
        // What it held is dropped once the state is let go.
        let _was = self.write(|s| s.hook.replace(hook));
    }

    /// [`Self::on_layout`], handed also what the render built the
    /// children from ([`crate::Div::track_scroll_built`]): the frame's
    /// container's, whether or not this pass laid it out again.
    pub(crate) fn on_layout_built(&self, f: impl Fn(&ScrollLayout, Option<&Built>) -> bool + 'static) {
        let hook: Hook = Rc::new(f);
        let _was = self.write(|s| s.hook.replace(hook));
    }

    /// Run `f` on what the render built the children from
    /// ([`crate::Div::track_scroll_built`]) after every layout pass that laid
    /// them out, the frame's last one too, before the hooks: what the frame
    /// shows is what its owner holds. The same `built` may come again. It
    /// returns whether it put the container elsewhere (the pass lays it out
    /// again at once, so the frame paints its rows where they are).
    pub(crate) fn on_built(&self, f: impl Fn(&Built) -> bool + 'static) {
        let take: Take = Rc::new(f);
        // What it held is dropped once the state is let go.
        let _was = self.write(|s| s.take.replace(take));
    }

    pub(crate) fn take_built(&self, built: &Built) -> bool {
        let Some(take) = self.read(|s| s.take.clone()).flatten() else {
            return false;
        };
        take(built)
    }

    /// Let `f` bring the child keyed `key` into view when assistive
    /// technology asks for it (`ScrollIntoView`): a container whose children
    /// are not all built (a virtual list) knows where one lies. It returns
    /// whether it scrolled.
    #[track_caller]
    pub fn on_reveal(&self, f: impl Fn(ElementKey) -> bool + 'static) {
        self.used();
        // A caller's closure, through the gate: a fork scrolls nothing.
        let reveal: Reveal =
            Rc::new(move |key| crate::engine::call_out(crate::engine::Site::Reveal, || f(key)).unwrap_or(false));
        let _was = self.write(|s| s.reveal.replace(reveal));
    }

    /// Bring the child keyed `key` into view through [`Self::on_reveal`];
    /// `None` when the container has no such hook.
    pub(crate) fn reveal(&self, key: ElementKey) -> Option<bool> {
        let f = self.read(|s| s.reveal.clone()).flatten();
        f.map(|f| f(key))
    }

    pub(crate) fn laid_out(&self, layout: ScrollLayout) {
        let changed = self.write(|s| {
            // Exactly where the layout put it: the clamp of an exact place,
            // never its pixels read back (a list deep down would round).
            s.pos = layout.pos;
            if s.laid.as_deref() == Some(&layout) {
                return None;
            }
            // In place while it is live's alone (no allocation); a new one
            // where a check's copy shares it, which is left as it was.
            match s.laid.as_mut().and_then(Rc::get_mut) {
                Some(laid) => *laid = layout,
                None => s.laid = Some(Rc::new(layout)),
            }
            Some(s.version)
        });
        if let Some(version) = changed.flatten() {
            version.update(|v| *v = v.wrapping_add(1));
        }
    }

    /// The UI's own wheel scrolled it by `delta`, added to where the UI has
    /// it now as the compositor's scroll taken in is ([`Self::add`]; held
    /// against its edge: counted all the same, nothing written, nothing
    /// placed again): the reader's scroll, not a jump, so what the
    /// compositor moves by itself goes on.
    pub(crate) fn wheel(&self, delta: Exact) {
        self.wheeled(1);
        self.add(delta);
    }

    /// The reader scrolled it by `steps` wheel steps, each told once (the
    /// UI's own wheel against its edge, one; the compositor's, as many as
    /// the UI took in, [`crate::compositor::ScrollDelta::steps`]),
    /// whether or not they moved it: counted, nothing written (a scroll
    /// asked since still lands).
    pub(crate) fn wheeled_by(&self, steps: u32) {
        // Nothing moved by this: nothing is placed again, no hook runs.
        self.wheeled(steps);
    }

    /// How far the compositor may scroll it before the commit after this
    /// frame's shows, signed as an offset moves (zero when it is still).
    #[must_use]
    #[track_caller]
    pub fn reach(&self) -> (f32, f32) {
        self.used();
        self.read(|s| s.reach).unwrap_or((0.0, 0.0))
    }

    pub(crate) fn set_reach(&self, reach: (f32, f32)) {
        self.write(|s| s.reach = reach);
    }

    /// Run the hook on the last layout; whether the frame must be laid out again.
    pub(crate) fn after_layout(&self, built: Option<&Built>) -> bool {
        // Cloned out, so the hook may set a new one.
        let Some((hook, laid)) = self.read(|s| (s.hook.clone(), s.laid.clone())) else {
            return false;
        };
        match (hook, laid) {
            (Some(f), Some(l)) => f(&l, built),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Scenario;
    use crate::prelude::*;
    use crate::{Signal, div};

    /// A list 100 high whose content is `rows` rows of 20, its offset held
    /// by `handle`.
    fn list(handle: &ScrollHandle, rows: Signal<u32>) -> Scenario {
        let handle = *handle;
        Scenario::measuring(
            move || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&handle)
                    .child(div().height(20.0 * f32::from(u16::try_from(rows.get()).unwrap_or(0))))
                    .into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario")
    }

    /// Opus r11: a frame runs each tracked handle's hook once, finding
    /// those it ran by a set, not by comparing each with every one before
    /// it (2.3 s a frame at 20,000 handles). 20,000 tracked lists, one
    /// handle shared by two of them: at most one comparison per list.
    #[test]
    fn a_frame_of_twenty_thousand_tracked_lists_runs_each_hook_once_in_linear_time() {
        const N: usize = 20_000;
        let handles: Vec<ScrollHandle> = (0..N).map(|_| ScrollHandle::new()).collect();
        let shared = handles[0];
        let page = {
            let handles = handles.clone();
            move || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .children(handles.iter().chain([&shared]).map(|h| {
                        div()
                            .width(4.0)
                            .height(0.01)
                            .flex_shrink(0.0)
                            .overflow_y(Overflow::Scroll)
                            .track_scroll(h)
                            .child(div().height(1.0))
                    }))
                    .into_element()
            }
        };
        let mut s = Scenario::measuring(page, 100, 100, 1.0).expect("a scenario");
        let before = crate::app::HANDLE_COMPARES.with(std::cell::Cell::get);
        let t = std::time::Instant::now();
        s.frame("first").expect("a frame");
        let took = t.elapsed();
        let compares = crate::app::HANDLE_COMPARES.with(std::cell::Cell::get) - before;
        eprintln!("{N} tracked handles: {compares} compared, frame {took:?}");
        assert!(
            compares <= 2 * (N + 1),
            "{compares} handle comparisons in a frame of {N}"
        );
        assert!(handles.iter().all(|h| h.layout().is_some()), "every list laid out");
        // A frame after one list scrolled: every hook runs again.
        let before = crate::app::HANDLE_COMPARES.with(std::cell::Cell::get);
        s.act("one scrolls", || handles[7].scroll_to((0.0, 0.5)));
        let t = std::time::Instant::now();
        s.frame("scrolled").expect("a frame");
        let took = t.elapsed();
        let compares = crate::app::HANDLE_COMPARES.with(std::cell::Cell::get) - before;
        eprintln!("then a frame after one scrolled: {compares} compared, {took:?}");
        assert!(
            compares <= 2 * (N + 1),
            "{compares} handle comparisons in a later frame"
        );
    }

    /// Opus r9 nit: scroll taken in past an edge adds to what the reader
    /// moved only what moved it ([`ScrollHandle::take_added`]): 10 rows
    /// of 20 in 100 reach 100; at 90, 30 more moves it 10.
    #[test]
    fn what_is_added_is_what_moved_it_within_its_range() {
        let (h, rows) = (ScrollHandle::new(), Signal::new(10_u32));
        let mut s = list(&h, rows);
        s.frame("first").expect("a frame");
        h.add(motion::units2((0.0, 90.0)));
        s.frame("at 90").expect("a frame");
        let _ = h.take_added();
        assert!(h.add(motion::units2((0.0, 30.0))));
        assert_eq!(h.offset_now(), (0.0, 100.0));
        assert_eq!(h.take_added(), (0.0, 10.0), "what moved it");
    }

    /// The program scrolls a list the compositor was scrolling: its reach
    /// (how far the compositor might scroll it, measured from where it
    /// was) goes at once, read in the same frame, before any layout.
    #[test]
    fn a_scroll_asked_for_drops_the_reach_at_once() {
        let h = ScrollHandle::new();
        h.set_reach((0.0, 300.0));
        h.scroll_to((0.0, 50.0));
        assert_eq!(h.reach(), (0.0, 0.0), "measured from where it no longer is");
        // A scroll to where its handle says it is is a jump all the same:
        // the compositor may have scrolled it on from there meanwhile.
        h.set_reach((0.0, 300.0));
        h.scroll_to((0.0, 50.0));
        assert_eq!(h.reach(), (0.0, 0.0), "a scroll to where it is jumps too");
    }

    /// A list held at its end, as one following its end is, when rows are
    /// added and it is held there again before the next layout (the last
    /// layout, which it reads, is stale): the next layout puts it at the
    /// new end, and that is no write (no new generation: what the
    /// compositor scrolled from there still lands).
    #[test]
    fn a_list_held_at_its_end_stays_there_as_rows_come() {
        let (h, rows) = (ScrollHandle::new(), Signal::new(10_u32));
        let mut s = list(&h, rows);
        s.frame("first").expect("a frame");
        h.scroll_to((0.0, f32::MAX));
        s.frame("at the end").expect("a frame");
        assert_eq!(h.offset(), (0.0, 100.0));
        let generation = h.generation();
        s.act("rows come", || {
            rows.set(20);
            h.keep_at((0.0, f32::MAX), Move::Rebase);
        });
        s.frame("grown").expect("a frame");
        assert_eq!(h.offset(), (0.0, 300.0), "at the new end");
        assert_eq!(h.generation(), generation, "and no write");
        // Moved by the program, it is held no more.
        s.act("away", || h.scroll_to((0.0, 40.0)));
        s.frame("away").expect("a frame");
        s.act("more rows", || rows.set(30));
        s.frame("more").expect("a frame");
        assert_eq!(h.offset(), (0.0, 40.0), "where the program put it");
    }

    /// A wheel gesture that took a list to its end goes on against it: the
    /// list moves no more, but the reader still scrolls it, so its overlay
    /// scrollbar shows ([`ScrollHandle::wheels`]), through the compositor
    /// that took the wheel and the main frame that tells the UI.
    #[test]
    fn a_wheel_against_the_end_counts_as_a_scroll() {
        let (h, rows) = (ScrollHandle::new(), Signal::new(10_u32));
        let mut s = list(&h, rows);
        s.hover(50.0, 50.0);
        s.frame("first").expect("a frame");
        assert!(s.wheel(500.0), "to its end");
        s.frame("scrolled").expect("a frame");
        assert_eq!(h.offset(), (0.0, 100.0));
        let wheels = h.wheels();
        let placed = || h.read(|s| s.moved.get_untracked());
        let before = placed();
        assert!(s.wheel(30.0), "the gesture's next turn, latched to it, is taken");
        s.frame("against the end").expect("a frame");
        assert_eq!(h.offset(), (0.0, 100.0), "nothing moved");
        assert!(h.wheels() > wheels, "but it was scrolled");
        assert_eq!(placed(), before, "and it was not placed again");
    }

    /// A list held at its end, wheeled against it (the compositor's wheel
    /// moved nothing, its main frame says only that it was wheeled): it is
    /// still held there, and rows that come keep it at the new end.
    #[test]
    fn a_wheel_against_a_held_end_keeps_it_held() {
        let (h, rows) = (ScrollHandle::new(), Signal::new(10_u32));
        let mut s = list(&h, rows);
        s.hover(50.0, 50.0);
        s.frame("first").expect("a frame");
        h.scroll_to((0.0, f32::MAX));
        s.frame("at the end").expect("a frame");
        assert_eq!(h.offset(), (0.0, 100.0));
        s.act("held there", || h.keep_at((0.0, f32::MAX), Move::Rebase));
        s.frame("held").expect("a frame");
        let wheels = h.wheels();
        assert!(s.wheel(30.0), "a wheel against the end is taken");
        s.frame("against the end").expect("a frame");
        assert!(h.wheels() > wheels, "it was scrolled");
        s.act("rows come", || rows.set(20));
        s.frame("grown").expect("a frame");
        assert_eq!(h.offset(), (0.0, 300.0), "still held at its end");
    }

    /// Two lists: the program jumps the first twice, the reader turns the
    /// wheel over the second, then the program, between inputs, jumps the
    /// second before the UI took the turn in. The UI takes the compositor's
    /// scroll in before each input and each frame (ADR-0032), not before
    /// its own work: the turn lands on top of the jump, from it, and the
    /// first list's jumps touch nothing of the second's.
    #[test]
    fn a_turn_not_yet_taken_lands_on_a_jump_made_between_inputs() {
        let (a, b) = (ScrollHandle::new(), ScrollHandle::new());
        let list = |h: ScrollHandle| {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(div().height(2_000.0))
        };
        let mut s = Scenario::measuring(
            move || {
                div()
                    .display(Display::Flex)
                    .child(list(a))
                    .child(list(b))
                    .into_element()
            },
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.hover(150.0, 50.0);
        s.frame("first").expect("a frame");
        s.act("jump the first twice", || {
            a.scroll_to((0.0, 100.0));
            a.scroll_to((0.0, 200.0));
        });
        assert!(s.wheel(40.0), "the turn over the second");
        s.act("jump the second", || b.scroll_to((0.0, 500.0)));
        s.frame("both come").expect("a frame");
        assert_eq!(b.offset(), (0.0, 540.0), "the turn lands on the jump");
        assert_eq!(a.offset(), (0.0, 200.0));
        s.frame("after").expect("a frame");
        assert_eq!(b.offset(), (0.0, 540.0), "once");
    }

    /// A list over 2,000 of content whose key handler jumps it to its top
    /// ("Top") and whose drag handler writes three places in a row.
    fn top_on_key(h: ScrollHandle) -> Scenario {
        Scenario::measuring(
            move || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&h)
                    .focusable()
                    .on_key_down(move |_| {
                        h.scroll_to((0.0, 0.0));
                        true
                    })
                    .on_drag(move |e| {
                        if e.phase == crate::events::DragPhase::Move {
                            h.scroll_to((0.0, 10.0));
                            h.scroll_to((0.0, 20.0));
                            h.scroll_to((0.0, 30.0));
                        }
                    })
                    .child(div().height(2_000.0))
                    .into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario")
    }

    fn arrow() -> crate::window::Input {
        crate::window::Input::Key(crate::events::KeyEvent {
            key: crate::events::Key::Named(crate::events::NamedKey::Home),
            text: None,
            modifiers: crate::events::Modifiers::default(),
        })
    }

    /// Three writes a handler makes in a row, under one input: the last
    /// one made stands (ADR-0032, I5: writes are assignments in program
    /// order, none refused; Codex r7 1).
    #[test]
    fn writes_one_handler_makes_end_at_the_last() {
        let h = ScrollHandle::new();
        let mut s = top_on_key(h);
        s.frame("first").expect("a frame");
        s.drag((50.0, 50.0), (50.0, 80.0));
        assert_eq!(h.offset(), (0.0, 30.0), "the last write");
        s.frame("laid").expect("a frame");
        assert_eq!(h.offset(), (0.0, 30.0));
    }

    /// The reader turns the wheel, then presses the key that jumps to the
    /// top, while the UI is stalled (it handles neither before both are
    /// sent, and the render thread applies the wheel at once), then turns
    /// it again before the UI gets to the key. The key is handled after
    /// what the turn before it moved, taken in first (I2), so it ends at
    /// the top whatever the stall; the turn after it moves from there.
    #[test]
    fn a_turn_before_a_key_is_taken_before_it_however_late() {
        let h = ScrollHandle::new();
        let mut s = top_on_key(h);
        s.click(50.0, 50.0);
        s.frame("focused").expect("a frame");
        assert!(s.wheel(300.0), "the turn before");
        let key = s.queue(arrow());
        assert!(s.wheel(40.0), "the turn after, applied before the UI gets to the key");
        // 500 ms of stall: the compositor draws, the UI does nothing.
        crate::clock::advance_to(
            std::time::Duration::from_nanos(crate::clock::current_nanos()) + std::time::Duration::from_millis(500),
        );
        s.handle(key);
        assert_eq!(
            h.offset(),
            (0.0, 0.0),
            "the key's place: the turn before it was taken first"
        );
        s.frame("after").expect("a frame");
        assert_eq!(h.offset(), (0.0, 40.0), "the turn after moves from the jump");
    }

    /// The key first, then the turn: the turn moves from the jump.
    #[test]
    fn a_turn_after_a_key_moves_from_its_jump() {
        let h = ScrollHandle::new();
        let mut s = top_on_key(h);
        s.click(50.0, 50.0);
        s.frame("focused").expect("a frame");
        assert!(s.wheel(300.0));
        s.frame("scrolled").expect("a frame");
        assert_eq!(h.offset(), (0.0, 300.0));
        s.send(arrow());
        assert!(s.wheel(40.0));
        s.frame("after").expect("a frame");
        assert_eq!(h.offset(), (0.0, 40.0), "from the top");
    }

    /// The UI thread's own loop ([`crate::window::run_ui_loop`]) handing
    /// what it takes to a scenario's pipeline, as the window's handler
    /// does, with the wheel number the loop hands on; `at_idle` runs, by
    /// the count of the loop's turns, what happens meanwhile (a program's
    /// jump, a wheel on the render thread).
    struct Through<'a> {
        s: &'a mut Scenario,
        idles: usize,
        at_idle: Vec<(usize, Meanwhile<'a>)>,
    }

    /// What happens meanwhile, at one of the loop's turns.
    type Meanwhile<'a> = Box<dyn FnOnce(&mut Scenario) + 'a>;

    impl crate::window::WindowHandler for Through<'_> {
        fn frame(
            &mut self,
            _: &mut dyn crate::compositor::Compose,
            _: crate::compositor::MainFrame,
            _: (u32, u32),
            _: f32,
        ) {
        }
        fn take_scroll(&mut self, pulled: crate::compositor::Pulled) -> bool {
            self.s.pipeline_mut().take_pulled(&pulled)
        }
        fn take_gesture(&mut self, gesture: crate::compositor::HandedGesture) {
            self.s.pipeline_mut().take_gesture(gesture);
        }
        fn input(&mut self, input: crate::window::Input, _: &crate::compositor::Shown) -> bool {
            let shown = self.s.shown();
            self.s.pipeline_mut().input_shown(input, &shown).redraw
        }
        fn cursor(&self) -> crate::style::Cursor {
            crate::style::Cursor::Default
        }
        fn ime_target(&mut self) -> Option<crate::interact::ImeTarget> {
            None
        }
        #[expect(clippy::unreachable, reason = "no screen reader connects in the test")]
        fn access_update(&mut self, _: f32, _: &str) -> accesskit::TreeUpdate {
            unreachable!("no screen reader connects")
        }
        fn access_reset(&mut self) {}
        fn idle(&mut self, _: &mut dyn crate::compositor::Compose) -> crate::window::Wake {
            self.idles = self.idles.saturating_add(1);
            let idles = self.idles;
            let due: Vec<_> = self.at_idle.extract_if(.., |(n, _)| *n == idles).collect();
            for (_, f) in due {
                f(self.s);
            }
            crate::window::Wake {
                redraw: false,
                animating: false,
                next_timer: None,
            }
        }
        fn focused(&mut self) -> bool {
            false
        }
        fn renderer_lost(&mut self) {}
        fn chrome(&mut self, _: crate::window::Chrome) -> bool {
            false
        }
        fn window_actions(&mut self) -> Vec<crate::window::WindowAction> {
            Vec::new()
        }
    }

    /// `input` as the platform thread hands it on, at `at` ms, after the
    /// platform sent the render thread its wheels up to the one numbered
    /// `wheels`.
    fn platform(s: &Scenario, input: crate::window::Input, at: u64) -> crate::window::ToUi {
        let at = crate::clock::instant_of(at.saturating_mul(1_000_000));
        crate::window::ToUi::Input(input, at, crate::compositor::Pulled::default(), s.shown().seen())
    }

    /// A list 100 high over 2,000 of content, its offset held by `h`.
    fn list_of(h: ScrollHandle) -> impl Fn() -> crate::Element {
        move || {
            div()
                .width(100.0)
                .height(100.0)
                .overflow_y(Overflow::Scroll)
                .track_scroll(&h)
                .child(div().height(2_000.0))
                .into_element()
        }
    }

    /// A thumb dragged (a held button's move, resampled by the UI loop
    /// and sent with the next frame) jumps its list; a wheel turned after
    /// the pointer moved but before that frame goes on from the jump: the
    /// held move carries the wheels sent before it (none), so it takes in
    /// nothing of the later wheel, which lands on the jump (Codex r6 2).
    #[test]
    fn a_wheel_after_a_held_drag_move_goes_on_from_its_jump() {
        let h = ScrollHandle::new();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&h)
                    .on_drag(move |e| {
                        if e.phase == crate::events::DragPhase::Move {
                            h.scroll_to((0.0, 100.0));
                        }
                    })
                    .child(div().height(2_000.0))
                    .into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario");
        s.hover(50.0, 50.0);
        s.frame("first").expect("a frame");
        use crate::window::Input;
        let msgs = vec![
            platform(&s, Input::MouseDown { x: 50.0, y: 50.0 }, 10),
            platform(&s, Input::PointerMoved { x: 50.0, y: 80.0 }, 20),
            crate::window::ToUi::Render(crate::compositor::thread::FromRender::Begin(
                crate::compositor::MainFrame::at(50_000_000_u64),
            )),
        ];
        let mut through = Through {
            s: &mut s,
            idles: 0,
            // After the move is held, before the frame takes it: the
            // wheel, on the render thread, at 30 ms.
            at_idle: vec![(
                3,
                Box::new(|s: &mut Scenario| {
                    crate::clock::advance_to(std::time::Duration::from_millis(30));
                    assert!(s.wheel(40.0), "the wheel scrolls");
                }),
            )],
        };
        crate::window::run_ui_loop(&mut through, msgs);
        assert!(through.at_idle.is_empty(), "the wheel turned");
        drop(through);
        assert_eq!(h.offset(), (0.0, 100.0), "the move jumped it, the wheel not taken");
        s.frame("the jump comes").expect("a frame");
        s.frame("the wheel after it").expect("a frame");
        assert_eq!(h.offset(), (0.0, 140.0), "the wheel goes on from the jump");
    }

    /// A finger down, then the program jumps the list, then the finger
    /// moves past the slop and pans: writes are made in program order,
    /// none refused (ADR-0032, I5), so the pan moves the list from where
    /// the jump put it (Codex r6 5, under the causal order).
    #[test]
    fn a_pan_after_a_jump_moves_from_it_though_it_began_before() {
        let h = ScrollHandle::new();
        let mut s = Scenario::measuring(list_of(h), 100, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        use crate::events::{TouchEvent, TouchPhase};
        use crate::window::Input;
        let touch = |phase, y| {
            Input::Touch(TouchEvent {
                id: 1,
                kind: crate::layer::PointerKind::Touch,
                phase,
                x: 50.0,
                y,
            })
        };
        let msgs = vec![
            platform(&s, touch(TouchPhase::Started, 80.0), 10),
            platform(&s, touch(TouchPhase::Moved, 20.0), 30),
            platform(&s, touch(TouchPhase::Moved, 0.0), 40),
        ];
        let mut through = Through {
            s: &mut s,
            idles: 0,
            at_idle: vec![(
                2,
                Box::new(move |_: &mut Scenario| {
                    crate::clock::advance_to(std::time::Duration::from_millis(20));
                    h.scroll_to((0.0, 500.0));
                }),
            )],
        };
        crate::window::run_ui_loop(&mut through, msgs);
        assert!(through.at_idle.is_empty(), "the program jumped");
        drop(through);
        let panned = h.offset();
        assert!(panned.1 > 500.0, "the pan moved it on from the jump: {panned:?}");
        s.frame("after").expect("a frame");
        assert_eq!(h.offset(), panned);
    }

    /// A key handler's write made after a program jump (its key pressed
    /// before the jump, handled after it): writes are assignments in the
    /// order made (ADR-0032, I5), so the handler's, made last, stands
    /// (Codex r6 3, under the causal order).
    #[test]
    fn a_handler_write_made_after_a_jump_stands() {
        let h = ScrollHandle::new();
        let mut s = Scenario::measuring(
            move || {
                div()
                    .width(100.0)
                    .height(100.0)
                    .overflow_y(Overflow::Scroll)
                    .track_scroll(&h)
                    .focusable()
                    .on_key_down(move |_| {
                        h.scroll_to((0.0, 10.0));
                        true
                    })
                    .child(div().height(2_000.0))
                    .into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario");
        s.click(50.0, 50.0);
        s.frame("focused").expect("a frame");
        crate::clock::advance_to(std::time::Duration::from_millis(100));
        s.act("the program jumps", || h.scroll_to((0.0, 500.0)));
        let key = crate::events::KeyEvent {
            key: crate::events::Key::Named(crate::events::NamedKey::ArrowDown),
            text: None,
            modifiers: crate::events::Modifiers::default(),
        };
        s.send(crate::window::Input::Key(key));
        s.frame("both").expect("a frame");
        assert_eq!(h.offset(), (0.0, 10.0), "the write made last stands");
    }

    /// A wheel the OS timed at zero, turned after a jump made at 10 s (an
    /// OS clock that is not the frame clock, or one set back): no clock
    /// orders anything, so a wheel turned after is after the jump, and
    /// scrolls on from it (Codex r6 9, r7 7).
    #[test]
    fn a_clock_set_back_keeps_input_after_the_jump_before_it() {
        let h = ScrollHandle::new();
        let mut s = Scenario::measuring(list_of(h), 100, 100, 1.0).expect("a scenario");
        s.hover(50.0, 50.0);
        s.frame("first").expect("a frame");
        crate::clock::script(std::time::Duration::from_secs(10));
        s.act("jump", || h.scroll_to((0.0, 500.0)));
        s.frame("jumped").expect("a frame");
        let c = s.compositor();
        let seen = c.present_last();
        let w = c.wheel_input(crate::compositor::WheelInput {
            point: (50.0, 50.0),
            delta: (0.0, 40.0),
            time: 0,
            precise: false,
            phase: crate::compositor::WheelPhase::None,
            seen,
        });
        assert!(
            matches!(w, crate::compositor::Wheeled::Scrolled),
            "the wheel scrolls: {w:?}"
        );
        s.frame("wheeled").expect("a frame");
        assert_eq!(h.offset(), (0.0, 540.0), "on from the jump");
    }

    /// A key that moves a text field's caret past its box asks the field
    /// to bring it into view at the next layout: the move that layout
    /// makes is one jump (one new generation), whenever layout gets to it.
    #[test]
    fn a_caret_brought_into_view_is_one_jump() {
        let field = crate::TextField::new("a long value that runs well past the end of its narrow box, and on");
        let mut s = Scenario::measuring(
            move || div().child(crate::text_input(&field).width(60.0)).into_element(),
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.frame("first").expect("a frame");
        s.click(10.0, 10.0);
        s.frame("focused").expect("a frame");
        let end = crate::events::KeyEvent {
            key: crate::events::Key::Named(crate::events::NamedKey::End),
            text: None,
            modifiers: crate::events::Modifiers::default(),
        };
        let before = field.scroll_generation();
        s.send(crate::window::Input::Key(end));
        s.frame("revealed").expect("a frame");
        assert!(field.offset().0 > 0.0, "the caret was brought into view");
        assert_eq!(field.scroll_generation(), before.wrapping_add(1), "one jump");
    }

    /// A handle is its making scope's: passed out of a scope that ended,
    /// its owner's code using it is a loud bug (in a debug build), not a
    /// handle that silently does nothing; the engine's own reads stay quiet.
    #[test]
    #[cfg(debug_assertions)]
    fn a_handle_used_after_its_scope_ended_is_a_loud_bug() {
        let scope = crate::Scope::new();
        let h = scope.run(ScrollHandle::new);
        h.scroll_to((0.0, 10.0));
        scope.dispose();
        assert!(!h.is_alive());
        assert_eq!(h.generation(), 0, "the engine reads it quietly");
        assert!(format!("{h:?}").contains("None"), "and its Debug");
        let used = std::panic::catch_unwind(|| h.scroll_to((0.0, 20.0)));
        let message = used
            .expect_err("loud")
            .downcast::<String>()
            .map(|m| *m)
            .unwrap_or_default();
        assert!(message.contains("scroll handle was disposed"), "{message}");
        // Made outside any scope, it lives until disposed by hand.
        let free = ScrollHandle::new();
        crate::Scope::new().dispose();
        free.scroll_to((0.0, 5.0));
        assert_eq!(free.offset(), (0.0, 5.0));
        free.dispose();
    }

    /// A handle reset reads as one never laid out, at the top, under a new
    /// generation; its scrolls counted stay.
    #[test]
    fn a_reset_handle_reads_as_never_laid_out() {
        let h = ScrollHandle::new();
        h.laid_out(ScrollLayout {
            offset: (0.0, 40.0),
            max: (0.0, 100.0),
            pos: motion::units2((0.0, 40.0)),
            end: motion::units2((0.0, 100.0)),
            ..ScrollLayout::default()
        });
        h.wheel(motion::units2((0.0, 50.0)));
        let (generation, wheels) = (h.generation(), h.wheels());
        h.reset();
        assert_eq!(h.layout(), None);
        assert_eq!(h.offset(), (0.0, 0.0));
        assert!(h.generation() > generation, "a new generation");
        assert_eq!(h.wheels(), wheels);
        assert!(!h.take_wheeled());
        h.dispose();
    }

    /// A scroll a fork made (ADR-0028, the alias test) is its own: after
    /// it the handle, and a copy made before it, read what they read
    /// before, and a handle the fork made is gone.
    #[cfg(debug_assertions)]
    #[test]
    fn a_scroll_a_fork_made_is_put_back() {
        let (h, rows) = (ScrollHandle::new(), Signal::new(30_u32));
        let mut s = list(&h, rows);
        s.hover(50.0, 50.0);
        s.frame("first").expect("a frame");
        assert!(s.wheel(60.0));
        s.frame("scrolled").expect("a frame");
        let held = |h: ScrollHandle| {
            (
                h.offset(),
                h.generation(),
                h.wheels(),
                h.reach(),
                h.layout(),
                h.wanted(),
            )
        };
        let alias = h;
        let before = held(h);
        let made = {
            let _cold = crate::engine::ColdFrame::begin();
            h.scroll_to((0.0, 200.0));
            h.add(motion::units2((0.0, 10.0)));
            h.set_reach((0.0, 90.0));
            assert_ne!(held(alias), before, "the fork sees its writes through any copy");
            ScrollHandle::new()
        };
        assert_eq!(held(h), before, "put back");
        assert_eq!(held(alias), before, "the copy made before reads the same");
        assert!(!made.is_alive(), "what the fork made went with it");
        s.frame("after").expect("a frame");
        assert_eq!(h.offset(), before.0, "the live list stays where it was");
    }

    /// A fork's jumps go with it: the live run counts its generations after
    /// a check as it would have without it (rule 3 of ADR-0028).
    #[cfg(debug_assertions)]
    #[test]
    fn a_fork_leaves_the_live_generations_as_they_were() {
        let run = |check: bool| {
            let mut s = list(&ScrollHandle::new(), Signal::new(30_u32));
            s.frame("first").expect("a frame");
            let h = ScrollHandle::new();
            h.scroll_to((0.0, 10.0));
            if check {
                let _cold = crate::engine::ColdFrame::begin();
                for k in 0..5_u8 {
                    h.scroll_to((0.0, f32::from(k)));
                }
            }
            h.scroll_to((0.0, 20.0));
            let after = (h.generation(), h.offset());
            h.dispose();
            after
        };
        assert_eq!(run(true), run(false), "the same after a check as without it");
    }

    /// ADR-0034, S7: a check's first write to a handle laid out with many
    /// children copies none of them: its copy shares the last layout and
    /// the callbacks (by identity), and live reads as it did after.
    #[cfg(debug_assertions)]
    #[test]
    fn a_checks_first_write_to_a_handle_shares_its_layout_and_callbacks() {
        let h = ScrollHandle::new();
        let children = (0..20_000_u64)
            .map(|i| ScrollChild {
                key: Some(i),
                bounds: Bounds::default(),
                focus_within: false,
            })
            .collect();
        h.laid_out(ScrollLayout {
            max: (0.0, 100.0),
            children,
            end: motion::units2((0.0, 100.0)),
            ..ScrollLayout::default()
        });
        h.on_wheel(|| ());
        let roots = |h: ScrollHandle| {
            h.read(|s| {
                (
                    s.laid.clone().expect("laid out"),
                    s.on_wheel.clone().expect("a callback"),
                )
            })
            .expect("alive")
        };
        let (laid, wheel) = roots(h);
        let (shared, bytes) = crate::engine::in_cold_frame(|| {
            crate::heap::allocated_bytes(|| {
                h.set_reach((0.0, 12.0));
                assert_eq!(h.reach(), (0.0, 12.0), "the check reads its write");
                let (l, w) = roots(h);
                Rc::ptr_eq(&l, &laid) && Rc::ptr_eq(&w, &wheel)
            })
        });
        assert!(shared, "the check's copy shares the layout and the callback");
        // The 20,000 children are about a megabyte: the copy is the
        // fork's own bookkeeping.
        assert!(bytes < 8 * 1024, "{bytes} bytes for one write to one handle");
        assert_eq!(h.reach(), (0.0, 0.0), "live is as it was");
        assert_eq!(roots(h).0.children.len(), 20_000);
        assert_eq!(
            Rc::strong_count(&laid),
            2,
            "live's and this test's: the check's copy is gone"
        );
        h.dispose();
    }
}
