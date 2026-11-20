//! The heap, counted: a binary that installs [`COUNTING`] as its global
//! allocator gets what each frame allocated and what the heap holds in its
//! [`crate::inspect::FrameReport`]. One line in the binary opts in:
//!
//! ```ignore
//! #[global_allocator]
//! static HEAP: &gui::heap::Counting = gui::heap::COUNTING;
//! ```
//!
//! The counts are the process's, every thread's allocations included.

use stats_alloc::{INSTRUMENTED_SYSTEM, StatsAlloc};
use std::alloc::System;

/// The system allocator with counters on every call.
pub type Counting = StatsAlloc<System>;

/// The one counting allocator a binary installs and [`now`] reads.
pub static COUNTING: &Counting = &INSTRUMENTED_SYSTEM;

/// The heap's counters at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Heap {
    /// Allocations made since the process started.
    pub allocations: usize,
    /// Bytes allocated since the process started, reallocations' growth included.
    pub allocated: u64,
    /// Bytes held now.
    pub live: u64,
}

impl Heap {
    /// What was allocated between `before` and `self`.
    #[must_use]
    pub fn since(self, before: Self) -> Self {
        Self {
            allocations: self.allocations.saturating_sub(before.allocations),
            allocated: self.allocated.saturating_sub(before.allocated),
            live: self.live,
        }
    }
}

/// The heap now; `None` when [`COUNTING`] is not the global allocator (it has
/// counted nothing, and a running process has allocated).
#[must_use]
pub fn now() -> Option<Heap> {
    let s = COUNTING.stats();
    if s.allocations == 0 {
        return None;
    }
    // A reallocation's growth is already in `bytes_allocated` and its
    // shrinking in `bytes_deallocated`.
    let bytes = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
    let allocated = bytes(s.bytes_allocated);
    let freed = bytes(s.bytes_deallocated);
    Some(Heap {
        allocations: s.allocations.saturating_add(s.reallocations),
        allocated,
        live: allocated.saturating_sub(freed),
    })
}
