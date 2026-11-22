//! The heap, counted: a binary that installs [`COUNTING`] as its global
//! allocator gets what each frame allocated, on the thread that built it and
//! in the whole process, and what the heap holds, in its
//! [`crate::inspect::FrameReport`]. One line in the binary opts in:
//!
//! ```ignore
//! #[global_allocator]
//! static HEAP: gui::heap::Counting = gui::heap::COUNTING;
//! ```
//!
//! Two sets of counters (ADR-0023): the process's (atomics: allocations,
//! bytes allocated, bytes held, the high-water mark), and each thread's own
//! (`thread_local` cells: allocations and bytes allocated on that thread),
//! so a frame's budget is the UI thread's work whatever a network thread,
//! the layout pool or the GPU driver allocates beside it. The allocator is
//! `smelt-perf`'s counting wrapper over the system allocator: the workspace
//! forbids `unsafe`, and implementing `GlobalAlloc` needs it.
//!
//! `dhat` is for diagnosis only: a binary built with its own `dhat-heap`
//! feature installs `dhat::Alloc` instead, and these counters read `None`.

use std::alloc::System;
use std::sync::Once;

use smelt_perf::alloc as counted;

/// The system allocator with process and per-thread counters on every call.
pub type Counting = counted::Counting<System>;

/// The counting allocator a binary installs, which [`now`] and [`thread`] read.
pub const COUNTING: Counting = Counting::new(System);

/// Per-thread counting starts with the first reading: the counters'
/// thread-local cells are written only once asked for.
fn start() {
    static STARTED: Once = Once::new();
    STARTED.call_once(counted::enable);
}

/// The process's heap at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Heap {
    /// Allocations (reallocations included) since the process started.
    pub allocations: usize,
    /// Bytes allocated since the process started, reallocations' growth included.
    pub allocated: u64,
    /// Bytes held now.
    pub live: u64,
    /// The most bytes held at once since the process started.
    pub peak: u64,
}

impl Heap {
    /// What was allocated between `before` and `self`.
    #[must_use]
    pub fn since(self, before: Self) -> Self {
        Self {
            allocations: self.allocations.saturating_sub(before.allocations),
            allocated: self.allocated.saturating_sub(before.allocated),
            live: self.live,
            peak: self.peak,
        }
    }
}

/// What one thread allocated.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadHeap {
    /// Allocations (reallocations included) made on the thread.
    pub allocations: u64,
    /// Bytes they allocated (a reallocation's growth).
    pub allocated: u64,
}

impl ThreadHeap {
    /// What the thread allocated between `before` and `self`.
    #[must_use]
    pub fn since(self, before: Self) -> Self {
        Self {
            allocations: self.allocations.saturating_sub(before.allocations),
            allocated: self.allocated.saturating_sub(before.allocated),
        }
    }
}

/// Whether [`COUNTING`] is the global allocator: it has counted something
/// (a running process has allocated).
fn installed() -> Option<counted::AllocStats> {
    let s = counted::snapshot();
    (s.allocs > 0).then_some(s)
}

fn bytes(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// The process's heap now; `None` when [`COUNTING`] is not the global allocator.
#[must_use]
pub fn now() -> Option<Heap> {
    start();
    let s = installed()?;
    Some(Heap {
        allocations: usize::try_from(s.allocs.saturating_add(s.reallocs)).unwrap_or(usize::MAX),
        allocated: s.bytes_allocated,
        live: bytes(s.current_bytes),
        peak: bytes(s.peak_bytes),
    })
}

/// What this thread has allocated since counting started (the first
/// reading on any thread); `None` when [`COUNTING`] is not the global allocator.
#[must_use]
pub fn thread() -> Option<ThreadHeap> {
    start();
    installed()?;
    let (allocations, allocated) = counted::thread_snapshot();
    Some(ThreadHeap { allocations, allocated })
}

/// What `f` allocated on this thread, beside what it returns; `None` for the
/// count when [`COUNTING`] is not the global allocator.
pub fn measure<R>(f: impl FnOnce() -> R) -> (R, Option<ThreadHeap>) {
    let before = thread();
    let out = f();
    (out, thread().zip(before).map(|(after, before)| after.since(before)))
}
