//! Component instances (ADR-0013): `setup` runs once per instance and returns
//! its render; the render is tracked and runs again only when something it
//! read changed, or when its parent passes props that differ.
//!
//! [`component`] makes a placeholder ([`Element::Component`]). Each frame
//! [`Root::resolve`] expands the tree: an instance whose render and subtree
//! are clean gives its last output back without running anything; a dirty one
//! renders; a child that left its parent's output unmounts, disposing its
//! scope. Identity is the path of keys (or positions) from the parent's
//! output root plus the type of the setup, as Flutter's and Dioxus' are. A
//! [`crate::Deferred`] layer is no step of that path: content moved between
//! its place in the flow and a floating layer keeps its instances, state
//! and scopes, as content under Vue's `Teleport` does.
//!
//! [`provide`] and [`use_context`] follow the instance tree, each instance
//! sharing its parent's table until it provides something of its own
//! (copy-on-write), as Vue's `provides` does.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use crate::element::{Element, ElementKey, IntoElement};
use crate::reactive::{RenderNode, Scope, Signal, SignalRef};

mod children;
mod control;

use children::Children;

#[cfg(debug_assertions)]
pub(crate) use control::report_duplicate;
pub use control::{
    DuplicateKey, Dynamic, Each, Row, Show, StructureStats, dynamic_view, each, show, structure_stats,
    take_duplicate_keys,
};

/// What an instance's setup gets: its props, reactive.
pub struct Ctx<P: 'static> {
    props: Signal<P>,
}

impl<P: 'static> Clone for Ctx<P> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static> Copy for Ctx<P> {}

impl<P: 'static> Ctx<P> {
    /// The props, as a signal: a render that reads them runs again when the
    /// parent passes props that differ.
    #[must_use]
    pub fn props(&self) -> Signal<P> {
        self.props
    }

    /// A stable handle to the event callback `field` finds in the props
    /// (React's `useEffectEvent`): each call reads the instance's props as
    /// they are then and calls the callback there. Kept in setup, a timer
    /// or a handler on its output, it calls the closure the parent passed
    /// last, which renders nothing again (callbacks compare equal). The
    /// handle is the instance's own: it writes nothing anywhere, and
    /// called once the instance is gone it does nothing (`R::default()`).
    #[must_use]
    pub fn event<A: 'static, R: Default + 'static>(
        &self,
        field: impl Fn(&P) -> &Callback<A, R> + 'static,
    ) -> Callback<A, R> {
        self.event_opt(move |p| Some(field(p)))
    }

    /// [`Self::event`] for an optional callback: one absent when called
    /// gives `R::default()`.
    #[must_use]
    pub fn event_opt<A: 'static, R: Default + 'static>(
        &self,
        field: impl Fn(&P) -> Option<&Callback<A, R>> + 'static,
    ) -> Callback<A, R> {
        let props = self.props;
        Callback {
            f: Rc::new(move |arg| {
                let now = props
                    .is_alive()
                    .then(|| props.with_untracked(|p| field(p).cloned()))
                    .flatten();
                now.and_then(|f| f.call(arg)).unwrap_or_default()
            }),
            // The callback it finds runs as its own maker.
            owner: None,
        }
    }
}

/// A function a parent passes down for an *event* (a click, a change, a
/// pick), compared equal whatever it holds, so passing a new closure each
/// render does not render the child again. It is immutable: a clone is
/// the same closure. It runs untracked, in the instance it was made in
/// (the parent whose render made it, [`handler_owner`]), wherever it is
/// called from: a timer or effect it makes is the parent's, and outlives
/// the child that called it.
///
/// A child that keeps a callback beyond the render that read it (in
/// setup, a timer, a handler on an output that is not rendered again)
/// keeps [`Ctx::event`]'s handle, which calls the one passed last.
///
/// A function a child calls *while it renders*, to compute what it shows
/// (a predicate, a formatter), is a [`Derive`] instead.
pub struct Callback<A: 'static, R: 'static = ()> {
    f: Rc<dyn Fn(A) -> R>,
    owner: Option<crate::reactive::OwnerId>,
}

impl<A: 'static, R: 'static> Callback<A, R> {
    pub fn new(f: impl Fn(A) -> R + 'static) -> Self {
        Self {
            f: Rc::new(f),
            owner: handler_owner(),
        }
    }

    /// Call it, untracked, in the instance that made it; through the one
    /// gate a caller's closure is reached by ([`crate::engine::call_out`]).
    /// `None` when it did not run (ADR-0026): its owner was disposed (a
    /// call into a disposed owner has a defined result, not a run with no
    /// owner), a fork is open (what it writes is the outside world's, which
    /// no rollback puts back, ADR-0028 rule 5), or it panicked inside a
    /// dispatch, which keeps the panic. A caller that asks for a veto
    /// treats `None` as no veto. One whose answer a fork would need is a
    /// bug there, loud in a debug build.
    pub fn call(&self, arg: A) -> Option<R> {
        if let Some(owner) = self.owner
            && !crate::reactive::is_live(owner)
        {
            return None;
        }
        let called = crate::engine::call_out(crate::engine::Site::Callback, || match self.owner {
            Some(owner) => crate::reactive::run_owned(Some(owner), || (self.f)(arg)),
            None => crate::reactive::untracked(|| (self.f)(arg)),
        });
        debug_assert!(
            called.is_some()
                || !crate::engine::forking()
                || std::any::TypeId::of::<R>() == std::any::TypeId::of::<()>(),
            "a fork needs the answer of a caller's callback, which it never calls"
        );
        called
    }
}

impl<A: 'static, R: 'static> Clone for Callback<A, R> {
    fn clone(&self) -> Self {
        Self {
            f: Rc::clone(&self.f),
            owner: self.owner,
        }
    }
}

impl<A: 'static, R: 'static> PartialEq for Callback<A, R> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<A: 'static, R: 'static> fmt::Debug for Callback<A, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback")
    }
}

impl<A: 'static, R: 'static, F: Fn(A) -> R + 'static> From<F> for Callback<A, R> {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

/// A function a component calls while it renders, to compute what it
/// shows: a predicate (a calendar's `is_date_disabled`), a formatter, a
/// derivation. Called inline, tracked as the render that calls it: the
/// signals it reads render that again when they change. Props holding one
/// compare by pointer: a parent passing a new closure renders the child
/// again (one made once, in setup, does not).
pub struct Derive<A: 'static, R: 'static = ()>(Rc<dyn Fn(A) -> R>);

impl<A: 'static, R: 'static> Derive<A, R> {
    /// Its kind is registered ([`crate::engine::Site::Derive`]): a render
    /// calls it inline, and its panic is that render's, contained at the
    /// instance (ADR-0015).
    pub fn new(f: impl Fn(A) -> R + 'static) -> Self {
        crate::engine::stored_kind(crate::engine::Site::Derive);
        Self(Rc::new(f))
    }

    /// Call it here, tracked as the caller.
    pub fn call(&self, arg: A) -> R {
        (self.0)(arg)
    }
}

impl<A: 'static, R: 'static> Clone for Derive<A, R> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<A: 'static, R: 'static> PartialEq for Derive<A, R> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<A: 'static, R: 'static> fmt::Debug for Derive<A, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Derive")
    }
}

impl<A: 'static, R: 'static, F: Fn(A) -> R + 'static> From<F> for Derive<A, R> {
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

/// A value shared by pointer: props holding one compare by `Rc::ptr_eq`, so a
/// large collection costs nothing to compare.
pub struct Shared<T: ?Sized>(pub Rc<T>);

impl<T: ?Sized> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T: ?Sized> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T: ?Sized> std::ops::Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Rc::new(value))
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A value two components share both ways (Vue's `v-model`): bound to a
/// parent's signal, writes go up; unbound, the child keeps it. `on_change`
/// hears every write either way.
pub struct Model<T: 'static> {
    signal: Signal<T>,
    on_change: Option<Callback<T>>,
}

impl<T: 'static> Clone for Model<T> {
    fn clone(&self) -> Self {
        Self {
            signal: self.signal,
            on_change: self.on_change.clone(),
        }
    }
}

impl<T: 'static> PartialEq for Model<T> {
    fn eq(&self, other: &Self) -> bool {
        self.signal == other.signal
    }
}

impl<T: 'static> fmt::Debug for Model<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model").finish_non_exhaustive()
    }
}

impl<T: Clone + PartialEq + 'static> Model<T> {
    /// Bound to `signal`: what the child writes, the parent sees.
    #[must_use]
    pub fn bind(signal: Signal<T>) -> Self {
        Self {
            signal,
            on_change: None,
        }
    }

    /// Kept by whoever makes it, starting at `value` (owned by the scope that
    /// runs now: a component's setup).
    #[must_use]
    #[track_caller]
    pub fn local(value: T) -> Self {
        Self {
            signal: Signal::new(value),
            on_change: None,
        }
    }

    /// Call `f` with each new value written through the model.
    /// A closure becomes a [`Callback`] here, run as the owner that gave
    /// it: what it makes outlives the child that writes.
    #[must_use]
    pub fn on_change(mut self, f: impl Into<Callback<T>>) -> Self {
        self.on_change = Some(f.into());
        self
    }

    /// The value, tracked.
    #[must_use]
    pub fn get(&self) -> T {
        self.signal.get()
    }

    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.signal.with(f)
    }

    #[must_use]
    pub fn get_untracked(&self) -> T {
        self.signal.get_untracked()
    }

    /// Write `value` (nothing happens when it equals the current one).
    pub fn set(&self, value: T) {
        if self.signal.with_untracked(|v| *v == value) {
            return;
        }
        self.signal.set(value.clone());
        if let Some(f) = &self.on_change {
            f.call(value);
        }
    }

    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut v = self.get_untracked();
        f(&mut v);
        self.set(v);
    }

    /// The signal behind it.
    #[must_use]
    pub fn signal(&self) -> Signal<T> {
        self.signal
    }
}

impl<T: Default + Clone + PartialEq + 'static> Default for Model<T> {
    #[track_caller]
    fn default() -> Self {
        Self::local(T::default())
    }
}

impl<T: Clone + PartialEq + 'static> From<Signal<T>> for Model<T> {
    fn from(signal: Signal<T>) -> Self {
        Self::bind(signal)
    }
}

/// A component's `v-model` prop, as reka's `useVModel` reads one: bound to
/// the caller's [`Model`] (`v-model`), held by the caller as a plain value
/// whose writes only call `on_change` (`:model-value` with
/// `@update:model-value`), or kept by the component, starting at a default
/// (`default-value`). `on_change` hears every write in each case.
pub struct Bind<T: 'static> {
    source: Source<T>,
    on_change: Option<Callback<T>>,
}

enum Source<T: 'static> {
    Model(Model<T>),
    Value(T),
    Default(T),
}

impl<T: Clone + 'static> Clone for Bind<T> {
    fn clone(&self) -> Self {
        Self {
            source: match &self.source {
                Source::Model(m) => Source::Model(m.clone()),
                Source::Value(v) => Source::Value(v.clone()),
                Source::Default(v) => Source::Default(v.clone()),
            },
            on_change: self.on_change.clone(),
        }
    }
}

impl<T: PartialEq + 'static> PartialEq for Bind<T> {
    fn eq(&self, other: &Self) -> bool {
        match (&self.source, &other.source) {
            (Source::Model(a), Source::Model(b)) => a == b,
            (Source::Value(a), Source::Value(b)) | (Source::Default(a), Source::Default(b)) => a == b,
            _ => false,
        }
    }
}

impl<T: 'static> fmt::Debug for Bind<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.source {
            Source::Model(_) => "Bind::Model",
            Source::Value(_) => "Bind::Value",
            Source::Default(_) => "Bind::Default",
        })
    }
}

impl<T: Default + 'static> Default for Bind<T> {
    fn default() -> Self {
        Self::default_value(T::default())
    }
}

impl<T: 'static> Bind<T> {
    /// Kept by the component, starting at `value`.
    #[must_use]
    pub fn default_value(value: T) -> Self {
        Self {
            source: Source::Default(value),
            on_change: None,
        }
    }

    /// Bound both ways to `model`.
    #[must_use]
    pub fn model(model: impl Into<Model<T>>) -> Self {
        Self {
            source: Source::Model(model.into()),
            on_change: None,
        }
    }

    /// Shown as `value`, which the caller keeps: a write only calls
    /// `on_change`.
    #[must_use]
    pub fn value(value: T) -> Self {
        Self {
            source: Source::Value(value),
            on_change: None,
        }
    }

    /// This bind, bound to `model` instead.
    #[must_use]
    pub fn with_model(self, model: impl Into<Model<T>>) -> Self {
        Self {
            source: Source::Model(model.into()),
            ..self
        }
    }

    /// This bind, shown as `value` instead.
    #[must_use]
    pub fn with_value(self, value: T) -> Self {
        Self {
            source: Source::Value(value),
            ..self
        }
    }

    /// This bind, kept by the component from `value` instead.
    #[must_use]
    pub fn with_default(self, value: T) -> Self {
        Self {
            source: Source::Default(value),
            ..self
        }
    }

    /// Call `f` with each value written (`@update:model-value`).
    /// A closure becomes a [`Callback`] here, run as the owner that gave
    /// it: what it makes outlives the child that writes.
    #[must_use]
    pub fn on_change(mut self, f: impl Into<Callback<T>>) -> Self {
        self.on_change = Some(f.into());
        self
    }

    /// The value, when the caller keeps it as a plain value: the component
    /// then holds no state and may render as part of its caller.
    #[must_use]
    pub fn as_value(&self) -> Option<&T> {
        match &self.source {
            Source::Value(v) => Some(v),
            Source::Model(_) | Source::Default(_) => None,
        }
    }

    /// Tell `on_change` of `value` (a write to a bind shown as a value).
    pub fn emit(&self, value: T) {
        if let Some(f) = &self.on_change {
            f.call(value);
        }
    }
}

