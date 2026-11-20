//! Fine-grained reactive primitives: Signal, Effect, and batch.
//!
//! Inspired by Vue 3's reactivity core. Key properties:
//! - Only the signals an effect actually reads subscribe it.
//! - Two-phase batch: dirty marking → effect flush, so an effect never
//!   sees intermediate state from a batch of writes.
//! - Single-threaded (UI thread). No Send/Sync.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::rc::{Rc, Weak};

pub(crate) type NodeId = u64;

// --- Thread-local runtime ---

thread_local! {
    static NEXT_ID: Cell<NodeId> = const { Cell::new(0) };
    /// The effect currently running; signals subscribe it on read.
    pub(crate) static ACTIVE: RefCell<Option<NodeId>> = const { RefCell::new(None) };
    /// Weak refs to all live effects, keyed by id.
    pub(crate) static REGISTRY: RefCell<HashMap<NodeId, Weak<EffectInner>>> = RefCell::new(HashMap::new());
    /// Nesting depth of batch() calls.
    static BATCH_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Effects scheduled to run when the outermost batch ends.
    pub(crate) static PENDING: RefCell<Vec<NodeId>> = const { RefCell::new(Vec::new()) };
    /// What the reactive runtime did since tracing started; `None` when off.
    static TRACE: RefCell<Option<ReactiveTrace>> = const { RefCell::new(None) };
    /// Set by every signal write; the window reads it to know a frame is due.
    static CHANGED: Cell<bool> = const { Cell::new(false) };
    /// Signals alive.
    static SIGNALS: Cell<usize> = const { Cell::new(0) };
    /// Computed values alive (each also holds a bridge in `REGISTRY`).
    pub(crate) static COMPUTEDS: Cell<usize> = const { Cell::new(0) };
    /// Subscriptions held: an effect or computed value on a source it read.
    static SUBSCRIPTIONS: Cell<usize> = const { Cell::new(0) };
    /// The labeled signals, for the devtools to read and write by label.
    static PROBES: RefCell<Vec<Box<dyn Probe>>> = const { RefCell::new(Vec::new()) };
}

type Count = std::thread::LocalKey<Cell<usize>>;

/// Add `n` to a live count.
pub(crate) fn count_up(key: &'static Count, n: usize) {
    key.with(|c| c.set(c.get().saturating_add(n)));
}

/// Take `n` from a live count; nothing once the thread's locals are gone (a
/// value dropped while the thread ends).
pub(crate) fn count_down(key: &'static Count, n: usize) {
    key.try_with(|c| c.set(c.get().saturating_sub(n))).unwrap_or(());
}

/// The reactive graph alive on this thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub signals: usize,
    pub computeds: usize,
    pub effects: usize,
    /// Edges from a source to what read it in its last run.
    pub subscriptions: usize,
}

/// The reactive graph alive on this thread now.
#[must_use]
pub fn graph() -> Graph {
    let computeds = COMPUTEDS.with(Cell::get);
    Graph {
        signals: SIGNALS.with(Cell::get),
        computeds,
        effects: REGISTRY.with(|r| r.borrow().len()).saturating_sub(computeds),
        subscriptions: SUBSCRIPTIONS.with(Cell::get),
    }
}

// --- Dependencies ---

/// What an effect or computed value reads: it forgets the reader when the
/// reader runs again (that run subscribes to what it reads then) or is dropped.
pub(crate) trait Source {
    fn unsubscribe(&self, id: NodeId);
}

/// Subscribe the running effect or computed value, if any, to the source
/// whose subscribers are `subs`; `source` names it for the reader to forget.
pub(crate) fn track(subs: &RefCell<HashSet<NodeId>>, source: impl FnOnce() -> Weak<dyn Source>) {
    let Some(id) = ACTIVE.with(|a| *a.borrow()) else {
        return;
    };
    if !subs.borrow_mut().insert(id) {
        return;
    }
    count_up(&SUBSCRIPTIONS, 1);
    if let Some(reader) = REGISTRY.with(|r| r.borrow().get(&id).and_then(Weak::upgrade)) {
        reader.sources.borrow_mut().push(source());
    }
}

/// Drop `id` from `subs`, counting the subscription gone.
pub(crate) fn untrack(subs: &RefCell<HashSet<NodeId>>, id: NodeId) {
    if subs.borrow_mut().remove(&id) {
        count_down(&SUBSCRIPTIONS, 1);
    }
}

