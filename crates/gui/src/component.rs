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
}

/// A function a parent passes down, compared equal whatever it holds, so
/// passing a new closure each render does not render the child again; the
/// child always calls the latest one (Vue's event handlers, React's
/// `useEffectEvent`).
pub struct Callback<A: 'static, R: 'static = ()>(Rc<dyn Fn(A) -> R>);

impl<A: 'static, R: 'static> Callback<A, R> {
    pub fn new(f: impl Fn(A) -> R + 'static) -> Self {
        Self(Rc::new(f))
    }

    pub fn call(&self, arg: A) -> R {
        (self.0)(arg)
    }
}

impl<A: 'static, R: 'static> Clone for Callback<A, R> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
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
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(T) + 'static) -> Self {
        self.on_change = Some(Callback::new(f));
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
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(T) + 'static) -> Self {
        self.on_change = Some(Callback::new(f));
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
#[must_use]
#[track_caller]
pub fn use_model<P: 'static, T: Clone + PartialEq + 'static>(
    props: Signal<P>,
    pick: fn(&P) -> &Bind<T>,
) -> UseModel<P, T> {
    let first = props.with_untracked(|p| match &pick(p).source {
        Source::Model(m) => m.get_untracked(),
        Source::Value(v) | Source::Default(v) => v.clone(),
    });
    UseModel {
        props,
        pick,
        local: Signal::new(first),
    }
}

impl<P: 'static, T: Clone + PartialEq + 'static> UseModel<P, T> {
    /// The value, tracked.
    #[must_use]
    pub fn get(&self) -> T {
        let local = self.local;
        self.props.with(|p| match &(self.pick)(p).source {
            Source::Model(m) => m.get(),
            Source::Value(v) => v.clone(),
            Source::Default(_) => local.get(),
        })
    }

    /// The value, untracked.
    #[must_use]
    pub fn get_untracked(&self) -> T {
        crate::reactive::untracked(|| self.get())
    }

