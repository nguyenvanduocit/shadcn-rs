//! Raster workers (ADR-0017, Chromium's raster worker pool): tiles are
//! planned ([`super::plan`]) on threads of their own, most urgent first,
//! while the render thread draws what is ready. The render thread hands
//! over what it wants each frame in priority order (visible tiles nearest
//! the centre, then prefetch), which replaces what still waits, so a tile
//! no longer wanted is never planned; plans come back by channel and are
//! rasterized within a frame's budget.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use super::WorldSnapshot;
use super::plan::{Step, View, cut_about, plan};

/// A tile: its world, its tiling (raster scale and turn, as bits) and its
/// place in the tiling. Not its version: a tile a change did not reach is
/// kept as it is (Chromium's invalidation), one it did is rastered again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub world: u64,
    pub scale: u64,
    pub turn: u64,
    /// Where the grid falls within a device pixel, as bits.
    pub phase: [u64; 2],
    pub x: i64,
    pub y: i64,
}

/// A tile to plan: its key, the world as it is, the view it shows.
pub struct Request {
    pub key: TileKey,
    pub world: Arc<WorldSnapshot>,
    pub view: View,
}

/// The steps of `view` of `world`: huge objects cut about the view's corner.
#[must_use]
pub fn plan_tile(world: &WorldSnapshot, view: &View) -> Vec<Step> {
    let mut out = Vec::new();
    let anchor = view.to_world((0.0, 0.0));
    plan(world, view, &mut |_, o| cut_about(o, view, anchor), &mut out);
    out
}

struct Queue {
    waiting: VecDeque<Request>,
    working: usize,
    /// The tiles being planned: not asked again while they are.
    planning: std::collections::HashSet<TileKey>,
    /// Workers take nothing while held (tests: what waits is then exactly
    /// what was wanted, whatever the scheduler does).
    held: bool,
    stop: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    idle: Condvar,
}

/// A tile planned: its key, the world version planned, its steps.
pub type Planned = (TileKey, u64, Vec<Step>);

/// The raster workers and what they were asked.
pub struct Planner {
    shared: Arc<Shared>,
    sender: mpsc::Sender<Planned>,
    done: mpsc::Receiver<Planned>,
    threads: Vec<std::thread::JoinHandle<()>>,
    count: usize,
}

impl Planner {
    /// A pool of `count` workers, started when first asked for work.
    #[must_use]
    pub fn new(count: usize) -> Self {
        let (sender, done) = mpsc::channel();
        Self {
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue {
                    waiting: VecDeque::new(),
                    working: 0,
                    planning: std::collections::HashSet::new(),
                    held: false,
                    stop: false,
                }),
                work: Condvar::new(),
                idle: Condvar::new(),
            }),
            sender,
            done,
            threads: Vec::new(),
            count: count.max(1),
        }
    }

    /// Half the cores, from one to four: the UI and render threads and
    /// layout's pool keep the rest.
    #[must_use]
    pub fn sized_for_machine() -> Self {
        let cores = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
        Self::new((cores / 2).clamp(1, 4))
    }

    fn start(&mut self) {
        while self.threads.len() < self.count {
            let shared = Arc::clone(&self.shared);
            let sender = self.sender.clone();
            let name = format!("gui-raster-{}", self.threads.len());
            match std::thread::Builder::new()
                .name(name)
                .spawn(move || work(&shared, &sender))
            {
                Ok(t) => self.threads.push(t),
                Err(_) => break,
            }
        }
    }

    /// Plan `wanted`, most urgent first, in place of whatever still waits.
    pub fn want(&mut self, wanted: Vec<Request>) {
        if wanted.is_empty() && self.threads.is_empty() {
            return;
        }
        self.start();
        let mut q = self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        let planning = &q.planning;
        let waiting: VecDeque<Request> = wanted.into_iter().filter(|r| !planning.contains(&r.key)).collect();
        q.waiting = waiting;
        if !q.waiting.is_empty() {
            self.shared.work.notify_all();
        }
    }

    /// Hold the workers (they take nothing that waits) or let them go.
    #[cfg(test)]
    pub(crate) fn hold(&self, held: bool) {
        let mut q = self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        q.held = held;
        if !held {
            self.shared.work.notify_all();
        }
    }

    /// The plans done since last asked.
    pub fn take(&self) -> Vec<Planned> {
        self.done.try_iter().collect()
    }

    /// Wait until nothing waits or is being planned, at most `limit`:
    /// whether it got there (a headless frame sees every plan asked for by
    /// the frame before, so its runs repeat).
    pub fn wait_idle(&self, limit: Duration) -> bool {
        let end = Instant::now().checked_add(limit);
        let mut q = self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        while !q.waiting.is_empty() || q.working > 0 {
            let left = end.map_or(limit, |e| e.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return false;
            }
            q = self
                .shared
                .idle
                .wait_timeout(q, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl Drop for Planner {
    fn drop(&mut self) {
        {
            let mut q = self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            q.stop = true;
            q.waiting.clear();
        }
        self.shared.work.notify_all();
        for t in self.threads.drain(..) {
            // A worker that panicked has nothing left to join.
            t.join().ok();
        }
    }
}

/// A worker: take the most urgent request, plan it, send it back.
fn work(shared: &Shared, sender: &mpsc::Sender<Planned>) {
    loop {
        let request = {
            let mut q = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if q.stop {
                    return;
                }
                if !q.held
                    && let Some(r) = q.waiting.pop_front()
                {
                    q.working = q.working.saturating_add(1);
                    q.planning.insert(r.key);
                    break r;
                }
                q = shared.work.wait(q).unwrap_or_else(PoisonError::into_inner);
            }
        };
        // A plan that panics leaves its tile empty, never the pool short.
        let steps = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            plan_tile(&request.world, &request.view)
        }))
        .unwrap_or_default();
        let gone = sender.send((request.key, request.world.version, steps)).is_err();
        let mut q = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        q.working = q.working.saturating_sub(1);
        q.planning.remove(&request.key);
        if q.waiting.is_empty() && q.working == 0 {
            shared.idle.notify_all();
        }
        if gone {
            return;
        }
    }
}