/// What a component's setup makes of a [`Bind`] prop (Vue's `useModel`):
/// read it and write it as one value, whoever keeps it. `Copy`.
pub struct UseModel<P: 'static, T: 'static> {
    props: Signal<P>,
    pick: fn(&P) -> &Bind<T>,
    local: Signal<T>,
    /// Whether the caller keeps the value: decided once, by the bind the
    /// instance was made with (reka's `useVModel` and its `passive` flag).
    caller: bool,
}

impl<P: 'static, T: 'static> Clone for UseModel<P, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: 'static, T: 'static> Copy for UseModel<P, T> {}

impl<P: 'static, T: 'static> fmt::Debug for UseModel<P, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UseModel")
    }
}

/// The model of the [`Bind`] that `pick` finds in `props`, called in setup:
/// a value the component keeps itself is made here, starting at the
/// bind's default (or, when it is bound, at the bound value).
///
/// Who keeps the value is decided here, once, as reka's `useVModel` decides
/// it (ADR-0025): a bind made with a model or a value is the caller's for
/// the instance's life, one made with a default the instance's own. A caller
/// that later drops its value shows the bind's default; one that later
/// passes a value to an instance keeping its own pushes that value in,
/// which no `on_change` echoes; a model it later binds is written by
/// [`UseModel::set`] too.
#[must_use]
#[track_caller]
pub fn use_model<P: 'static, T: Clone + PartialEq + 'static>(
    props: Signal<P>,
    pick: fn(&P) -> &Bind<T>,
) -> UseModel<P, T> {
    let (first, caller) = props.with_untracked(|p| match &pick(p).source {
        Source::Model(m) => (m.get_untracked(), true),
        Source::Value(v) => (v.clone(), true),
        Source::Default(v) => (v.clone(), false),
    });
    let local = Signal::new(first);
    if !caller {
        crate::reactive::watch(
            move || {
                props.with(|p| match &pick(p).source {
                    Source::Model(m) => Some(m.get()),
                    Source::Value(v) => Some(v.clone()),
                    Source::Default(_) => None,
                })
            },
            move |pushed, _| {
                if let Some(v) = pushed
                    && local.with_untracked(|l| l != v)
                {
                    local.set(v.clone());
                }
            },
        );
    }
    UseModel {
        props,
        pick,
        local,
        caller,
    }
}

impl<P: 'static, T: Clone + PartialEq + 'static> UseModel<P, T> {
    /// The value, tracked.
    #[must_use]
    pub fn get(&self) -> T {
        if !self.caller {
            return self.local.get();
        }
        self.props.with(|p| match &(self.pick)(p).source {
            Source::Model(m) => m.get(),
            Source::Value(v) | Source::Default(v) => v.clone(),
        })
    }

    /// Whether the caller keeps the value (controlled), decided as the
    /// instance was made.
    #[must_use]
    pub fn is_controlled(&self) -> bool {
        self.caller
    }

    /// The value, untracked.
    #[must_use]
    pub fn get_untracked(&self) -> T {
        crate::reactive::untracked(|| self.get())
    }

    /// Borrow the value, tracked.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        if !self.caller {
            return self.local.with(f);
        }
        self.props.with(|p| match &(self.pick)(p).source {
            Source::Model(m) => m.with(f),
            Source::Value(v) | Source::Default(v) => f(v),
        })
    }

    /// Write `value`: up to the caller's model, or to the component's own;
    /// then `on_change`. A caller holding a plain value is only told (it
    /// writes the value back, or the shown value stays). An instance
    /// keeping its own value that a caller later binds to a model writes
    /// both, so the model keeps matching what is shown (Vue's passive
    /// `v-model` emits `update:modelValue` the same way). Nothing happens
    /// when it equals the current value, whoever keeps it: one equality
    /// rule, `T`'s `PartialEq` (ADR-0025).
    pub fn set(&self, value: T) {
        let (model, on_change) = self.props.with_untracked(|p| {
            let b = (self.pick)(p);
            let model = match &b.source {
                Source::Model(m) => Some(m.clone()),
                Source::Value(_) | Source::Default(_) => None,
            };
            (model, b.on_change.clone())
        });
        if self.get_untracked() == value {
            return;
        }
        if !self.caller {
            self.local.set(value.clone());
        }
        if let Some(m) = model {
            m.set(value.clone());
        }
        if let Some(f) = on_change {
            f.call(value);
        }
    }

    /// Change the value in place.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut v = self.get_untracked();
        f(&mut v);
        self.set(v);
    }
}

/// What a parent passes a component to show (Vue's slot content, an
/// `as-child` element): built in the parent's render and never equal, so a
/// parent that renders again renders the component again with what it
/// passes now, as Vue does for slots it cannot prove stable. A component
/// rendering again on its own reuses the last one passed, whose instances
/// stay as they are.
pub struct Slot<T>(pub T);

impl<T: Clone> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> PartialEq for Slot<T> {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl<T> fmt::Debug for Slot<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Slot")
    }
}

impl<T> std::ops::Deref for Slot<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// Made by a parent's render and consumed when the frame resolves it.
trait Mount {
    /// Run setup (inside the instance's scope); the props signal and render.
    fn setup(&self) -> (Rc<dyn Any>, Render);
    /// Pass new props to a live instance; it renders again if they differ.
    fn update(&self, props: &dyn Any);
}

/// A render: `Fn`, as Compose's and Flutter's are. What it keeps between
/// runs lives in signals and hooks (made in setup, or `remember`ed), never
/// in its own captures, so running it again (the cold check does) changes
/// nothing of the instance's.
type Render = Rc<dyn Fn() -> Element>;

struct Pending<P, S> {
    props: P,
    setup: S,
}

impl<P, S, R> Mount for Pending<P, S>
where
    P: Clone + PartialEq + 'static,
    S: Clone + FnOnce(Ctx<P>) -> R + 'static,
    R: Fn() -> Element + 'static,
{
    fn setup(&self) -> (Rc<dyn Any>, Render) {
        let props = Signal::new(self.props.clone());
        let render = (self.setup.clone())(Ctx { props });
        (Rc::new(props), Rc::new(render))
    }

    fn update(&self, props: &dyn Any) {
        let Some(signal) = props.downcast_ref::<Signal<P>>() else {
            return;
        };
        // A cold frame passes them all, to its own copy of the signal.
        #[cfg(debug_assertions)]
        if crate::reactive::is_cold() {
            signal.store_cold(self.props.clone());
            return;
        }
        if signal.with_untracked(|old| *old == self.props) {
            // Equal (a `Callback` always is): stored quietly, so a handle
            // calls the latest closures and the child does not render again.
            signal.store_quiet(self.props.clone());
        } else {
            signal.set(self.props.clone());
        }
    }
}

/// A component placeholder in an element tree (see [`component`]).
pub struct ComponentElement {
    pub(crate) key: Option<ElementKey>,
    pub(crate) name: &'static str,
    mount: Box<dyn Mount>,
    type_id: TypeId,
    /// The style its parent gives its output's root (Vue's attribute
    /// fallthrough: a parent that joins or sizes its children by position
    /// styles a component's root as it would a node's).
    pub(crate) fallthrough: Option<Fallthrough>,
    /// A control node's description ([`show`], [`each`], [`dynamic_view`]):
    /// its rows are spliced into its parent's children, with no box.
    control: Option<Rc<dyn control::Control>>,
}

/// A parent's change to a component's root style, applied after the
/// component's own.
pub(crate) type Fallthrough = Rc<dyn Fn(crate::Style) -> crate::Style>;

impl ComponentElement {
    /// Its identity among its siblings.
    #[must_use]
    pub fn key(mut self, key: impl Into<ElementKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The name the debug tools show for it.
    #[must_use]
    pub fn named(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }
}

impl IntoElement for ComponentElement {
    fn into_element(self) -> Element {
        Element::Component(Rc::new(self))
    }
}

/// A component instance at this place in the tree: `setup` runs once, the
/// render it returns runs tracked, again only when what it read changed or
/// the parent passes `props` that differ.
///
/// Its place is its [`ComponentElement::key`] among its siblings, else its
/// position, as Compose's and React's are: an unkeyed component that a
/// sibling is inserted before is another instance (its state, scopes and
/// a drag on its output start over). Children that come and go before
/// others take keys.
///
/// The render is `Fn`, as Compose's composables and Flutter's `build`
/// are: what it keeps from one run to the next is state the runtime holds
/// (a signal or a computed made in `setup`), never a capture it mutates,
/// so a cold frame (the debug check that incremental equals cold) works
/// on that state as it does on the rest of the runtime. A render that
/// memoises keeps its memo in a hook:
///
/// ```
/// use gui::{Computed, Ctx, IntoElement, component, div, text};
/// let _ = component(2_u32, |cx: Ctx<u32>| {
///     let props = cx.props();
///     let squared = Computed::new(move || props.get() * props.get());
///     move || div().child(text(squared.get().to_string())).into_element()
/// });
/// ```
///
/// A memo in a mutable capture does not compile:
///
/// ```compile_fail,E0594
/// use gui::{Ctx, IntoElement, component, div, text};
/// let _ = component(2_u32, |cx: Ctx<u32>| {
///     let props = cx.props();
///     let mut memo: Option<(u32, u32)> = None;
///     move || {
///         let p = props.get();
///         let squared = match memo {
///             Some((q, s)) if q == p => s,
///             _ => p * p,
///         };
///         memo = Some((p, squared));
///         div().child(text(squared.to_string())).into_element()
///     }
/// });
/// ```
#[must_use]
pub fn component<P, S, R>(props: P, setup: S) -> ComponentElement
where
    P: Clone + PartialEq + 'static,
    S: Clone + FnOnce(Ctx<P>) -> R + 'static,
    R: Fn() -> Element + 'static,
{
    ComponentElement {
        key: None,
        name: short_name::<S>(),
        type_id: TypeId::of::<S>(),
        mount: Box::new(Pending { props, setup }),
        fallthrough: None,
        control: None,
    }
}

fn short_name<S>() -> &'static str {
    let full = std::any::type_name::<S>();
    // `crate::module::function::{{closure}}` names the function.
    let trimmed = full.trim_end_matches("::{{closure}}");
    trimmed.rsplit("::").next().unwrap_or(trimmed)
}

/// An instance's place in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct InstanceKey {
    path: u64,
    type_id: TypeId,
}

crate::engine::fork_data_struct! {
    /// An instance: its slot and the generation the slot had when it was
    /// made. A slot freed and used again has a new generation, so an id kept
    /// past its instance (a parent's, a root's) finds nothing, never the
    /// instance made in its place.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct Id {
        slot: usize,
        generation: u32,
    }
}

crate::engine::parts_whole!(Id);

type Context = Rc<HashMap<TypeId, Rc<dyn Any>>>;