/// Count the subscriptions of a source being dropped as gone.
pub(crate) fn drop_subs(subs: &RefCell<HashSet<NodeId>>) {
    count_down(&SUBSCRIPTIONS, subs.borrow().len());
}

// --- Tracing ---

/// A signal as it appears in a trace: its id and the label it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalRef {
    pub id: u64,
    pub label: Option<&'static str>,
}

impl std::fmt::Display for SignalRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.label {
            Some(label) => write!(f, "{label}"),
            None => write!(f, "signal#{}", self.id),
        }
    }
}

/// What the reactive runtime did between [`start_trace`] and [`take_trace`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReactiveTrace {
    /// Signals written, in write order (a signal written twice appears twice).
    pub writes: Vec<SignalRef>,
    /// Distinct signals read, in first-read order.
    pub reads: Vec<SignalRef>,
    /// Number of effect runs.
    pub effect_runs: u32,
}

/// Start recording signal writes, reads and effect runs on this thread,
/// discarding anything recorded before.
pub fn start_trace() {
    TRACE.with(|t| *t.borrow_mut() = Some(ReactiveTrace::default()));
}

/// Return what was recorded since the last [`start_trace`] or `take_trace`
/// and keep recording; empty when tracing is off.
pub fn take_trace() -> ReactiveTrace {
    TRACE.with(|t| t.borrow_mut().as_mut().map(std::mem::take).unwrap_or_default())
}

/// Whether any signal was written since the last call; clears the flag.
pub fn take_changed() -> bool {
    CHANGED.with(|c| c.replace(false))
}

/// Stop recording on this thread.
pub fn stop_trace() {
    TRACE.with(|t| *t.borrow_mut() = None);
}

fn record(f: impl FnOnce(&mut ReactiveTrace)) {
    TRACE.with(|t| {
        if let Some(trace) = t.borrow_mut().as_mut() {
            f(trace);
        }
    });
}

pub(crate) fn alloc_id() -> NodeId {
    NEXT_ID.with(|n| {
        let id = n.get();
        n.set(id.saturating_add(1));
        id
    })
}

fn start_batch() {
    BATCH_DEPTH.with(|d| d.set(d.get().saturating_add(1)));
}

fn end_batch() {
    let depth = BATCH_DEPTH.with(Cell::get);
    BATCH_DEPTH.with(|d| d.set(depth.saturating_sub(1)));
    if depth == 1 {
        flush();
    }
}

/// Drain the pending queue until quiescent.
/// Effects triggered by effects are picked up in the next loop iteration.
fn flush() {
    loop {
        let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
        if pending.is_empty() {
            break;
        }
        let mut seen = HashSet::new();
        for id in pending {
            if seen.insert(id)
                && let Some(inner) = REGISTRY.with(|r| r.borrow().get(&id).and_then(|w| w.upgrade()))
                && inner.dirty.get()
            {
                run_effect(&inner);
            }
        }
    }
}

fn run_effect(inner: &Rc<EffectInner>) {
    record(|t| t.effect_runs = t.effect_runs.saturating_add(1));
    inner.dirty.set(false);
    inner.forget_sources();
    let prev = ACTIVE.with(|a| *a.borrow());
    ACTIVE.with(|a| *a.borrow_mut() = Some(inner.id));
    (inner.f.borrow())();
    ACTIVE.with(|a| *a.borrow_mut() = prev);
}

// --- Signal ---

struct SignalInner<T> {
    id: NodeId,
    label: Option<&'static str>,
    value: RefCell<T>,
    /// Ids of effects that read this signal in their last run.
    subs: RefCell<HashSet<NodeId>>,
}

/// A reactive cell. Reading it inside an [`Effect`] creates a subscription;
/// writing it schedules all subscribers for re-run.
pub struct Signal<T>(Rc<SignalInner<T>>);

