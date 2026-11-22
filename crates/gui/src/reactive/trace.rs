//! What the devtools see of the graph: traces of reads and writes, the size
//! of the graph, and labeled signals read and written by name (ADR-0010).

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::fmt;

use super::Signal;
use super::runtime::{self, NodeId};

/// A signal as it appears in a trace: its id and the label it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalRef {
    pub id: u64,
    pub label: Option<&'static str>,
}

impl fmt::Display for SignalRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    runtime::with(|rt| rt.trace = Some(ReactiveTrace::default()));
}

/// What was recorded since the last [`start_trace`] or `take_trace`; it keeps
/// recording. Empty when tracing is off.
pub fn take_trace() -> ReactiveTrace {
    runtime::with(|rt| rt.trace.as_mut().map(std::mem::take).unwrap_or_default())
}

/// Stop recording on this thread.
pub fn stop_trace() {
    runtime::with(|rt| rt.trace = None);
}

/// The reactive graph alive on this thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Graph {
    pub signals: usize,
    pub computeds: usize,
    pub effects: usize,
    pub scopes: usize,
    /// Component renders (ADR-0013).
    pub renders: usize,
    /// Links: a dependency to what read it in its last run, and an owner to
    /// the effects and scopes it owns.
    pub subscriptions: usize,
}

/// The reactive graph alive on this thread now.
#[must_use]
pub fn graph() -> Graph {
    runtime::with(|rt| Graph {
        signals: rt.counts.signals,
        computeds: rt.counts.computeds,
        effects: rt.counts.effects,
        scopes: rt.counts.scopes,
        renders: rt.counts.renders,
        subscriptions: rt.counts.links,
    })
}

/// The bytes this thread's reactive arena holds (nodes, links, queues; the
/// values signals hold aside): the reactive owner in the memory report.
#[must_use]
pub fn held_bytes() -> u64 {
    runtime::with(|rt| rt.held_bytes())
}

// --- Probes ---

/// A labeled signal as the devtools see it, whatever its type: two plain
/// functions made for its type, no allocation per signal.
type Snapshot = fn(NodeId) -> Option<SignalValue>;

struct Probe {
    id: NodeId,
    snapshot: Snapshot,
    write: fn(NodeId, &str) -> Option<Result<(), String>>,
}

thread_local! {
    static PROBES: RefCell<Vec<Probe>> = const { RefCell::new(Vec::new()) };
    /// The length at which the list is next swept of disposed signals:
    /// twice what lived at the last sweep, so labelling stays linear.
    static SWEEP_AT: std::cell::Cell<usize> = const { std::cell::Cell::new(64) };
}

pub(crate) fn register_probe<T: fmt::Debug + 'static>(id: NodeId) {
    PROBES.with(|p| {
        let mut p = p.borrow_mut();
        if p.len() >= SWEEP_AT.get() {
            // Forget the signals disposed since, so the list follows what lives.
            runtime::with(|rt| p.retain(|probe| rt.live(probe.id).is_some()));
            SWEEP_AT.set(p.len().saturating_mul(2).max(64));
        }
        p.push(Probe {
            id,
            snapshot: snapshot_of::<T>,
            write: write_of::<T>,
        });
    });
}

fn snapshot_of<T: fmt::Debug + 'static>(id: NodeId) -> Option<SignalValue> {
    let signal = Signal::<T>::from_id(id)?;
    let (label, subscribers) = runtime::with(|rt| {
        let n = rt.live(id)?;
        Some((n.label.unwrap_or_default(), rt.sub_count(id.index)))
    })?;
    let value = signal.with_untracked(|v| format!("{v:?}"));
    Some(SignalValue {
        id: id.as_u64(),
        label,
        type_name: std::any::type_name::<T>(),
        value,
        subscribers,
    })
}

fn write_of<T: 'static>(id: NodeId, text: &str) -> Option<Result<(), String>> {
    let signal = Signal::<T>::from_id(id)?;
    Some(parse_as::<T>(text).map(|value| signal.replace(value)))
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
    /// Effects and computeds that read it in their last run.
    pub subscribers: usize,
}

/// Every labeled signal alive on this thread, oldest first.
#[must_use]
pub fn probes() -> Vec<SignalValue> {
    let list: Vec<(NodeId, Snapshot)> =
        PROBES.with(|p| p.borrow().iter().map(|probe| (probe.id, probe.snapshot)).collect());
    list.into_iter().filter_map(|(id, snapshot)| snapshot(id)).collect()
}

/// Write `text` into the labeled signal `id` as a click would: subscribers
/// run and the next frame shows it. `Err` names why not (gone, a type the
/// command line cannot spell, a value that does not parse).
pub fn write_probe(id: u64, text: &str) -> Result<(), String> {
    // Write outside the borrow: the write runs effects, which may label new
    // signals.
    let found = PROBES.with(|p| {
        p.borrow()
            .iter()
            .find(|probe| probe.id.as_u64() == id)
            .map(|probe| (probe.id, probe.write))
    });
    let Some((node, write)) = found else {
        return Err(format!("no labeled signal #{id}"));
    };
    write(node, text).unwrap_or_else(|| Err(format!("signal #{id} was disposed")))
}