struct Instance {
    generation: u32,
    /// The key its output's root is placed by ([`Chain`]): every keyed
    /// component in the chain whose root it is, nested. Its output's root
    /// keeps its place (its path) as keyed siblings move.
    key: Option<ElementKey>,
    /// The place among its siblings of the outermost component in that
    /// chain: what tells apart the roots of unkeyed siblings that key
    /// theirs alike.
    at: usize,
    /// That outermost's static place beside a range, when it is unkeyed
    /// there: its output's root is placed by it
    /// ([`crate::element::Common::place`]).
    place: Option<u32>,
    /// Its last render's output as given and as marked ([`marked`]), when
    /// it was given a node shared with another (a cached output): the
    /// same output given again is the same marked node.
    marked: Option<(Element, Element)>,
    parent: Option<Id>,
    depth: u32,
    name: &'static str,
    scope: Scope,
    node: Option<RenderNode>,
    /// Shared, so new props are passed to it without taking it out (the
    /// reactive code they run may panic).
    props: Rc<dyn Any>,
    render: Render,
    /// The scopes of the last renders (what a render makes, such as a
    /// builder's signal, lives while its output may be on screen).
    render_scopes: Vec<Scope>,
    /// Its own render must run.
    dirty: bool,
    /// Why it is dirty.
    why: Why,
    /// How soon it must render.
    lane: Priority,
    /// Something below it must render.
    below: bool,
    /// The last render's output, with placeholders.
    raw: Option<Element>,
    /// `raw` with every placeholder replaced by its instance's output.
    output: Option<Element>,
    /// In the order they were placed: torn down the last first.
    children: Children,
    /// Its output as its parent's fallthrough styled it: the output it was
    /// made from, the fallthrough, and the result, so a clean instance
    /// gives back the same styled node.
    styled: Option<(Element, Fallthrough, Element)>,
    context: Context,
    mounted: Vec<Box<dyn FnOnce()>>,
    updated: Vec<Rc<dyn Fn()>>,
    renders: u64,
    /// The placeholder it was last resolved from: met again (its parent's
    /// render reused it), it keeps the props it has. Kept here, in the
    /// instance tree, not on the placeholder its parent's output shares.
    met: std::rc::Weak<ComponentElement>,
    /// What panics of user code caught at it in turns left it holding
    /// ([`contain`]): part of its root's state, so one root's hold is
    /// nothing to another's, and what a cold frame of its root reuses as
    /// it is ([`Holds`]).
    holds: Holds,
    /// A control node's rows and description ([`control`]); its rows are
    /// its children, each an instance whose render gives back what its
    /// view built once.
    control: Option<Box<dyn control::State>>,
    /// A control node's last range: its rows' outputs, in order. For
    /// another instance, its output's range when its output is a control
    /// node (a row whose view is a list).
    range: Option<Rc<[Element]>>,
    /// A control node's range as placed where no box takes it ([`boxed`]).
    boxed: Option<(Rc<[Element]>, Element)>,
}

/// What an instance holds over panics of its user code caught in turns
/// ([`contain`], ADR-0015). A cold frame treats each as opaque: it
/// reuses what the live tree holds (the output, the props, the empty
/// slot), never running the code again, and checks everything else.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct Holds {
    /// Its render panicked: it shows its last output.
    render: bool,
    /// The comparison of the props passed to it panicked: it keeps its
    /// props (and its output).
    props: bool,
    /// The setups of the children made at these slots of its output
    /// panicked: each slot is empty until its output is expanded again.
    setups: Vec<InstanceKey>,
}

#[cfg(debug_assertions)]
impl Instance {
    fn cold_copy(&self) -> Self {
        Self {
            generation: self.generation,
            key: self.key,
            at: self.at,
            place: self.place,
            marked: self.marked.clone(),
            parent: self.parent,
            depth: self.depth,
            name: self.name,
            scope: self.scope,
            node: self.node,
            props: Rc::clone(&self.props),
            render: Rc::clone(&self.render),
            render_scopes: self.render_scopes.clone(),
            dirty: self.dirty,
            why: self.why,
            lane: self.lane,
            below: self.below,
            raw: self.raw.clone(),
            output: self.output.clone(),
            children: self.children.clone(),
            styled: self.styled.clone(),
            context: Rc::clone(&self.context),
            mounted: Vec::new(),
            updated: self.updated.clone(),
            renders: self.renders,
            met: self.met.clone(),
            holds: self.holds.clone(),
            control: self.control.as_ref().map(|c| c.clone_box()),
            range: self.range.clone(),
            boxed: self.boxed.clone(),
        }
    }
}

#[derive(Default)]
struct Tree {
    slab: Vec<Option<Instance>>,
    /// Each slot's next generations, by the one slot rule of every arena
    /// ([`crate::engine::Generations`], ADR-0028 rule 3): never put back,
    /// so an instance id a cold frame made names nothing after it, and the
    /// live ids are as without it.
    generations: crate::engine::Generations,
    free: Vec<usize>,
    /// The instance whose setup or render runs now.
    current: Vec<Id>,
    /// While a cold frame runs: its undo log ([`TreeLog`]).
    #[cfg(debug_assertions)]
    cold: Option<Box<TreeLog>>,
    /// The instances cold frames make, in a region of their own (slots
    /// numbered `FORK_INSTANCE | index`), so the live slots, the free ones
    /// and their numbering are as without the frame; emptied by each
    /// rollback, their next generations never put back.
    #[cfg(debug_assertions)]
    fork_slab: Vec<Option<Instance>>,
    #[cfg(debug_assertions)]
    fork_generations: crate::engine::Generations,
}

/// The top bit of a slot: an instance a cold frame made.
const FORK_INSTANCE: usize = 1 << (usize::BITS - 1);

/// One step a cold frame took on the instances, undone at its end.
#[cfg(debug_assertions)]
enum TreeUndo {
    /// A slot that was there before the frame, as it was then.
    Slot(usize, Option<Box<Instance>>),
}

/// What a cold frame did to the instance tree (decision B): each slot that
/// was there before it, kept as it was on its first write.
#[cfg(debug_assertions)]
struct TreeLog {
    undo: Vec<TreeUndo>,
    logged: std::collections::HashSet<usize>,
    /// The live instances running while the frame runs.
    current: Vec<Id>,
}

/// The tree's cold policy ([`TREE`], Fork by undo log): the log starts.
#[cfg(debug_assertions)]
fn tree_cold_enter() -> Box<dyn Any> {
    with_tree(|t| {
        t.cold = Some(Box::new(TreeLog {
            undo: Vec::new(),
            logged: std::collections::HashSet::new(),
            current: std::mem::take(&mut t.current),
        }));
    });
    Box::new(())
}

/// What the frame made, taken out of the tree while it still runs: the
/// instances in the slots it wrote or added.
#[cfg(debug_assertions)]
fn tree_cold_drain(_: &dyn Any) -> Vec<Box<dyn Any>> {
    let made = TREE.try_with(|t| t.try_borrow_mut().map(|mut t| t.drain_cold()).unwrap_or_default());
    made.unwrap_or_default()
}

/// The tree put back as it was, by moves (ADR-0034, S1).
#[cfg(debug_assertions)]
fn tree_cold_exit(_: Box<dyn Any>) {
    TREE.switch_with(|t| {
        if let Ok(mut t) = t.try_borrow_mut() {
            t.rollback();
        }
    });
}

/// How many steps the running cold frame's tree log holds (its cost).
#[cfg(all(debug_assertions, test))]
pub(crate) fn cold_tree_steps() -> usize {
    with_tree(|t| t.cold.as_ref().map_or(0, |l| l.undo.len()))
}

/// A render node's mark: instance, generation, the write that woke it, its lane.
type Mark = (Id, u32, Option<SignalRef>, Priority);

/// A hook due after layout, and the scope that owns it: the instance's, or
/// the owner a handler runs in. Disposed meanwhile, it never runs.
/// A hook due after layout: its owner (disposed, it never runs), and the
/// hook, kept in the window it was queued in (it runs there; once that
/// window closed, never: [`crate::engine::Stored`]).
type PostHook = (crate::reactive::OwnerId, crate::engine::Stored<Box<dyn FnOnce()>>);

crate::engine::engine_local! {
    /// The instances. A cold frame writes them in place, each slot's live
    /// instance kept on its first write and put back at the end.
    [Fork(tree_cold_enter, tree_cold_drain, tree_cold_exit)] static TREE: RefCell<Tree> = RefCell::new(Tree::default());
    /// Instances whose render was marked since the last resolve: their
    /// generation and the signal whose write marked them.
    [Fresh] static MARKED: RefCell<Vec<Mark>> = const { RefCell::new(Vec::new()) };
    /// Idle instances left dirty by the last resolve (its budget ran out).
    [Fresh] static DEFERRED: RefCell<Vec<(Id, u32)>> = const { RefCell::new(Vec::new()) };
    /// The priority writes made now mark renders with.
    [Fork] static PRIORITY: Cell<Priority> = const { Cell::new(Priority::Input) };
    /// Until when the resolve running now may render idle instances.
    [Fork] static DEADLINE: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    /// Hooks due after the frame is laid out (`on_mounted`, `on_updated`).
    /// Each by the scope that owns it: one whose owner was disposed
    /// meanwhile never runs.
    [Fresh] static POST: RefCell<Vec<PostHook>> = RefCell::new(Vec::new());
    /// While a resolve that leaves disposal to its caller runs
    /// ([`Root::resolve_leaving`]): the scopes of the instances it unmounted,
    /// disposed once the caller let go of what they hold.
    [Fresh] static LEAVING: RefCell<Option<Vec<Scope>>> = const { RefCell::new(None) };
    /// Something outside a render that the frame read changed.
    [Fresh] static FRAME_DUE: Cell<bool> = const { Cell::new(false) };
    /// The last resolve left an instance due below the root (marked as
    /// its siblings resolved): another pass is.
    [Fresh] static STRANDED: Cell<bool> = const { Cell::new(false) };
    /// The instances rendered since the last [`take_rendered`].
    [Fresh] static RENDERED: RefCell<Vec<Rendered>> = const { RefCell::new(Vec::new()) };
    [Fork] static STATS: Cell<Stats> = const { Cell::new(Stats { renders: 0, setups: 0, unmounts: 0 }) };
}

/// A panic of user code the layout ran (a binding) caught in a turn: to
/// the arbiter, the binding to hold what it last gave; outside a turn,
/// on to the caller. A cold frame runs no binding the live tree holds
/// (it reuses what it holds, [`crate::layout`]'s held values), so one
/// that panics there panicked where the live one did not: on, a mismatch.
pub(crate) fn contain_binding(failed: crate::engine::Failed) {
    if !crate::engine::dispatching() {
        failed.go_on();
    }
    failed.report();
}

/// What a panic of user code caught at an instance, in a turn, left it
/// holding ([`contain`], [`Holds`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// Its render panicked: it shows its last output.
    Render,
    /// The comparison of the props passed to it panicked: it keeps its
    /// props (and its output).
    Props,
    /// The setup of the child at this slot of its output panicked: the
    /// slot is empty until its output is expanded again.
    Setup(InstanceKey),
}

/// A panic of user code caught at instance `id` (`fault`, ADR-0015, the
/// turn rule). In a turn, as Flutter puts an error widget in the place of
/// the one element whose build threw and draws the rest: the panic goes
/// to the arbiter, and what it left is held by the instance alone
/// ([`Holds`]): the rest of the tree resolves, lays out and draws as it
/// would. Outside a turn, on to the caller: the root is torn down. A cold
/// frame never runs what the live tree holds (it reuses it), so a panic
/// it catches is one the live tree did not have: on, a mismatch.
fn contain(failed: crate::engine::Failed, id: Id, fault: Fault) {
    if !crate::engine::dispatching() {
        failed.go_on();
    }
    failed.report();
    with_tree(|t| {
        let Some(i) = t.get_mut(id) else { return };
        match fault {
            Fault::Render => i.holds.render = true,
            Fault::Props => i.holds.props = true,
            Fault::Setup(slot) => {
                if !i.holds.setups.contains(&slot) {
                    i.holds.setups.push(slot);
                }
            }
        }
    });
}

/// Instance `id` no longer holds `fault`: it rendered, or was passed
/// props, through. A cold frame changes no hold: it reads the live ones.
fn cleared(id: Id, fault: Fault) {
    if crate::engine::is_cold() {
        return;
    }
    with_tree(|t| {
        let Some(i) = t.get_mut(id) else { return };
        match fault {
            Fault::Render => i.holds.render = false,
            Fault::Props => i.holds.props = false,
            Fault::Setup(slot) => i.holds.setups.retain(|s| *s != slot),
        }
    });
}

/// Instance `id`'s output is expanded again: every slot a setup's panic
/// left empty mounts anew ([`cleared`], a cold frame's excepted).
fn cleared_setups(id: Id) {
    if crate::engine::is_cold() {
        return;
    }
    with_tree(|t| {
        if let Some(i) = t.get_mut(id)
            && !i.holds.setups.is_empty()
        {
            i.holds.setups.clear();
        }
    });
}

/// Whether a cold frame reuses what instance `id` holds (`fault`) rather
/// than run its user code: the live tree holds it ([`Holds`]).
fn opaque(id: Id, fault: Fault) -> bool {
    crate::engine::is_cold()
        && with_tree(|t| {
            t.get(id).is_some_and(|i| match fault {
                Fault::Render => i.holds.render,
                Fault::Props => i.holds.props,
                Fault::Setup(slot) => i.holds.setups.contains(&slot),
            })
        })
}

/// How soon a render a write causes must show (ADR-0013's scheduler).
/// `Input` and `Animation` render in the frame they were written for;
/// `Idle` renders while the frame has time left, checked between instances,
/// and otherwise in a later frame, its old output shown until then.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    #[default]
    Input,
    Animation,
    Idle,
}

/// Run `f`, the renders its writes cause marked with `priority`.
pub fn with_priority<R>(priority: Priority, f: impl FnOnce() -> R) -> R {
    struct Restore(Priority);
    impl Drop for Restore {
        fn drop(&mut self) {
            PRIORITY.set(self.0);
        }
    }
    let _restore = Restore(PRIORITY.replace(priority));
    f()
}