    /// Borrow the value, tracked.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let local = self.local;
        self.props.with(|p| match &(self.pick)(p).source {
            Source::Model(m) => m.with(f),
            Source::Value(v) => f(v),
            Source::Default(_) => local.with(f),
        })
    }

    /// Write `value`: up to the caller's model, or to the component's own;
    /// then `on_change`. Nothing happens when it equals the current value.
    pub fn set(&self, value: T) {
        let (model, on_change) = self.props.with_untracked(|p| {
            let b = (self.pick)(p);
            let model = match &b.source {
                Source::Model(m) => Some(Some(m.clone())),
                Source::Value(_) => Some(None),
                Source::Default(_) => None,
            };
            (model, b.on_change.clone())
        });
        if self.get_untracked() == value {
            return;
        }
        match model {
            Some(Some(m)) => m.set(value.clone()),
            Some(None) => {}
            None => self.local.set(value.clone()),
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
    fn setup(&self) -> (Box<dyn Any>, Render);
    /// Pass new props to a live instance; it renders again if they differ.
    fn update(&self, props: &dyn Any);
}

type Render = Box<dyn FnMut() -> Element>;

struct Pending<P, S> {
    props: P,
    setup: S,
}

impl<P, S, R> Mount for Pending<P, S>
where
    P: Clone + PartialEq + 'static,
    S: Clone + FnOnce(Ctx<P>) -> R + 'static,
    R: FnMut() -> Element + 'static,
{
    fn setup(&self) -> (Box<dyn Any>, Render) {
        let props = Signal::new(self.props.clone());
        let render = (self.setup.clone())(Ctx { props });
        (Box::new(props), Box::new(render))
    }

    fn update(&self, props: &dyn Any) {
        let Some(signal) = props.downcast_ref::<Signal<P>>() else {
            return;
        };
        if signal.with_untracked(|old| *old == self.props) {
            // Equal (a `Callback` always is): stored quietly, so the child
            // calls the latest closures without rendering again.
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
    /// Whether a frame has resolved it: met again (its parent's render
    /// reused it), its instance keeps the props it has.
    used: Cell<bool>,
    type_id: TypeId,
    /// The style its parent gives its output's root (Vue's attribute
    /// fallthrough: a parent that joins or sizes its children by position
    /// styles a component's root as it would a node's).
    pub(crate) fallthrough: Option<Fallthrough>,
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
#[must_use]
pub fn component<P, S, R>(props: P, setup: S) -> ComponentElement
where
    P: Clone + PartialEq + 'static,
    S: Clone + FnOnce(Ctx<P>) -> R + 'static,
    R: FnMut() -> Element + 'static,
{
    ComponentElement {
        key: None,
        name: short_name::<S>(),
        type_id: TypeId::of::<S>(),
        mount: Box::new(Pending { props, setup }),
        used: Cell::new(false),
        fallthrough: None,
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
struct Place {
    path: u64,
    type_id: TypeId,
}

type Id = usize;

type Context = Rc<HashMap<TypeId, Rc<dyn Any>>>;

struct Instance {
    generation: u32,
    parent: Option<Id>,
    depth: u32,
    name: &'static str,
    scope: Scope,
    node: Option<RenderNode>,
    props: Box<dyn Any>,
    render: Option<Render>,
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
    children: HashMap<Place, Id>,
    /// Its output as its parent's fallthrough styled it: the output it was
    /// made from, the fallthrough, and the result, so a clean instance
    /// gives back the same styled node.
    styled: Option<(Element, Fallthrough, Element)>,
    context: Context,
    mounted: Vec<Box<dyn FnOnce()>>,
    updated: Vec<Rc<dyn Fn()>>,
    renders: u64,
}

#[derive(Default)]
struct Tree {
    slab: Vec<Option<Instance>>,
    generations: Vec<u32>,
    free: Vec<Id>,
    /// The instance whose setup or render runs now.
    current: Vec<Id>,
}

/// A render node's mark: instance, generation, the write that woke it, its lane.
type Mark = (Id, u32, Option<SignalRef>, Priority);

thread_local! {
    static TREE: RefCell<Tree> = RefCell::new(Tree::default());
    /// Instances whose render was marked since the last resolve: their
    /// generation and the signal whose write marked them.
    static MARKED: RefCell<Vec<Mark>> = const { RefCell::new(Vec::new()) };
    /// Idle instances left dirty by the last resolve (its budget ran out).
    static DEFERRED: RefCell<Vec<(Id, u32)>> = const { RefCell::new(Vec::new()) };
    /// The priority writes made now mark renders with.
    static PRIORITY: Cell<Priority> = const { Cell::new(Priority::Input) };
    /// Until when the resolve running now may render idle instances.
    static DEADLINE: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
    /// Hooks due after the frame is laid out (`on_mounted`, `on_updated`).
    static POST: RefCell<Vec<Box<dyn FnOnce()>>> = RefCell::new(Vec::new());
    /// Something outside a render that the frame read changed.
    static FRAME_DUE: Cell<bool> = const { Cell::new(false) };
    /// The instances rendered since the last [`take_rendered`].
    static RENDERED: RefCell<Vec<Rendered>> = const { RefCell::new(Vec::new()) };
    static STATS: Cell<Stats> = const { Cell::new(Stats { renders: 0, setups: 0, unmounts: 0 }) };
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

thread_local! {
    /// How long a frame may spend on idle renders.
    static IDLE_BUDGET: Cell<std::time::Duration> = const { Cell::new(std::time::Duration::from_millis(4)) };
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
    /// Every instance rendered again (a cold frame).
    All,
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mount => f.write_str("mount"),
            Self::Signal(s) => write!(f, "{s}"),
            Self::Props => f.write_str("props"),
            Self::All => f.write_str("all"),
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

thread_local! {
    static BINDINGS: Cell<u64> = const { Cell::new(0) };
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
    TREE.with(|t| f(&mut t.borrow_mut()))
}

impl Tree {
    fn get(&self, id: Id) -> Option<&Instance> {
        self.slab.get(id).and_then(Option::as_ref)
    }

    fn get_mut(&mut self, id: Id) -> Option<&mut Instance> {
        self.slab.get_mut(id).and_then(Option::as_mut)
    }

    fn insert(&mut self, inst: Instance) -> Id {
        if let Some(id) = self.free.pop() {
            let generation = self.generations.get(id).copied().unwrap_or(0);
            if let Some(s) = self.slab.get_mut(id) {
                *s = Some(Instance { generation, ..inst });
            }
            id
        } else {
            self.slab.push(Some(inst));
            self.generations.push(0);
            self.slab.len().saturating_sub(1)
        }
    }

    fn generation(&self, id: Id) -> u32 {
        self.generations.get(id).copied().unwrap_or(0)
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
pub struct Root {
    id: Id,
}

impl Root {
    /// A root instance rendering `render` (owned by nothing).
    pub fn new(render: impl FnMut() -> Element + 'static) -> Self {
        let scope = Scope::detached();
        let id = create(None, scope, Box::new(()), Box::new(render), "app");
        Self { id }
    }

    /// Expand the tree: render what is dirty, reuse what is not. The root's
    /// output with every placeholder replaced.
    pub fn resolve(&self) -> Element {
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
        let out = resolve(self.id);
        DEADLINE.set(None);
        out
    }

    /// The output with every instance rendered again, as a cold build gives
    /// it (the debug check that incremental equals cold). Hooks the extra
    /// renders queue are dropped, and they are not counted or logged as
    /// renders: the check leaves the frame's report as it was.
    pub fn resolve_cold(&self) -> Element {
        let queued = POST.with(|p| p.borrow().len());
        let logged = RENDERED.with(|r| r.borrow().len());
        let stats = STATS.get();
        self.invalidate_all();
        let out = self.resolve();
        POST.with(|p| p.borrow_mut().truncate(queued));
        RENDERED.with(|r| r.borrow_mut().truncate(logged));
        STATS.set(stats);
        out
    }

    /// Mark every instance to render again (a frame built cold).
    pub fn invalidate_all(&self) {
        with_tree(|t| {
            let n = t.slab.len();
            for id in 0..n {
                let generation = t.get(id).map(|i| i.generation);
                if let Some(generation) = generation {
                    t.mark(id, generation, Why::All, Priority::Input);
                }
            }
        });
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            unmount(self.id);
        }
    }
}

/// Make an instance (its setup runs in [`mount_child`]).
fn create(parent: Option<Id>, scope: Scope, props: Box<dyn Any>, render: Render, name: &'static str) -> Id {
    let (depth, context) = with_tree(|t| {
        parent.and_then(|p| t.get(p)).map_or((0, Context::default()), |p| {
            (p.depth.saturating_add(1), Rc::clone(&p.context))
        })
    });
    let (id, generation) = with_tree(|t| {
        let id = t.insert(Instance {
            generation: 0,
            parent,
            depth,
            name,
            scope,
            node: None,
            props,
            render: Some(render),
            render_scopes: Vec::new(),
            dirty: true,
            why: Why::Mount,
            lane: Priority::Input,
            below: false,
            raw: None,
            output: None,
            children: HashMap::new(),
            styled: None,
            context,
            mounted: Vec::new(),
            updated: Vec::new(),
            renders: 0,
        });
        (id, t.generation(id))
    });
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

/// Mount a child from `mount` under `parent`.
fn mount_child(parent: Id, mount: &dyn Mount, name: &'static str) -> Id {
    let scope = with_tree(|t| t.get(parent).map(|p| p.scope));
    let scope = scope.map_or_else(Scope::detached, |s| s.run(Scope::new));
    // Setup runs as the instance, so `provide`, hooks and context find it;
    // it is pushed before it exists and filled in after.
    let id = create(Some(parent), scope, Box::new(()), Box::new(Element::default), name);
    with_tree(|t| t.current.push(id));
    let guard = PopCurrent;
    let (props, render) = scope.run(|| mount.setup());
    drop(guard);
    with_tree(|t| {
        if let Some(inst) = t.get_mut(id) {
            inst.props = props;
            inst.render = Some(render);
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
    let Some(inst) = with_tree(|t| {
        let inst = t.slab.get_mut(id).and_then(Option::take)?;
        if let Some(g) = t.generations.get_mut(id) {
            *g = g.wrapping_add(1);
        }
        t.free.push(id);
        Some(inst)
    }) else {
        return;
    };
    for child in inst.children.values() {
        unmount(*child);
    }
    for s in inst.render_scopes {
        s.dispose();
    }
    // `on_unmounted` and `on_cleanup` run as the scope is disposed.
    inst.scope.dispose();
    count(|s| s.unmounts = s.unmounts.saturating_add(1));
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
    let raw = with_tree(|t| t.get_mut(id).and_then(|i| i.raw.take()));
    let Some(raw) = raw else {
        return Element::Empty;
    };
    let before: HashMap<Place, Id> = with_tree(|t| t.get(id).map(|i| i.children.clone()).unwrap_or_default());
    let mut seen: HashMap<Place, Id> = HashMap::with_capacity(before.len());
    let out = expand(id, &raw, 0, &before, &mut seen).unwrap_or_else(|| raw.clone());
    for (slot, child) in &before {
        if seen.get(slot) != Some(child) {
            unmount(*child);
        }
    }
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            i.raw = Some(raw);
            i.children = seen;
            i.output = Some(out.clone());
            i.below = false;
        }
    });
    out
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
    before: &HashMap<Place, Id>,
    seen: &mut HashMap<Place, Id>,
) -> Option<Element> {
    match raw {
        Element::Component(c) => {
            let slot = Place {
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
            // A placeholder met before (its parent's render reused it)
            // passes nothing new; one whose instance left mounts again.
            let fresh = !c.used.replace(true);
            let child = match before.get(&slot) {
                Some(&child) if fresh => {
                    // Taken out while new props are passed: writing them
                    // runs reactive code that may read the tree.
                    let props = with_tree(|t| t.get_mut(child).map(|i| std::mem::replace(&mut i.props, Box::new(()))));
                    if let Some(props) = props {
                        c.mount.update(&*props);
                        with_tree(|t| {
                            if let Some(i) = t.get_mut(child) {
                                i.props = props;
                            }
                        });
                    }
                    child
                }
                Some(&child) => child,
                None => mount_child(parent, &*c.mount, c.name),
            };
            seen.insert(slot, child);
            apply_marks();
            let out = resolve(child);
            Some(match &c.fallthrough {
                Some(f) => fall_through(child, out, f),
                None => out,
            })
        }
        Element::Div(r) => {
            let mut kids: Option<Vec<Element>> = None;
            for (i, rc) in r.kind.iter().enumerate() {
                let step = child_step(rc, i);
                if let Some(new) = expand(parent, rc, path_step(path, step), before, seen)
                    && let Some(at) = kids.get_or_insert_with(|| r.kind.clone()).get_mut(i)
                {
                    *at = new;
                }
            }
            kids.map(|kind| {
                Element::Div(Rc::new(crate::element::Node {
                    common: r.common.clone(),
                    kind,
                }))
            })
        }
        // A layer adds no step: what moves between its place in the flow and
        // a floating layer keeps its instances (Vue's `Teleport`).
        Element::Deferred(r) => expand(parent, &r.child, path, before, seen).map(|child| {
            Element::Deferred(Rc::new(crate::element::Deferred {
                placement: r.placement,
                child,
            }))
        }),
        _ => None,
    }
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
    let Some((f, node, scope, rendered)) = with_tree(|t| {
        let i = t.get_mut(id)?;
        i.dirty = false;
        let rendered = Rendered {
            name: i.name,
            why: i.why,
        };
        Some((i.render.take()?, i.node?, i.scope, Some(rendered)))
    }) else {
        return;
    };
    let rs = scope.run(Scope::new);
    with_tree(|t| t.current.push(id));
    let pop = PopCurrent;
    let mut holder = Holder { id, f: Some(f) };
    let raw = rs.run(|| node.track(|| holder.f.as_mut().map_or(Element::Empty, |f| f())));
    drop(pop);
    drop(holder);
    let (old, first, updated) = with_tree(|t| {
        let Some(i) = t.get_mut(id) else {
            return (Vec::new(), true, Vec::new());
        };
        i.raw = Some(raw);
        i.renders = i.renders.saturating_add(1);
        i.render_scopes.push(rs);
        let excess = i.render_scopes.len().saturating_sub(KEPT_RENDERS);
        let old: Vec<Scope> = i.render_scopes.drain(..excess).collect();
        let first = i.renders == 1;
        let updated = if first { Vec::new() } else { i.updated.clone() };
        if first {
            let mounted = std::mem::take(&mut i.mounted);
            POST.with(|p| p.borrow_mut().extend(mounted));
        }
        (old, first, updated)
    });
    for s in old {
        s.dispose();
    }
    if !first {
        POST.with(|p| {
            p.borrow_mut().extend(
                updated
                    .into_iter()
                    .map(|f| -> Box<dyn FnOnce()> { Box::new(move || f()) }),
            );
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

/// Puts an instance's render back, on return and on unwind (a render that
/// panicked renders again next time).
struct Holder {
    id: Id,
    f: Option<Render>,
}

impl Drop for Holder {
    fn drop(&mut self) {
        let Some(f) = self.f.take() else { return };
        let id = self.id;
        TREE.with(|t| {
            if let Ok(mut t) = t.try_borrow_mut()
                && let Some(i) = t.get_mut(id)
            {
                i.render = Some(f);
            }
        });
    }
}

/// Run the hooks due after the frame was laid out; whether there were any.
pub fn run_post() -> bool {
    let hooks = POST.with(|p| std::mem::take(&mut *p.borrow_mut()));
    let any = !hooks.is_empty();
    for f in hooks {
        f();
    }
    any
}

fn current() -> Option<Id> {
    with_tree(|t| t.current.last().copied())
}

/// Run `f` once, after the next layout pass (Vue's `nextTick`): what it
/// writes shows in the same frame. For code that needs what is on screen,
/// such as a menu that opens on its first item ([`crate::Members`]).
pub fn after_layout(f: impl FnOnce() + 'static) {
    POST.with(|p| p.borrow_mut().push(Box::new(f)));
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
    let value: Rc<dyn Any> = Rc::new(value);
    let Some(id) = current() else {
        GLOBAL.with(|g| g.borrow_mut().insert(TypeId::of::<T>(), value));
        return;
    };
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            Rc::make_mut(&mut i.context).insert(TypeId::of::<T>(), value);
        }
    });
}

thread_local! {
    static GLOBAL: RefCell<HashMap<TypeId, Rc<dyn Any>>> = RefCell::new(HashMap::new());
}

/// The nearest value provided for `T` above this instance (or globally).
#[must_use]
pub fn use_context<T: Clone + 'static>() -> Option<T> {
    let key = TypeId::of::<T>();
    let local = current().and_then(|id| with_tree(|t| t.get(id).and_then(|i| i.context.get(&key).cloned())));
    let any = local.or_else(|| GLOBAL.with(|g| g.borrow().get(&key).cloned()))?;
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