impl<T: Clone + fmt::Debug + 'static> Signal<T> {
    /// A signal that traces and debug reports show by `label` instead of its
    /// id, and whose value the devtools read ([`probes`]) and, for a plain
    /// value (a number, `bool`, `char` or `String`), write.
    pub fn labeled(label: &'static str, value: T) -> Self {
        let signal = Self::create(value, Some(label));
        let probe: Box<dyn Probe> = Box::new(Rc::downgrade(&signal.0));
        PROBES.with(|p| {
            let mut p = p.borrow_mut();
            // Forget the signals dropped since, so the list follows what lives.
            p.retain(|probe| probe.alive());
            p.push(probe);
        });
        signal
    }
}

impl<T: Clone + 'static> Signal<T> {
    pub fn new(value: T) -> Self {
        Self::create(value, None)
    }

    fn create(value: T, label: Option<&'static str>) -> Self {
        count_up(&SIGNALS, 1);
        Self(Rc::new(SignalInner {
            id: alloc_id(),
            label,
            value: RefCell::new(value),
            subs: RefCell::new(HashSet::new()),
        }))
    }

    fn as_ref(&self) -> SignalRef {
        SignalRef {
            id: self.0.id,
            label: self.0.label,
        }
    }

    /// Read the current value, tracking any active effect as a subscriber.
    pub fn get(&self) -> T {
        let me = self.as_ref();
        record(|t| {
            if !t.reads.contains(&me) {
                t.reads.push(me);
            }
        });
        track(&self.0.subs, || -> Weak<dyn Source> {
            Rc::<SignalInner<T>>::downgrade(&self.0)
        });
        self.0.value.borrow().clone()
    }

    /// Write a new value and queue all subscribers to re-run.
    pub fn set(&self, value: T) {
        let me = self.as_ref();
        record(|t| t.writes.push(me));
        CHANGED.with(|c| c.set(true));
        *self.0.value.borrow_mut() = value;
        self.notify();
    }

    /// Apply a function to the current value and write the result.
    pub fn update(&self, f: impl FnOnce(T) -> T) {
        let new_val = {
            let cur = self.0.value.borrow().clone();
            f(cur)
        };
        self.set(new_val);
    }

    fn notify(&self) {
        let subs: Vec<NodeId> = self.0.subs.borrow().iter().copied().collect();

        // Mark dirty before entering the batch so effects see the right state
        // even if flush happens synchronously in nested code.
        for id in &subs {
            let inner = REGISTRY.with(|r| r.borrow().get(id).and_then(|w| w.upgrade()));
            if let Some(inner) = inner {
                inner.dirty.set(true);
            }
        }

        start_batch();
        PENDING.with(|p| p.borrow_mut().extend(subs));
        end_batch();
    }
}

impl<T> Source for SignalInner<T> {
    fn unsubscribe(&self, id: NodeId) {
        untrack(&self.subs, id);
    }
}

impl<T> Drop for SignalInner<T> {
    fn drop(&mut self) {
        count_down(&SIGNALS, 1);
        drop_subs(&self.subs);
    }
}

/// Its label (or id) and value, read without subscribing anything.
impl<T: fmt::Debug> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let me = SignalRef {
            id: self.0.id,
            label: self.0.label,
        };
        write!(f, "Signal({me}: {:?})", self.0.value.borrow())
    }
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

// --- Probes ---

/// A labeled signal as the devtools see it, whatever its type.
trait Probe {
    fn alive(&self) -> bool;
    fn snapshot(&self) -> Option<SignalValue>;
    /// Write `text`, parsed as the signal's type; `None` when it is gone.
    fn write(&self, text: &str) -> Option<Result<(), String>>;
}

impl<T: Clone + fmt::Debug + 'static> Probe for Weak<SignalInner<T>> {
    fn alive(&self) -> bool {
        self.strong_count() > 0
    }

    fn snapshot(&self) -> Option<SignalValue> {
        let inner = self.upgrade()?;
        let value = format!("{:?}", inner.value.borrow());
        let subscribers = inner.subs.borrow().len();
        Some(SignalValue {
            id: inner.id,
            label: inner.label.unwrap_or_default(),
            type_name: std::any::type_name::<T>(),
            value,
            subscribers,
        })
    }

    fn write(&self, text: &str) -> Option<Result<(), String>> {
        let inner = self.upgrade()?;
        Some(parse_as::<T>(text).map(|value| Signal(inner).set(value)))
    }
}