/// Run `f`, the renders its writes cause done when the frame has time (a
/// search over a long list as the reader types, React's `startTransition`).
pub fn idle<R>(f: impl FnOnce() -> R) -> R {
    with_priority(Priority::Idle, f)
}

crate::engine::engine_local! {
    /// How long a frame may spend on idle renders.
    [Fork] static IDLE_BUDGET: Cell<std::time::Duration> = const { Cell::new(std::time::Duration::from_millis(4)) };
}

/// Set how long a frame may spend on idle renders (4 ms by default).
pub fn set_idle_budget(budget: std::time::Duration) {
    IDLE_BUDGET.set(budget);
}

/// Why an instance rendered (devtools' "why did this render").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Its first render.
    Mount,
    /// A signal it read was written.
    Signal(SignalRef),
    /// Its parent passed props that differ.
    Props,
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mount => f.write_str("mount"),
            Self::Signal(s) => write!(f, "{s}"),
            Self::Props => f.write_str("props"),
        }
    }
}

/// An instance that rendered, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub name: &'static str,
    pub why: Why,
}

/// Counts since the thread started (devtools, tests, benches).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub renders: u64,
    pub setups: u64,
    pub unmounts: u64,
}

crate::engine::engine_local! {
    [Fresh] static BINDINGS: Cell<u64> = const { Cell::new(0) };
}

/// A bound closure ran (a style or text binding, at layout).
pub(crate) fn count_binding() {
    BINDINGS.set(BINDINGS.get().saturating_add(1));
}

/// How many bound closures ran since the last call.
#[must_use]
pub fn take_bindings() -> u64 {
    BINDINGS.replace(0)
}

/// What the instances did so far.
#[must_use]
pub fn stats() -> Stats {
    STATS.get()
}

fn count(f: impl FnOnce(&mut Stats)) {
    let mut s = STATS.get();
    f(&mut s);
    STATS.set(s);
}

/// Whether a frame is due: an instance must render, or something the last
/// frame read outside renders changed. Clears the second.
#[must_use]
pub fn take_due() -> bool {
    let frame = FRAME_DUE.replace(false);
    frame
        || STRANDED.get()
        || MARKED.with(|m| !m.borrow().is_empty())
        || DEFERRED.with(|d| !d.borrow().is_empty())
        // A hook waiting for layout: the next input is routed after it ran.
        || POST.with(|p| !p.borrow().is_empty())
}

/// Mark the frame due (a tracker of what layout and paint read).
pub(crate) fn frame_due() {
    FRAME_DUE.set(true);
}

fn with_tree<R>(f: impl FnOnce(&mut Tree) -> R) -> R {
    #[cfg(debug_assertions)]
    crate::engine::no_component_context();
    TREE.with(|t| f(&mut t.borrow_mut()))
}

impl Tree {
    /// Slot `slot`, in the live region or the frames'.
    fn slot(&self, slot: usize) -> Option<&Option<Instance>> {
        if slot & FORK_INSTANCE != 0 {
            #[cfg(debug_assertions)]
            return self.fork_slab.get(slot & !FORK_INSTANCE);
            #[cfg(not(debug_assertions))]
            return None;
        }
        self.slab.get(slot)
    }

    fn slot_mut(&mut self, slot: usize) -> Option<&mut Option<Instance>> {
        if slot & FORK_INSTANCE != 0 {
            #[cfg(debug_assertions)]
            return self.fork_slab.get_mut(slot & !FORK_INSTANCE);
            #[cfg(not(debug_assertions))]
            return None;
        }
        self.slab.get_mut(slot)
    }

    fn get(&self, id: Id) -> Option<&Instance> {
        self.slot(id.slot)
            .and_then(Option::as_ref)
            .filter(|i| i.generation == id.generation)
    }

    fn get_mut(&mut self, id: Id) -> Option<&mut Instance> {
        self.log(id.slot);
        self.slot_mut(id.slot)
            .and_then(Option::as_mut)
            .filter(|i| i.generation == id.generation)
    }

    /// In a cold frame, live slot `id` as it
    /// was, kept on its first write.
    #[cfg(debug_assertions)]
    fn log(&mut self, id: usize) {
        if crate::engine::forking()
            && let Some(log) = self.cold.as_deref_mut()
            && id & FORK_INSTANCE == 0
            && log.logged.insert(id)
            && let Some(slot) = self.slab.get_mut(id)
        {
            let copy = slot.as_ref().map(Instance::cold_copy);
            let old = std::mem::replace(slot, copy).map(Box::new);
            log.undo.push(TreeUndo::Slot(id, old));
            crate::engine::fork_work(1);
        }
    }

    #[cfg(not(debug_assertions))]
    #[inline]
    fn log(&mut self, _: usize) {}

    /// The instances the cold frame wrote or added, taken out while it
    /// still runs (each slot's live instance is in the log).
    #[cfg(debug_assertions)]
    fn drain_cold(&mut self) -> Vec<Box<dyn Any>> {
        let Some(log) = self.cold.as_deref() else {
            return Vec::new();
        };
        let slots: Vec<usize> = log.logged.iter().copied().collect();
        let mut made: Vec<Box<dyn Any>> = slots
            .into_iter()
            .filter_map(|i| self.slab.get_mut(i).and_then(Option::take))
            .map(|i| -> Box<dyn Any> { Box::new(i) })
            .collect();
        made.extend(
            self.fork_slab
                .iter_mut()
                .filter_map(Option::take)
                .map(|i| -> Box<dyn Any> { Box::new(i) }),
        );
        made
    }

    /// Put back what the cold frame changed, by moves (ADR-0034, S1): the
    /// slots it wrote and its own were emptied by the drain, so what is
    /// replaced holds no instance; one left is quarantined, never dropped
    /// here.
    #[cfg(debug_assertions)]
    fn rollback(&mut self) {
        let Some(log) = self.cold.take() else { return };
        let TreeLog { undo, current, .. } = *log;
        for step in undo.into_iter().rev() {
            let TreeUndo::Slot(id, old) = step;
            if let Some(slot) = self.slab.get_mut(id)
                && let Some(left) = std::mem::replace(slot, old.map(|b| *b))
            {
                crate::engine::quarantined("TREE", Box::new(left));
            }
        }
        // The live free slots were never touched; the frame's own go.
        let own: Vec<Instance> = self.fork_slab.drain(..).flatten().collect();
        if !own.is_empty() {
            crate::engine::quarantined("TREE", Box::new(own));
        }
        // Ids of the instances current: data.
        self.current = current;
    }

    /// Take instance `id` out (it unmounts): its slot freed; the next
    /// instance there takes a generation none had.
    fn take(&mut self, id: Id) -> Option<Instance> {
        self.get(id)?;
        self.log(id.slot);
        let inst = self.slot_mut(id.slot).and_then(Option::take)?;
        // The frame's own slots, and a live one a running frame empties
        // (the log puts it back), are never freed: the live free slots
        // are as without the frame. A slot whose generations ran out is
        // retired, never reused: an id from before could not be told from
        // one issued after.
        if id.slot & FORK_INSTANCE == 0 && !crate::engine::forking() && self.generations.reusable(id.slot) {
            self.free.push(id.slot);
        }
        Some(inst)
    }

    fn insert(&mut self, inst: Instance) -> Id {
        // The frame's if one runs (the one answer, at the moment it is
        // made): in its own region, under the forks' generations.
        #[cfg(debug_assertions)]
        if crate::engine::forking() && self.cold.is_some() {
            loop {
                let index = self.fork_slab.len();
                if let Some(generation) = self.fork_generations.take(index, true) {
                    self.fork_slab.push(Some(Instance { generation, ..inst }));
                    return Id {
                        slot: index | FORK_INSTANCE,
                        generation,
                    };
                }
                self.fork_slab.push(None);
            }
        }
        while let Some(slot) = self.free.pop() {
            let Some(generation) = self.generations.take(slot, false) else {
                continue;
            };
            if let Some(s) = self.slab.get_mut(slot) {
                *s = Some(Instance { generation, ..inst });
            }
            return Id { slot, generation };
        }
        loop {
            let slot = self.slab.len();
            if let Some(generation) = self.generations.take(slot, false) {
                self.slab.push(Some(Instance { generation, ..inst }));
                return Id { slot, generation };
            }
            // Retired (a rollback cut it off after its generations ran out).
            self.slab.push(None);
        }
    }

    /// Mark `id` to render and its ancestors as having work below.
    fn mark(&mut self, id: Id, generation: u32, why: Why, lane: Priority) {
        let Some(inst) = self.get_mut(id) else { return };
        if inst.generation != generation {
            return;
        }
        if inst.dirty {
            inst.lane = inst.lane.min(lane);
        } else {
            inst.why = why;
            inst.lane = lane;
        }
        inst.dirty = true;
        let mut up = inst.parent;
        while let Some(p) = up {
            let Some(parent) = self.get_mut(p) else { break };
            if parent.below {
                break;
            }
            parent.below = true;
            up = parent.parent;
        }
    }
}

/// Apply the marks queued by render nodes.
fn apply_marks() {
    let marked = MARKED.with(|m| std::mem::take(&mut *m.borrow_mut()));
    if marked.is_empty() {
        return;
    }
    with_tree(|t| {
        for (id, generation, by, lane) in marked {
            t.mark(id, generation, by.map_or(Why::Props, Why::Signal), lane);
        }
    });
}

/// A tree of instances under one root, which a pipeline resolves each frame.
///
/// In a turn, a panic of user code as it resolves is held by the one
/// instance it failed at ([`contain`]): a render keeps its last output,
/// props compared keep the props, a setup's half-made instance is torn
/// down alone (ADR-0015, the turn rule: a failing component does not
/// blank the window). Outside a turn (and for the engine's own panics) it
/// poisons the root: its whole tree is torn down at once, every cleanup
/// run, and the panic goes on to the caller, once; the next resolve mounts
/// the tree again from the root's render, fresh. Either way nothing a
/// frame left half done is reused (ADR-0013): a render and a comparison
/// are pure, and what a setup half made is gone.
///
/// A handle: an id and its render, both fixed as it is made. What changes
/// (the root instance, whether it is poisoned or ended) is the registry's
/// ([`ROOTS`], ADR-0034, S1; review r17, decision 1), so what a check does
/// to a root it reaches is the check's, by construction.
pub struct Root {
    life: Rc<Life>,
    render: Render,
    /// Its last handle's end ([`RootEnd`]).
    _end: crate::engine::Ends<RootEnd>,
}

crate::engine::fork_data_struct! {
    /// What a root is now, by its id ([`Root`]).
    #[derive(Clone, Copy)]
    struct RootState {
        /// The tree's root instance (a poisoned root mounts a new one).
        id: Id,
        /// Unmounted, for good: a resolve after gives nothing.
        ended: bool,
        /// A panic tore the tree down since the last resolve.
        poisoned: bool,
        /// The turn a panic tore the tree down in: down until it is done.
        down: crate::engine::ThisTurn,
    }
}

crate::engine::engine_local! {
    /// Every root's state, by its id: a check's writes are its own.
    [Fork(log)] static ROOTS: RefCell<crate::engine::ForkSlab<RootState>> = RefCell::default();
}

impl Root {
    /// A root instance rendering `render`, owned by what runs now: the
    /// scope running, else the window whose code runs. Its tree unmounts
    /// as the first of that owner and its last handle ends ([`Life`],
    /// ADR-0034 S4), so a handle something else pins (a value a runaway
    /// check set aside) never keeps it mounted past its owner. One a check
    /// makes is the check's own.
    ///
    /// # Panics
    /// In a debug build, with neither a scope running nor a window's code
    /// (a test's top level): make it in a [`Scope`], whose `dispose` ends
    /// it, or in a window.
    #[track_caller]
    pub fn new(render: impl Fn() -> Element + 'static) -> Self {
        let root = Self::made(Rc::new(render));
        let as_owned: Rc<dyn crate::focus::WindowOwned> = Rc::<Life>::clone(&root.life);
        crate::engine::own("gui::Root::new", &as_owned);
        drop(as_owned);
        root
    }

    /// A pipeline's root: its pipeline is its owner, and unmounts it as
    /// its window closes ([`Self::close`]).
    pub(crate) fn of_pipeline(render: impl Fn() -> Element + 'static) -> Self {
        Self::made(Rc::new(render))
    }

    fn made(render: Render) -> Self {
        let id = create(None, Scope::detached(), Box::new(()), Rc::clone(&render), "app");
        let key = crate::engine::allocate(
            &ROOTS,
            RootState {
                id,
                ended: false,
                poisoned: false,
                down: crate::engine::ThisTurn::default(),
            },
        );
        Self {
            life: Rc::new(Life { key }),
            render,
            _end: crate::engine::Ends::new(RootEnd(key)),
        }
    }