/// `text` as a `T`, for the plain types a command line can spell.
fn parse_as<T: 'static>(text: &str) -> Result<T, String> {
    fn parsed<V: std::str::FromStr<Err: fmt::Display> + 'static>(text: &str) -> Result<Box<dyn Any>, String> {
        text.parse::<V>()
            .map(|v| -> Box<dyn Any> { Box::new(v) })
            .map_err(|e| format!("{text:?} is not a {}: {e}", std::any::type_name::<V>()))
    }
    let t = TypeId::of::<T>();
    let value = if t == TypeId::of::<String>() {
        // Quoted as the value is shown, or bare.
        let bare = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(text);
        let value: Box<dyn Any> = Box::new(bare.to_owned());
        Ok(value)
    } else if t == TypeId::of::<bool>() {
        parsed::<bool>(text)
    } else if t == TypeId::of::<char>() {
        parsed::<char>(text)
    } else if t == TypeId::of::<i32>() {
        parsed::<i32>(text)
    } else if t == TypeId::of::<i64>() {
        parsed::<i64>(text)
    } else if t == TypeId::of::<u8>() {
        parsed::<u8>(text)
    } else if t == TypeId::of::<u16>() {
        parsed::<u16>(text)
    } else if t == TypeId::of::<u32>() {
        parsed::<u32>(text)
    } else if t == TypeId::of::<u64>() {
        parsed::<u64>(text)
    } else if t == TypeId::of::<usize>() {
        parsed::<usize>(text)
    } else if t == TypeId::of::<f32>() {
        parsed::<f32>(text)
    } else if t == TypeId::of::<f64>() {
        parsed::<f64>(text)
    } else {
        Err(format!(
            "a {} cannot be written from the command line (numbers, bool, char and String can)",
            std::any::type_name::<T>()
        ))
    }?;
    value
        .downcast::<T>()
        .map(|b| *b)
        .map_err(|_| "the parsed value has another type".to_owned())
}

/// A labeled signal's value at one moment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalValue {
    pub id: u64,
    pub label: &'static str,
    pub type_name: &'static str,
    /// Its value as `{:?}` prints it.
    pub value: String,
    /// Effects and computed values that read it in their last run.
    pub subscribers: usize,
}

/// Every labeled signal alive on this thread, oldest first.
#[must_use]
pub fn probes() -> Vec<SignalValue> {
    PROBES.with(|p| p.borrow().iter().filter_map(|probe| probe.snapshot()).collect())
}

/// Write `text` into the labeled signal `id` as a click would: subscribers
/// run and the next frame shows it. `Err` names why not (gone, a type the
/// command line cannot spell, a value that does not parse).
pub fn write_probe(id: u64, text: &str) -> Result<(), String> {
    // Parse and write outside the borrow: the write runs effects, which may
    // label new signals.
    let probe = PROBES.with(|p| {
        p.borrow()
            .iter()
            .position(|probe| probe.snapshot().is_some_and(|v| v.id == id))
    });
    let Some(at) = probe else {
        return Err(format!("no labeled signal #{id}"));
    };
    let taken = PROBES.with(|p| {
        let mut p = p.borrow_mut();
        (at < p.len()).then(|| p.remove(at))
    });
    let Some(probe) = taken else {
        return Err(format!("no labeled signal #{id}"));
    };
    let result = probe
        .write(text)
        .unwrap_or_else(|| Err(format!("signal #{id} was dropped")));
    PROBES.with(|p| {
        let mut p = p.borrow_mut();
        let at = at.min(p.len());
        p.insert(at, probe);
    });
    result
}

// --- Effect ---

pub(crate) struct EffectInner {
    pub(crate) id: NodeId,
    pub(crate) f: RefCell<Box<dyn Fn()>>,
    pub(crate) dirty: Cell<bool>,
    /// What its last run read.
    sources: RefCell<Vec<Weak<dyn Source>>>,
}

impl EffectInner {
    /// An effect running `f` under `id`, registered for its sources to wake it.
    pub(crate) fn register(id: NodeId, f: Box<dyn Fn()>) -> Rc<Self> {
        let inner = Rc::new(Self {
            id,
            f: RefCell::new(f),
            dirty: Cell::new(false),
            sources: RefCell::new(Vec::new()),
        });
        REGISTRY.with(|r| r.borrow_mut().insert(id, Rc::downgrade(&inner)));
        inner
    }