    /// Whether a panic tore the tree down since the last resolve: the next
    /// mounts it again.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.life.state().is_some_and(|s| s.poisoned)
    }

    /// The tree's root instance now (a stale id, naming nothing, once the
    /// root's state is gone).
    fn id(&self) -> Id {
        self.life.state().map_or(
            Id {
                slot: usize::MAX,
                generation: 0,
            },
            |s| s.id,
        )
    }

    /// Expand the tree: render what is dirty, reuse what is not. The root's
    /// output with every placeholder replaced.
    ///
    /// In a turn, a panic of user code is held by the instance it failed
    /// at ([`contain`], ADR-0015, the turn rule): the rest of the tree
    /// resolves as it would, and nothing unwinds here. What does (outside
    /// a turn, any panic; in one, the engine's own) poisons the root
    /// ([`Root`]). In a turn, its panic goes to the arbiter and the tree
    /// stays down, its output empty, for the rest of that turn, mounted
    /// again at the first resolve of a later one: never in the turn that
    /// failed, which would run the same code over the same state again.
    /// Outside a turn, the panic goes on to the caller, once, and the next
    /// resolve mounts the tree again.
    pub fn resolve(&self) -> Element {
        match crate::engine::attempt(|| self.resolve_fresh()) {
            Ok(out) => out,
            Err(failed) => {
                self.failed(failed);
                Element::Empty
            }
        }
    }

    /// Resolve, the tree mounted again first if a panic tore it down in
    /// an earlier turn (or outside any); empty while the turn it failed in
    /// runs.
    fn resolve_fresh(&self) -> Element {
        let Some(state) = self.life.state() else {
            return Element::Empty;
        };
        // Its owner ended it: nothing is mounted again.
        if state.ended {
            return Element::Empty;
        }
        if state.poisoned {
            if state.down.is_now() {
                return Element::Empty;
            }
            let id = create(None, Scope::detached(), Box::new(()), Rc::clone(&self.render), "app");
            self.life.write(|s| {
                s.poisoned = false;
                s.id = id;
            });
        }
        self.resolve_live()
    }

    /// A resolve, or the layout of what it built, panicked: the root
    /// poisoned, the panic to the arbiter first, as it came before any of
    /// its teardown's (inside a turn; outside one, it goes on once the tree
    /// is down).
    pub(crate) fn failed(&self, failed: crate::engine::Failed) {
        if crate::engine::dispatching() {
            failed.report();
            self.poison();
        } else {
            self.poison();
            failed.report();
        }
    }

    /// Tear the whole tree down in one pass, every teardown run though one
    /// panics (those panics later than the one that poisoned it: to the
    /// arbiter after it, inside a turn; dropped outside one).
    fn poison(&self) {
        DEADLINE.set(None);
        STRANDED.set(false);
        // What this resolve left its caller to dispose goes now, with the
        // rest: nothing of the tree outlives it.
        let left = LEAVING.with(|l| l.borrow_mut().as_mut().map(std::mem::take));
        let outer = LEAVING.with(|l| l.borrow_mut().take());
        let id = self.id();
        // The engine's re-run retirement of a failed render (ADR-0035, D3):
        // the root keeps its owner and mounts again at a later resolve.
        crate::engine::contained(|| unmount_by(id, Scope::retire));
        crate::engine::contained(|| {
            each_to_completion(left.unwrap_or_default(), Scope::retire);
        });
        LEAVING.with(|l| *l.borrow_mut() = outer);
        // The hooks queued for the instances torn down are cancelled.
        let queued = POST.with(|p| std::mem::take(&mut *p.borrow_mut()));
        let live: Vec<_> = queued
            .into_iter()
            .filter(|(owner, _)| crate::reactive::is_live(*owner))
            .collect();
        POST.with(|p| {
            let mut p = p.borrow_mut();
            let since = std::mem::replace(&mut *p, live);
            p.extend(since);
        });
        self.life.write(|s| {
            s.poisoned = true;
            s.down = crate::engine::ThisTurn::now();
        });
    }

    fn resolve_live(&self) -> Element {
        let root = self.id();
        // What the last resolve left for later is due again.
        let deferred = DEFERRED.with(|d| std::mem::take(&mut *d.borrow_mut()));
        with_tree(|t| {
            for (id, generation) in deferred {
                let (why, lane) = t.get(id).map_or((Why::Props, Priority::Idle), |i| (i.why, i.lane));
                t.mark(id, generation, why, lane);
            }
        });
        apply_marks();
        DEADLINE.set(std::time::Instant::now().checked_add(IDLE_BUDGET.get()));
        let out = resolve(root);
        DEADLINE.set(None);
        let left = with_tree(|t| t.get(root).is_some_and(|i| i.dirty || i.below));
        STRANDED.set(left);
        out
    }

    /// [`Self::resolve`], the instances it unmounted left undisposed: the
    /// engine first cancels what their nodes captured (a drag, a touch),
    /// each handler run while its instance is alive, then drops the
    /// [`Leaving`], which disposes them.
    pub(crate) fn resolve_leaving(&self) -> (Element, Leaving) {
        let outer = LEAVING.with(|l| l.borrow_mut().replace(Vec::new()));
        let guard = LeaveGuard(outer);
        let out = self.resolve();
        let left = LEAVING.with(|l| l.borrow_mut().take()).unwrap_or_default();
        drop(guard);
        (out, Leaving::new(left))
    }

    /// Run `f` in a cold frame of this tree ([`ColdFrame`]), the debug
    /// check that incremental equals cold: its result if it stands, `None`
    /// if the fork was doomed. The frame is scoped by the closure (ADR-0028,
    /// F1 review 11): `f` gets it by reference, so no use of engine state
    /// it made can outlast the fork, which ends as `f` returns or unwinds.
    /// The frame cannot leave the closure (its lifetime is the closure's),
    /// so no fork ends inside a use of a piece it made. Engine-private
    /// (ADR-0034, S10): the pipeline's check of a frame is the production
    /// operation.
    ///
    /// `None` too when the thread's checks are off: an earlier one faulted
    /// (ADR-0034, S2).
    ///
    /// # Panics
    /// `f` panicked: its panic goes on, as a string, once the fork has
    /// ended. Or disposing of what the fork made faulted (ADR-0034, S2).
    /// The tests' form of [`Self::check`].
    #[cfg(all(test, debug_assertions))]
    #[expect(
        clippy::panic,
        reason = "a faulted check fails the debug build's check, as a mismatch does"
    )]
    pub(crate) fn cold<R>(&self, f: impl FnOnce(&ColdFrame) -> R) -> Option<R> {
        match self.check(f) {
            Ok(out) => Some(out),
            Err(crate::engine::CheckReport::Mismatch(crate::engine::Mismatch::Panicked(said))) => {
                std::panic::resume_unwind(Box::new(said))
            }
            Err(crate::engine::CheckReport::Faulted(fault)) => panic!("{fault}"),
            Err(_) => None,
        }
    }

    /// Run `f` in a cold frame of this tree, as [`Self::cold`], its end
    /// said in the closed report (ADR-0034, S10): its result if it stands,
    /// else why not. Never panics for what the check did.
    #[cfg(debug_assertions)]
    pub(crate) fn check<R>(&self, f: impl FnOnce(&ColdFrame) -> R) -> Result<R, crate::engine::CheckReport> {
        use crate::engine::{CheckReport, Ended, Mismatch};
        // What an instance holds over a panic of its user code, the cold
        // frame reuses as the live tree holds it ([`Holds`]): a hold never
        // switches the check off, and checks the rest of the tree.
        let root = self.id();
        match crate::engine::fork(|| f(&ColdFrame::begin(root))) {
            Ended::Kept(out) => Ok(out),
            Ended::Dropped(site) => Err(CheckReport::Skipped(format!(
                "the check reached `{site}`, which cannot answer inside one"
            ))),
            Ended::Cancelled => Err(CheckReport::Cancelled),
            Ended::Poisoned => Err(CheckReport::Skipped(String::from(
                "checks are off on this thread: an earlier one faulted",
            ))),
            Ended::Panicked(said) => Err(CheckReport::Mismatch(Mismatch::Panicked(said))),
            Ended::Faulted(fault) => Err(CheckReport::Faulted(fault.to_string())),
        }
    }
}

/// A frame built cold, inside [`Root::cold`]: one transaction over every
/// piece of per-thread engine state ([`crate::engine::fork`]), the root's
/// instances all due to render. Whatever it does (renders, props passed,
/// instances mounted or unmounted, hooks and cleanups registered, its
/// layout, bindings and paint) is undone as the closure ends, on return or
/// unwind. Engine-private (ADR-0034, S10): no caller outside `gui` holds
/// one, so none can keep one past its check.
#[cfg(debug_assertions)]
pub(crate) struct ColdFrame {
    root: Id,
}

#[cfg(debug_assertions)]
impl ColdFrame {
    fn begin(root: Id) -> Self {
        // Every instance under the root renders, as a tree built from
        // nothing does; another root's are not touched.
        with_tree(|t| {
            let mut todo = vec![root];
            while let Some(id) = todo.pop() {
                let Some(inst) = t.get(id) else { continue };
                let generation = inst.generation;
                todo.extend(inst.children.values().copied());
                t.mark(id, generation, Why::Props, Priority::Input);
            }
        });
        Self { root }
    }

    /// The root's output, the tree resolved cold.
    ///
    /// First the structural referee (ADR-0026): every control node's source
    /// listed again against what its commit gave the parent. A difference
    /// fails the check as a panic of the cold build does: the cold path
    /// differs where the live one did not.
    pub fn resolve(&self) -> Element {
        let wrong = control::referee(self.root);
        if !wrong.is_empty() {
            std::panic::resume_unwind(Box::new(format!(
                "the structural referee: the live structure differs from its sources:\n{}",
                wrong.join("\n")
            )));
        }
        apply_marks();
        resolve(self.root)
    }

    /// The live signals the frame tried to write (none: a render, a
    /// getter or a binding is pure).
    #[must_use]
    pub fn wrote(&self) -> Vec<String> {
        crate::reactive::cold_wrote()
    }
}

impl Root {
    /// Unmount the tree now, as its window closes inside its own turn
    /// (`Pipeline::close`): every instance's cleanups and unmount hooks
    /// run there, in that window. Its drop then finds nothing to unmount.
    /// In a check, in the check's world: its state and tree are the
    /// check's copies (review r17, decision 3).
    pub(crate) fn close(&self) {
        self.life.end();
    }

    /// [`Self::close`], for later: what its pipeline's drop queues to run
    /// live, as the window's close turn runs (review r18, decision 4).
    pub(crate) fn closer(&self) -> impl FnOnce() + 'static {
        let life = Rc::clone(&self.life);
        move || life.end()
    }
}

/// What a root owes the live world: its tree's unmount, once, by the first
/// of its owners to end (ADR-0034, S4): the scope or window it was made
/// in ([`Root::new`]), its pipeline ([`Root::of_pipeline`]), or its last
/// handle's drop. Never only the last: a handle a runaway check set aside
/// pins the count, not the unmount (review r16, Codex 2). It holds only the
/// root's id: whether it ended is the registry's ([`ROOTS`]).
struct Life {
    key: crate::engine::SlabId,
}

impl Life {
    /// The root's state as the running world sees it (`None`: gone).
    fn state(&self) -> Option<RootState> {
        ROOTS.with_borrow(|s| s.read(self.key, |s| *s))
    }

    /// Change the root's state, in the running world.
    fn write(&self, f: impl FnOnce(&mut RootState)) {
        ROOTS.with_borrow(|s| s.write(self.key, f));
    }

    /// Unmount the tree, once, in the running world. A check never ends a
    /// live root: since ADR-0035 D3, an explicit end (its scope's dispose,
    /// its window's close, `Root::close`) made inside a check that would
    /// dispose live nodes is refused as a fault of the check. A root the
    /// check made itself ends in the check's world, with its tree.
    fn end(&self) {
        end_root(self.key);
    }
}

/// Unmount root `key`'s tree, once, in the running world.
fn end_root(key: crate::engine::SlabId) {
    // The thread's engine torn down (a root an app's thread-local held):
    // nothing is left to unmount.
    let id = ROOTS
        .try_with(|s| {
            s.with(|s| {
                s.write(key, |s| {
                    if s.ended {
                        None
                    } else {
                        s.ended = true;
                        Some(s.id)
                    }
                })
            })
        })
        .ok()
        .flatten()
        .flatten();
    let Some(id) = id else { return };
    if std::thread::panicking() {
        // Ended as a panic unwinds: torn down all the same, a panic of
        // its teardown dropped, never raised over the one going on.
        if let Err(later) = crate::engine::attempt(|| unmount(id)) {
            later.dismiss();
        }
    } else {
        unmount(id);
    }
}

impl crate::focus::WindowOwned for Life {
    /// Its window closed: in a check, the check's copy of the window
    /// (review r17, decision 3).
    fn end_with_window(&self) {
        self.end();
    }
}

/// A root's last handle: the tree unmounted, then its state let go, in the
/// world that made it ([`crate::engine::Ends`], ADR-0035). A live root a
/// check lets go is ended once the check has switched back, never in the
/// check's world, however the check lets it go, by a normal drop or as
/// its body unwinds (Codex r15-4).
struct RootEnd(crate::engine::SlabId);

impl crate::engine::Ending for RootEnd {
    fn end(self) {
        let key = self.0;
        let end = move || {
            end_root(key);
            let _gone = ROOTS.try_with(|s| s.with_mut(|s| s.remove(key)));
        };
        if std::thread::panicking() {
            if let Err(later) = crate::engine::attempt(end) {
                later.dismiss();
            }
        } else {
            end();
        }
    }
}

/// The hooks due after layout as the live registry holds them: what V2
/// compares between two engines (review r18, decision 6).
#[cfg(test)]
pub(crate) fn post_summary() -> usize {
    POST.with(|p| p.borrow().len())
}

/// Make an instance (its setup runs in [`mount_child`]).
fn create(parent: Option<Id>, scope: Scope, props: Box<dyn Any>, render: Render, name: &'static str) -> Id {
    let (depth, context) = with_tree(|t| {
        parent.and_then(|p| t.get(p)).map_or((0, Context::default()), |p| {
            (p.depth.saturating_add(1), Rc::clone(&p.context))
        })
    });
    let id = with_tree(|t| {
        t.insert(Instance {
            generation: 0,
            key: None,
            at: 0,
            place: None,
            marked: None,
            parent,
            depth,
            name,
            scope,
            node: None,
            props: Rc::from(props),
            render,
            render_scopes: Vec::new(),
            dirty: true,
            why: Why::Mount,
            lane: Priority::Input,
            below: false,
            raw: None,
            output: None,
            children: Children::default(),
            styled: None,
            context,
            mounted: Vec::new(),
            updated: Vec::new(),
            renders: 0,
            met: std::rc::Weak::new(),
            holds: Holds::default(),
            control: None,
            range: None,
            boxed: None,
        })
    });
    let generation = id.generation;
    let node = scope.run(|| {
        RenderNode::new(move || {
            let by = crate::reactive::waking();
            let lane = PRIORITY.get();
            MARKED.with(|m| m.borrow_mut().push((id, generation, by, lane)));
        })
    });
    with_tree(|t| {
        if let Some(inst) = t.get_mut(id) {
            inst.node = Some(node);
        }
    });
    id
}

/// Mount a child from `mount` under `parent`, at `slot` among its children.
fn mount_child(parent: Id, slot: InstanceKey, mount: &dyn Mount, name: &'static str) -> Id {
    let scope = with_tree(|t| t.get(parent).map(|p| p.scope));
    let scope = scope.map_or_else(Scope::detached, |s| s.run(Scope::new));
    // Setup runs as the instance, so `provide`, hooks and context find it;
    // it is pushed before it exists and filled in after.
    let id = create(Some(parent), scope, Box::new(()), Rc::new(Element::default), name);
    // Its parent owns it before its setup runs: a setup that panics
    // poisons the root, whose teardown reaches it through its parent.
    with_tree(|t| {
        if let Some(p) = t.get_mut(parent) {
            p.children.insert(slot, id);
        }
    });
    with_tree(|t| t.current.push(id));
    let guard = PopCurrent;
    let (props, render) = scope.run(|| crate::reactive::rendering(name, || mount.setup()));
    drop(guard);
    with_tree(|t| {
        if let Some(inst) = t.get_mut(id) {
            inst.props = props;
            inst.render = render;
        }
    });
    count(|s| s.setups = s.setups.saturating_add(1));
    id
}

struct PopCurrent;

impl Drop for PopCurrent {
    fn drop(&mut self) {
        TREE.with(|t| {
            if let Ok(mut t) = t.try_borrow_mut() {
                t.current.pop();
            }
        });
    }
}

/// Unmount `id` and everything below it: hooks, then its scope.
fn unmount(id: Id) {
    unmount_by(id, Scope::dispose);
}

/// [`unmount`], its scopes ended by `end` (the engine's retirement of a
/// failed render's tree passes [`Scope::retire`], ADR-0035, D3).
fn unmount_by(id: Id, end: fn(Scope)) {
    let Some(inst) = with_tree(|t| t.take(id)) else {
        return;
    };
    // Each child, then each scope, though one's hooks panic: the first
    // panic goes on once all are down (inside a turn, to its arbiter).
    // One order (ADR-0026, "Teardown has one order"): the children the
    // last placed first, as a scope runs its cleanups the last made first.
    let mut all = crate::engine::Gather::new();
    for child in inst.children.values().rev() {
        all.run(|| unmount_by(*child, end));
    }
    let mut scopes = inst.render_scopes;
    scopes.push(inst.scope);
    // Left to the resolve's caller, or disposed now: `on_unmounted` and
    // `on_cleanup` run as the scope is disposed.
    let kept = LEAVING.with(|l| match l.borrow_mut().as_mut() {
        Some(leaving) => {
            leaving.append(&mut scopes);
            true
        }
        None => false,
    });
    if !kept {
        all.run(|| each_to_completion(scopes, end));
    }
    count(|s| s.unmounts = s.unmounts.saturating_add(1));
    all.finish();
}

/// The instances a resolve unmounted, their scopes not yet disposed
/// ([`Root::resolve_leaving`]): what they own is alive until it is dropped,
/// which disposes them, each instance's render scopes then its own, the
/// deepest first.
#[must_use]
pub(crate) struct Leaving(crate::engine::Ends<Scopes>);

/// The scopes of a [`Leaving`], disposed in the world that made it
/// ([`crate::engine::Ends`], ADR-0035).
pub(crate) struct Scopes(Vec<Scope>);

impl Leaving {
    fn new(scopes: Vec<Scope>) -> Self {
        Self(crate::engine::Ends::new(Scopes(scopes)))
    }

    /// Whether no instance left.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.0.is_empty()
    }

    /// Whether `owner` (as [`handler_owner`] gives it) is what a leaving
    /// instance owns its handlers by.
    pub(crate) fn holds(&self, owner: crate::reactive::OwnerId) -> bool {
        self.0.0.iter().any(|s| s.id().owner_id() == owner)
    }
}

impl crate::engine::Ending for Scopes {
    fn end(self) {
        // Every one is disposed, though one's cleanup panics. Ended as a
        // panic unwinds, a cleanup's panic is dropped: raised over the one
        // going on, it would abort the process.
        let scopes = self.0;
        if std::thread::panicking() {
            if let Err(later) = crate::engine::attempt(|| each_to_completion(scopes, Scope::dispose)) {
                later.dismiss();
            }
        } else {
            each_to_completion(scopes, Scope::dispose);
        }
    }
}

/// Run `f` on every item, though one panics: the first panic goes on once
/// all have run (a teardown a panic skipped would never run).
pub(crate) fn each_to_completion<T>(items: impl IntoIterator<Item = T>, mut f: impl FnMut(T)) {
    let mut all = crate::engine::Gather::new();
    for item in items {
        all.run(|| f(item));
    }
    all.finish();
}

/// Puts back the disposal a resolve left to its caller, should it unwind.
struct LeaveGuard(Option<Vec<Scope>>);

impl Drop for LeaveGuard {
    fn drop(&mut self) {
        let left = LEAVING.with(|l| std::mem::replace(&mut *l.borrow_mut(), self.0.take()));
        drop(left.map(Leaving::new));
    }
}

/// How many render scopes an instance keeps: what is on screen and what a
/// leaving node may still be found in.
const KEPT_RENDERS: usize = 2;

fn resolve(id: Id) -> Element {
    let past = DEADLINE.get().is_some_and(|d| std::time::Instant::now() >= d);
    let (dirty, below, cached) = with_tree(|t| {
        t.get(id).map_or((false, false, None), |i| {
            // An idle render waits when the frame's time is spent: its old
            // output stays on screen, and it renders in a later frame.
            let defer = i.dirty && i.lane == Priority::Idle && i.output.is_some() && past;
            if defer {
                DEFERRED.with(|d| d.borrow_mut().push((id, i.generation)));
            }
            let dirty = i.dirty && !defer;
            let cached = (!dirty && !i.below).then(|| i.output.clone()).flatten();
            (dirty, i.below, cached)
        })
    });
    if let Some(out) = cached {
        return out;
    }
    if dirty {
        render(id);
    } else if !below {
        return Element::Empty;
    }
    // Expand the raw output: placeholders become their instance's output.
    // Shared, not taken (an `Rc` clone): the children's setups, renders and
    // effects run within, and one that panics leaves it in place.
    let raw = with_tree(|t| t.get(id).and_then(|i| i.raw.clone()));
    let Some(raw) = raw else {
        return Element::Empty;
    };
    let before: Children = with_tree(|t| t.get(id).map(|i| i.children.clone()).unwrap_or_default());
    let mut seen = Children::with_capacity(before.len());
    // A component that is the root of this output is placed by this
    // instance's key, its own nested in it.
    let chain = with_tree(|t| t.get(id).map(|i| Chain::Root(i.key, i.at, i.place))).unwrap_or(Chain::Child(0));
    // Expanded again: a slot a setup's panic left empty mounts anew.
    cleared_setups(id);
    // An output that is a control node is a range: kept whole for a
    // parent that splices it (a row of a control node), and given to any
    // other in one implicit box.
    let (out, range) = match &raw {
        Element::Component(c) if c.control.is_some() => {
            let (ctl, range) = expand_control(id, c, 0, &before, &mut seen);
            // The box stands in for this instance's root: placed by the
            // instance's key, made by it, as `mark_root` places a root.
            let place = with_tree(|t| t.get(id).map(|i| (i.key, i.place, i.scope.id().owner_id())));
            let (key, at, owner) = place.map_or((None, None, None), |(k, p, o)| (k, p, Some(o)));
            (boxed(ctl, &range, key, at, owner), Some(range))
        }
        _ => (
            expand(id, &raw, 0, chain, &before, &mut seen).unwrap_or_else(|| raw.clone()),
            None,
        ),
    };
    // Those that left, the last placed first (the teardown order).
    each_to_completion(
        before
            .iter()
            .rev()
            .filter(|(slot, child)| seen.get(slot) != Some(*child)),
        |(_, child)| unmount(*child),
    );
    with_tree(|t| {
        // A child marked while its siblings resolved (an effect of a later
        // one's wrote what it read) is still due: the work below stays
        // marked, so the next pass reaches it (never cleared while one is).
        let pending = seen.values().any(|&c| t.get(c).is_some_and(|c| c.dirty || c.below));
        if let Some(i) = t.get_mut(id) {
            i.raw = Some(raw);
            i.children = seen;
            i.output = Some(out.clone());
            if i.control.is_none() {
                i.range = range;
            }
            i.below = pending;
        }
    });
    out
}

/// The range of control node `ctl` in one box, where no box takes it (a
/// component's root, a layer's content, a paragraph's inline): the same
/// box while the range and its place are the same, so layout patches it as
/// kept. The box takes the place of what it stands in for (`key`: the
/// owning instance's key, or the static slot or key of the layer it is the
/// content of), so its path is the one that root would have had.
fn boxed(
    ctl: Id,
    range: &Rc<[Element]>,
    key: Option<ElementKey>,
    place: Option<u32>,
    owner: Option<crate::reactive::OwnerId>,
) -> Element {
    let (key, place) = if control::suppressed(control::Suppress::Unplaced) {
        (None, None)
    } else {
        (key, place)
    };
    let cached = with_tree(|t| {
        t.get(ctl)
            .and_then(|i| i.boxed.as_ref())
            .filter(|(r, b)| {
                Rc::ptr_eq(r, range) && b.common().map(|c| (c.component_key, c.place)) == Some((key, place))
            })
            .map(|(_, b)| b.clone())
    });
    if let Some(b) = cached {
        return b;
    }
    let mut b = crate::element::div().children(range.iter().cloned()).into_element();
    if let Some(c) = b.common_mut() {
        c.component_key = key;
        c.place = place;
        if owner.is_some() {
            c.handlers.instance = owner;
        }
    }
    with_tree(|t| {
        if let Some(i) = t.get_mut(ctl) {
            i.boxed = Some((Rc::clone(range), b.clone()));
        }
    });
    b
}

/// What instance `id` gives a parent that splices: its range when its
/// output is a control node's, else its one output.
fn resolve_into(id: Id, out: &mut Vec<Element>) {
    let one = resolve(id);
    match with_tree(|t| t.get(id).and_then(|i| i.range.clone())) {
        Some(range) if control::truncated(range.len()) => out.extend(range.iter().take(1).cloned()),
        Some(range) => out.extend(range.iter().cloned()),
        None => out.push(one),
    }
}