    /// Unsubscribe from what the last run read, before a run reads again.
    pub(crate) fn forget_sources(&self) {
        for source in self.sources.take() {
            if let Some(source) = source.upgrade() {
                source.unsubscribe(self.id);
            }
        }
    }
}

impl Drop for EffectInner {
    fn drop(&mut self) {
        self.forget_sources();
        REGISTRY
            .try_with(|r| drop(r.borrow_mut().remove(&self.id)))
            .unwrap_or(());
    }
}

/// A reactive side effect. Re-runs automatically whenever any [`Signal`]
/// it read in its previous run is updated.
///
/// Dropping an `Effect` stops it and unregisters it.
pub struct Effect(Rc<EffectInner>);

impl Effect {
    /// Create and immediately run the effect.
    pub fn new(f: impl Fn() + 'static) -> Self {
        let this = Self(EffectInner::register(alloc_id(), Box::new(f)));
        run_effect(&this.0);
        this
    }
}

// --- Batch ---

/// Run `f` as a single batch: signal writes inside it mark effects dirty but
/// do not flush until `f` returns, coalescing multiple writes into one pass.
pub fn batch(f: impl FnOnce()) {
    start_batch();
    f();
    end_batch();
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn effect_runs_on_signal_change() {
        let count = Signal::new(0_i32);
        let runs = Rc::new(Cell::new(0_u32));

        let runs_clone = Rc::clone(&runs);
        let count_clone = count.clone();
        let _e = Effect::new(move || {
            let _ = count_clone.get();
            runs_clone.set(runs_clone.get().saturating_add(1));
        });

        assert_eq!(runs.get(), 1);
        count.set(1);
        assert_eq!(runs.get(), 2);
        count.set(2);
        assert_eq!(runs.get(), 3);
    }

    #[test]
    fn batch_coalesces_writes() {
        let a = Signal::new(0_i32);
        let b = Signal::new(0_i32);
        let runs = Rc::new(Cell::new(0_u32));

        let runs_clone = Rc::clone(&runs);
        let a_clone = a.clone();
        let b_clone = b.clone();
        let _e = Effect::new(move || {
            let _ = a_clone.get();
            let _ = b_clone.get();
            runs_clone.set(runs_clone.get().saturating_add(1));
        });

        assert_eq!(runs.get(), 1);
        batch(|| {
            a.set(1);
            b.set(1);
        });
        // Both writes coalesced → effect runs exactly once more.
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn trace_names_what_was_written_and_read() {
        let tab = Signal::labeled("tab", 0_i32);
        let other = Signal::new(0_i32);
        start_trace();
        let _ = tab.get();
        let _ = tab.get();
        other.set(1);
        let trace = take_trace();
        stop_trace();
        assert_eq!(trace.reads.iter().map(ToString::to_string).collect::<Vec<_>>(), ["tab"]);
        assert_eq!(trace.writes.len(), 1);
        assert!(trace.writes.first().is_some_and(|w| w.label.is_none()));
    }

    #[test]
    fn an_effect_wakes_only_for_what_its_last_run_read() {
        let gate = Signal::new(true);
        let early = Signal::new(0_i32);
        let runs = Rc::new(Cell::new(0_u32));
        let (g, e, r) = (gate.clone(), early.clone(), Rc::clone(&runs));
        let effect = Effect::new(move || {
            if g.get() {
                let _ = e.get();
            }
            r.set(r.get().saturating_add(1));
        });
        gate.set(false);
        assert_eq!(runs.get(), 2);
        early.set(1);
        assert_eq!(runs.get(), 2, "the last run did not read `early`");
        assert_eq!(graph().subscriptions, 1, "only `gate` holds it");
        drop(effect);
        assert_eq!(graph().subscriptions, 0, "a dropped effect leaves no subscription");
    }

    #[test]
    fn dropped_effect_stops_reacting() {
        let s = Signal::new(0_i32);
        let runs = Rc::new(Cell::new(0_u32));

        let runs_clone = Rc::clone(&runs);
        let s_clone = s.clone();
        let e = Effect::new(move || {
            let _ = s_clone.get();
            runs_clone.set(runs_clone.get().saturating_add(1));
        });

        assert_eq!(runs.get(), 1);
        drop(e);
        s.set(1);
        assert_eq!(runs.get(), 1); // no more runs after drop
    }
}