/// Where a component sits, for the key its output's root is placed by.
#[derive(Clone, Copy)]
enum Chain {
    /// Child `n` of a node.
    Child(usize),
    /// The root of an instance's output: that instance's key (the chain's
    /// so far), its outermost's place, and that one's static place beside
    /// a range when it is unkeyed there.
    Root(Option<ElementKey>, usize, Option<u32>),
}

impl Chain {
    /// The key a component keyed `own` placed here is placed by, its
    /// outermost's place, and that one's static place. Every component of
    /// a chain of roots adds its own: wrappers keyed 1 and 2 whose roots
    /// are each keyed 7 place them apart, as do unkeyed wrappers side by
    /// side, by their places.
    fn place(self, own: Option<ElementKey>) -> (Option<ElementKey>, usize, Option<u32>) {
        match (self, own) {
            (Self::Child(n), own) => (own, n, None),
            (Self::Root(outer, n, p), None) => (outer, n, p),
            (Self::Root(Some(outer), n, _), Some(own)) => {
                (Some(path_step(path_step(0x6e65_7374, outer), own)), n, None)
            }
            (Self::Root(None, n, _), Some(own)) => (
                Some(path_step(
                    path_step(0x0070_6c63, u64::try_from(n).unwrap_or(u64::MAX)),
                    own,
                )),
                n,
                None,
            ),
        }
    }
}

fn path_step(path: u64, step: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    step.hash(&mut h);
    h.finish()
}

/// `raw` (an instance's own output) with every placeholder replaced by its
/// instance's output, or `None` when `raw` holds no placeholder. What holds
/// none is shared, not copied: only the boxes on the way to a placeholder
/// are new. In `raw` they stay placeholders, with their mounts consumed.
fn expand(
    parent: Id,
    raw: &Element,
    path: u64,
    chain: Chain,
    before: &Children,
    seen: &mut Children,
) -> Option<Element> {
    match raw {
        // A control node where no box takes a range (a layer's content, a
        // paragraph's inline): its rows in one implicit box, all of them,
        // live and cold alike (ADR-0026, "Control nodes").
        // The box takes the place the chain gives (a component's key, a
        // static slot beside a range), else its position.
        Element::Component(c) if c.control.is_some() => {
            let (ctl, range) = expand_control(parent, c, path, before, seen);
            let (key, place) = match chain {
                Chain::Root(key, _, place) => (key, place),
                Chain::Child(_) => (None, None),
            };
            Some(boxed(ctl, &range, key, place, None))
        }
        Element::Component(c) => {
            let slot = InstanceKey {
                path,
                type_id: c.type_id,
            };
            if seen.contains_key(&slot) {
                eprintln!(
                    "gui: two `{}` components share one key under one parent; the second is not shown",
                    c.name
                );
                return Some(Element::Empty);
            }
            // The placeholder its instance last met (its parent's render
            // reused it) passes nothing new; one whose instance left mounts
            // again.
            let child = match before.get(&slot) {
                Some(&child) => {
                    let fresh = with_tree(|t| {
                        t.get_mut(child).is_some_and(|i| {
                            let met = std::rc::Weak::ptr_eq(&i.met, &Rc::downgrade(c));
                            if !met {
                                i.met = Rc::downgrade(c);
                            }
                            !met
                        })
                    });
                    // Shared, not taken out, while new props are passed:
                    // writing them runs reactive code that may read the
                    // tree, or panic.
                    let props = fresh
                        .then(|| with_tree(|t| t.get(child).map(|i| Rc::clone(&i.props))))
                        .flatten();
                    // Their comparison is user code: one that panics in a
                    // turn leaves the child its props and its output
                    // ([`contain`]); the next props passed reach it.
                    // A cold frame keeps the props the live child holds.
                    if let Some(props) = props
                        && !opaque(child, Fault::Props)
                    {
                        let passed = crate::engine::attempt(|| c.mount.update(&*props));
                        match passed {
                            Ok(()) => cleared(child, Fault::Props),
                            Err(failed) => contain(failed, child, Fault::Props),
                        }
                    }
                    child
                }
                None => {
                    // A setup that panics in a turn leaves nothing of the
                    // instance it was making: torn down, its slot empty
                    // until its parent's output is expanded again. A cold
                    // frame leaves empty the slot the live one holds empty.
                    if opaque(parent, Fault::Setup(slot)) {
                        return Some(Element::Empty);
                    }
                    let made = crate::engine::attempt(|| mount_child(parent, slot, &*c.mount, c.name));
                    let child = match made {
                        Ok(child) => child,
                        Err(failed) => {
                            // Outside a turn, on to the caller first: the
                            // root's teardown reaches the half-made one
                            // through its parent.
                            contain(failed, parent, Fault::Setup(slot));
                            let half = with_tree(|t| t.get_mut(parent).and_then(|p| p.children.remove(&slot)));
                            if let Some(half) = half {
                                unmount(half);
                            }
                            return Some(Element::Empty);
                        }
                    };
                    with_tree(|t| {
                        if let Some(i) = t.get_mut(child) {
                            i.met = Rc::downgrade(c);
                        }
                    });
                    child
                }
            };
            seen.insert(slot, child);
            let (key, at, place) = chain.place(c.key);
            with_tree(|t| {
                if let Some(i) = t.get_mut(child)
                    && (i.key, i.at, i.place) != (key, at, place)
                {
                    i.key = key;
                    i.at = at;
                    i.place = place;
                    // Kept, its place changed: its root marked again from
                    // the chain as it is now, and its output expanded
                    // again (a root component of its takes the new chain),
                    // with no render.
                    if let Some(raw) = i.raw.take() {
                        i.raw = Some(mark_root(&mut i.marked, raw, key, place, i.scope.id().owner_id()));
                        i.below = true;
                    }
                }
            });
            apply_marks();
            let out = resolve(child);
            Some(match &c.fallthrough {
                Some(f) => fall_through(child, out, f),
                None => out,
            })
        }
        // A box's children and a paragraph's atomic inlines alike.
        // A control node among a box's children gives its rows in its
        // place, with no box of its own (ADR-0026). A paragraph's atomic
        // inlines stand on its content's object marks, one each: there it
        // gives one.
        Element::Div(_) | Element::Text(_) => {
            let all = raw.kids()?;
            let splices = matches!(raw, Element::Div(_));
            let mut kids: Option<Vec<Element>> = None;
            let copied = |i: usize| {
                let mut v = Vec::with_capacity(all.len());
                v.extend(all.iter().take(i).cloned());
                v
            };
            // Beside a range, an unkeyed component's root is placed by its
            // static place, as [`mark_statics`] places the other nodes.
            let ranged = splices && all.iter().any(is_control);
            for (i, rc) in all.iter().enumerate() {
                let at = path_step(path, child_step(rc, i));
                if let Element::Component(c) = rc
                    && c.control.is_some()
                    && splices
                {
                    let (_, range) = expand_control(parent, c, at, before, seen);
                    kids.get_or_insert_with(|| copied(i)).extend(range.iter().cloned());
                    continue;
                }
                let chain = match rc {
                    Element::Component(c) if ranged && c.key.is_none() => Chain::Root(None, i, u32::try_from(i).ok()),
                    // A layer whose content is a range: its box is placed
                    // by the layer's static slot.
                    Element::Deferred(d) if ranged && is_control(&d.child) => {
                        Chain::Root(None, i, u32::try_from(i).ok())
                    }
                    _ => Chain::Child(i),
                };
                match expand(parent, rc, at, chain, before, seen) {
                    Some(new) => kids.get_or_insert_with(|| copied(i)).push(new),
                    None => {
                        if let Some(kids) = kids.as_mut() {
                            kids.push(rc.clone());
                        }
                    }
                }
            }
            kids.map(|kids| raw.with_kids(kids))
        }
        // A layer adds no step: what moves between its place in the flow and
        // a floating layer keeps its instances (Vue's `Teleport`).
        Element::Deferred(r) => expand(parent, &r.child, path, chain, before, seen).map(|child| {
            Element::Deferred(Rc::new(crate::element::Deferred {
                positioning: r.positioning,
                placed: r.placed,
                anchor_part: r.anchor_part,
                tab_after: r.tab_after,
                child,
            }))
        }),
        _ => None,
    }
}

/// The range of the control node `c` places at `path` under `parent`: its
/// instance made on first sight (a scope under the parent's, its source
/// tracked by its own node), given the newer description when a parent on
/// the old path rendered again, and resolved ([`control::resolve_control`]).
fn expand_control(
    parent: Id,
    c: &Rc<ComponentElement>,
    path: u64,
    before: &Children,
    seen: &mut Children,
) -> (Id, Rc<[Element]>) {
    let none = || {
        (
            Id {
                slot: usize::MAX,
                generation: 0,
            },
            Rc::from(Vec::new()),
        )
    };
    let Some(desc) = c.control.as_ref() else {
        return none();
    };
    let slot = InstanceKey {
        path,
        type_id: c.type_id,
    };
    if seen.contains_key(&slot) {
        eprintln!(
            "gui: two `{}` nodes share one key under one parent; the second is not shown",
            c.name
        );
        return none();
    }
    let child = match before.get(&slot) {
        Some(&child) => {
            with_tree(|t| {
                if let Some(i) = t.get_mut(child)
                    && !std::rc::Weak::ptr_eq(&i.met, &Rc::downgrade(c))
                {
                    i.met = Rc::downgrade(c);
                    if let Some(state) = i.control.as_mut() {
                        state.renew(desc.as_any());
                    }
                    i.dirty = true;
                }
            });
            child
        }
        None => {
            let scope = with_tree(|t| t.get(parent).map(|p| p.scope));
            let scope = scope.map_or_else(Scope::detached, |s| s.run(Scope::new));
            let child = create(Some(parent), scope, Box::new(()), Rc::new(Element::default), c.name);
            with_tree(|t| {
                if let Some(i) = t.get_mut(child) {
                    i.control = Some(desc.fresh());
                    i.met = Rc::downgrade(c);
                }
                if let Some(p) = t.get_mut(parent) {
                    p.children.insert(slot, child);
                }
            });
            child
        }
    };
    seen.insert(slot, child);
    apply_marks();
    (child, control::resolve_control(child))
}

fn is_control(el: &Element) -> bool {
    matches!(el, Element::Component(c) if c.control.is_some())
}

/// Whether `el` holds a control node, not looking into components (each
/// marks its own output).
fn holds_control(el: &Element) -> bool {
    match el {
        Element::Deferred(d) => holds_control(&d.child),
        other => other
            .kids()
            .is_some_and(|k| k.iter().any(|k| is_control(k) || holds_control(k))),
    }
}

/// `raw` with each unkeyed node beside a range in a box placed by its
/// static place among its parent's elements
/// ([`crate::element::Common::place`], ADR-0026,
/// "the node's static child position"): the place it had before any
/// range, so a range added, a branch opened or rows inserted before it
/// leave its path, and with it its node id, as they were. Done once per render, so the marked nodes are the
/// same between renders. What holds no control node is not copied.
fn mark_statics(mut raw: Element) -> Element {
    if !holds_control(&raw) {
        return raw;
    }
    if let Element::Deferred(d) = &mut raw {
        let d = Rc::make_mut(d);
        d.child = mark_statics(std::mem::take(&mut d.child));
        return raw;
    }
    let ranged = matches!(raw, Element::Div(_)) && raw.kids().is_some_and(|k| k.iter().any(is_control));
    if let Some(kids) = raw.kids_mut() {
        for (i, kid) in kids.iter_mut().enumerate() {
            if ranged {
                let place = u32::try_from(i).ok();
                let unkeyed =
                    |c: &crate::element::Common| c.key.is_none() && c.component_key.is_none() && c.place != place;
                if let Element::Deferred(d) = &mut *kid {
                    if d.child.common().is_some_and(unkeyed)
                        && let Some(c) = Rc::make_mut(d).child.common_mut()
                    {
                        c.place = place;
                    }
                } else if kid.common().is_some_and(unkeyed)
                    && let Some(c) = kid.common_mut()
                {
                    c.place = place;
                }
            }
            *kid = mark_statics(std::mem::take(kid));
        }
    }
    raw
}

/// Instance's render output `raw`, its root marked ([`marked`]) where it
/// is not yet: written in place when the output is its own; copied when
/// shared (a cached output), the copy kept in `cache`, so the same output
/// given again is the same marked node.
fn mark_root(
    cache: &mut Option<(Element, Element)>,
    raw: Element,
    key: Option<ElementKey>,
    place: Option<u32>,
    owner: crate::reactive::OwnerId,
) -> Element {
    if !unmarked(&raw, key, place, owner) {
        return raw;
    }
    if !raw.is_shared() {
        *cache = None;
        return marked(raw, key, place, owner);
    }
    if let Some((from, to)) = cache.as_ref()
        && from.ptr_eq(&raw)
    {
        return to.clone();
    }
    let to = marked(raw.clone(), key, place, owner);
    *cache = Some((raw, to.clone()));
    to
}

/// Whether `raw`'s root (through a layer around it) lacks the mark
/// [`marked`] writes.
fn unmarked(raw: &Element, key: Option<ElementKey>, place: Option<u32>, owner: crate::reactive::OwnerId) -> bool {
    match raw {
        // A component at the root marks its own, placed by this key.
        Element::Component(_) => false,
        Element::Deferred(d) => unmarked(&d.child, key, place, owner),
        other => other
            .common()
            .is_some_and(|c| c.component_key != key || c.place != place || c.handlers.instance != Some(owner)),
    }
}

/// An instance's render output `raw` with its root (through a layer around
/// it) marked: placed by the instance's key when it is keyed (its own
/// path segment, [`crate::geometry::PathSeg::Component`]: the root keeps
/// its path as keyed siblings are reordered, as React keeps a keyed
/// component's DOM, and shares it with no sibling), and made by the
/// instance (`owner`, its scope): what tells a node it made from one
/// another instance made at its path.
fn marked(mut raw: Element, key: Option<ElementKey>, place: Option<u32>, owner: crate::reactive::OwnerId) -> Element {
    if let Element::Deferred(d) = &raw {
        return Element::Deferred(Rc::new(crate::element::Deferred {
            positioning: d.positioning,
            placed: d.placed,
            anchor_part: d.anchor_part,
            tab_after: d.tab_after,
            child: marked(d.child.clone(), key, place, owner),
        }));
    }
    if let Some(c) = raw.common_mut() {
        c.component_key = key;
        c.place = place;
        c.handlers.instance = Some(owner);
    }
    raw
}

/// Instance `id`'s output `out` with its parent's fallthrough `f` on its
/// root, the same node as last time when neither changed.
fn fall_through(id: Id, out: Element, f: &Fallthrough) -> Element {
    let cached = with_tree(|t| {
        t.get(id).and_then(|i| {
            i.styled
                .as_ref()
                .filter(|(from, by, _)| from.ptr_eq(&out) && Rc::ptr_eq(by, f))
                .map(|(_, _, styled)| styled.clone())
        })
    });
    if let Some(styled) = cached {
        return styled;
    }
    let by = Rc::clone(f);
    let styled = out.clone().restyle(move |s| by(s));
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            i.styled = Some((out, Rc::clone(f), styled.clone()));
        }
    });
    styled
}

/// A child's step in a path: its key when it has one, else its position.
fn child_step(el: &Element, index: usize) -> u64 {
    let key = match el {
        Element::Component(c) => c.key,
        // A layer is its content's place: keyed as its content is.
        Element::Deferred(d) => return child_step(&d.child, index),
        other => other.common().and_then(|c| c.key),
    };
    key.map_or(u64::try_from(index).unwrap_or(u64::MAX), |k| path_step(0x006b_6579, k))
}

/// Run instance `id`'s render, tracked, inside a fresh render scope.
fn render(id: Id) {
    // A cancellation point per instance a check renders (ADR-0028).
    crate::engine::cancel_point();
    // A cold frame reuses the output the live instance holds over its
    // render's panic, never running it again ([`Holds`]).
    let held = opaque(id, Fault::Render);
    let Some((f, node, scope, rendered)) = with_tree(|t| {
        let i = t.get_mut(id)?;
        i.dirty = false;
        if held {
            return None;
        }
        let rendered = Rendered {
            name: i.name,
            why: i.why,
        };
        Some((Rc::clone(&i.render), i.node?, i.scope, Some(rendered)))
    }) else {
        return;
    };
    let rs = scope.run(Scope::new);
    with_tree(|t| t.current.push(id));
    let pop = PopCurrent;
    let name = rendered.as_ref().map_or("component", |r| r.name);
    // Running already (a render that resolves itself), it gives nothing.
    let ran = crate::engine::attempt(|| rs.run(|| crate::reactive::rendering(name, || node.track(|| f()))));
    drop(pop);
    let raw = match ran {
        Ok(raw) => raw,
        Err(failed) => {
            // A render is pure: one that panicked changed nothing of the
            // instance but what it made in its own scope, disposed now.
            // In a turn the instance keeps its last output (none before
            // its first render) and renders again when what it read
            // before the panic changes ([`contain`]).
            contain(failed, id, Fault::Render);
            rs.dispose();
            return;
        }
    };
    cleared(id, Fault::Render);
    let (old, first, updated, scope) = with_tree(|t| {
        let Some(i) = t.get_mut(id) else {
            return (Vec::new(), true, Vec::new(), None);
        };
        let (key, place, owner) = (i.key, i.place, i.scope.id().owner_id());
        i.raw = Some(mark_root(&mut i.marked, mark_statics(raw), key, place, owner));
        i.renders = i.renders.saturating_add(1);
        i.render_scopes.push(rs);
        let excess = i.render_scopes.len().saturating_sub(KEPT_RENDERS);
        let old: Vec<Scope> = i.render_scopes.drain(..excess).collect();
        let first = i.renders == 1;
        let updated = if first { Vec::new() } else { i.updated.clone() };
        if first {
            let mounted = std::mem::take(&mut i.mounted);
            POST.with(|p| {
                p.borrow_mut().extend(
                    mounted
                        .into_iter()
                        .map(|f| (owner, crate::engine::Stored::new(crate::engine::Site::AfterLayout, f))),
                );
            });
        }
        (old, first, updated, Some(owner))
    });
    // The engine's re-run retirement (ADR-0035, D3): a check's re-render
    // of a live instance retires its old scope as the arena's transaction.
    each_to_completion(old, Scope::retire);
    if let Some(owner) = scope
        && !first
    {
        POST.with(|p| {
            p.borrow_mut().extend(updated.into_iter().map(|f| -> PostHook {
                (
                    owner,
                    crate::engine::Stored::new(crate::engine::Site::AfterLayout, Box::new(move || f())),
                )
            }));
        });
    }
    count(|s| s.renders = s.renders.saturating_add(1));
    if let Some(rendered) = rendered {
        RENDERED.with(|r| r.borrow_mut().push(rendered));
    }
}

/// The instances rendered since the last call, in order, and why.
#[must_use]
pub fn take_rendered() -> Vec<Rendered> {
    RENDERED.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Run the hooks due after the frame was laid out; whether there were any.
/// In a fork, none runs: a hook is a caller's closure, writing what it
/// captured, which no rollback puts back (ADR-0028, rule 5). The fork's
/// own hooks go with it (`POST` is Fresh).
pub fn run_post() -> bool {
    if crate::engine::forking() {
        return false;
    }
    let hooks = POST.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let any = !hooks.is_empty();
    // Every one runs, though one panics: the first panic goes on once all
    // have (taken out, a hook a panic skipped would never run); inside a
    // turn, to its arbiter as it is caught (`engine::Gather`).
    let mut all = crate::engine::Gather::new();
    for (owner, f) in hooks {
        // Its owner was disposed since it was queued (its instance
        // unmounted): cancelled.
        if !crate::reactive::is_live(owner) {
            continue;
        }
        // In the window it was queued in, whichever window's layout runs
        // the hooks (ADR-0015); never once that window closed.
        all.run(|| f.call_once(|f| f()));
    }
    all.finish();
    any
}

/// `window` closes: the hooks queued in it are cancelled (ADR-0015, the
/// close order), dropped once out of the queue, each under a catch, in its
/// close turn. How many were.
pub(crate) fn cancel_hooks(window: &crate::focus::FocusRoot) -> usize {
    let gone: Vec<PostHook> = POST
        .try_with(|p| {
            p.try_borrow_mut()
                .map(|mut p| {
                    let (gone, kept) = std::mem::take(&mut *p).into_iter().partition(|(_, f)| f.is_of(window));
                    *p = kept;
                    gone
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let n = gone.len();
    for hook in gone {
        crate::engine::contained(move || drop(hook));
    }
    n
}

fn current() -> Option<Id> {
    with_tree(|t| t.current.last().copied())
}

/// What a handler made now runs in ([`crate::Div::on_click`] and the
/// rest): the component instance rendering now, by its scope, which lives
/// as long as it does (not the render's own scope, which a later render
/// replaces); else what owns what is made now.
pub(crate) fn handler_owner() -> Option<crate::reactive::OwnerId> {
    current()
        .and_then(|id| with_tree(|t| t.get(id).map(|i| i.scope.id().owner_id())))
        .or_else(crate::reactive::owner)
}

/// Whether a cold frame runs ([`ColdFrame`]): its renders, its layout and
/// its bindings, which render every instance again to compare. For what
/// counts renders or binding runs (a test's "renders nothing"). Never in
/// a release build.
#[must_use]
pub fn is_cold_render() -> bool {
    crate::reactive::is_cold()
}

/// Run `f` once, after the next layout pass (Vue's `nextTick`): what it
/// writes shows in the same frame. For code that needs what is on screen,
/// such as a menu that opens on its first item ([`crate::Members`]).
/// Owned by what runs now (the instance rendering, or the owner a handler
/// runs in): if that is disposed before the layout, `f` never runs. Called
/// with no owner, it is a bug (a debug build panics) and `f` never runs.
pub fn after_layout(f: impl FnOnce() + 'static) {
    match Owner::current() {
        Some(owner) => owner.after_layout(f),
        None => debug_assert!(false, "gui: `after_layout` outside any owner never runs"),
    }
}

/// What code runs in now, kept to queue work for it from code that runs
/// outside it (a part's unmounting, telling the whole it belongs to): the
/// instance rendering, or the owner a handler runs in. The work runs only
/// while it is not disposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Owner(crate::reactive::OwnerId);

impl Owner {
    /// What runs now's owner, if any ([`after_layout`]'s).
    #[must_use]
    pub fn current() -> Option<Self> {
        handler_owner().map(Self)
    }

    /// [`after_layout`], for this owner: `f` never runs if it is disposed
    /// before the layout.
    pub fn after_layout(self, f: impl FnOnce() + 'static) {
        let f: Box<dyn FnOnce()> = Box::new(f);
        let f = crate::engine::Stored::new(crate::engine::Site::AfterLayout, f);
        POST.with(|p| p.borrow_mut().push((self.0, f)));
    }
}

/// Run `f` after this instance's first frame is laid out.
pub fn on_mounted(f: impl FnOnce() + 'static) {
    match current() {
        Some(id) => with_tree(|t| {
            if let Some(i) = t.get_mut(id) {
                i.mounted.push(Box::new(f));
            }
        }),
        None => eprintln!("gui: `on_mounted` outside a component's setup never runs"),
    }
}

/// Run `f` after each frame in which this instance rendered again.
pub fn on_updated(f: impl Fn() + 'static) {
    match current() {
        Some(id) => with_tree(|t| {
            if let Some(i) = t.get_mut(id) {
                i.updated.push(Rc::new(f));
            }
        }),
        None => eprintln!("gui: `on_updated` outside a component's setup never runs"),
    }
}

/// Run `f` when this instance unmounts.
#[track_caller]
pub fn on_unmounted(f: impl FnOnce() + 'static) {
    crate::reactive::on_cleanup(f);
}

/// Make `value` what [`use_context`] of its type returns in this instance and
/// below it; outside any instance, everywhere.
pub fn provide<T: 'static>(value: T) {
    #[cfg(debug_assertions)]
    crate::engine::no_component_context();
    let value: Rc<dyn Any> = Rc::new(value);
    let Some(id) = current() else {
        GLOBAL.with_borrow_mut(|g| {
            g.insert(TypeId::of::<T>(), crate::engine::AppHandle::new(value));
        });
        return;
    };
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            Rc::make_mut(&mut i.context).insert(TypeId::of::<T>(), value);
        }
    });
}

crate::engine::engine_local! {
    /// What [`provide`] gave outside any instance: app objects by handle
    /// (ADR-0034 S7, "What a check isolates").
    [Fork(log)] static GLOBAL: RefCell<crate::engine::ForkMap<TypeId, crate::engine::AppHandle<dyn Any>>> = RefCell::default();
}

/// The nearest value provided for `T` above this instance (or globally).
#[must_use]
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    #[cfg(debug_assertions)]
    crate::engine::no_component_context();
    let key = TypeId::of::<T>();
    let local = current().and_then(|id| with_tree(|t| t.get(id).and_then(|i| i.context.get(&key).cloned())));
    let any = local.or_else(|| GLOBAL.with_borrow(|g| g.get(&key).map(crate::engine::AppHandle::rc)))?;
    any.downcast_ref::<T>().cloned()
}

/// One instance as the devtools show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceInfo {
    pub name: &'static str,
    pub depth: u32,
    pub renders: u64,
    /// What its last render read: a write to one of these renders it again.
    pub reads: Vec<SignalRef>,
}

/// Every live instance, in slab order.
#[must_use]
pub fn instances() -> Vec<InstanceInfo> {
    with_tree(|t| {
        t.slab
            .iter()
            .flatten()
            .map(|i| InstanceInfo {
                name: i.name,
                depth: i.depth,
                renders: i.renders,
                reads: i.node.map(RenderNode::deps).unwrap_or_default(),
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
