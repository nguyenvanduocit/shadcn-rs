//! Seeded runs for tests and benches (ADR-0023), as GPUI's test scheduler
//! has them (`references/zed/crates/gpui/src/seeds.rs`,
//! `platform/test/dispatcher.rs`, `randomized_element_tree.rs`):
//!
//! - [`seeds`] and [`run_seeded`]: a randomized test runs `0..ITERATIONS`,
//!   or `SEED..SEED + ITERATIONS` when `SEED` is set, and a failure names the
//!   seed that repeats it;
//! - [`Rng`]: the one seeded generator (devtools' `monkey` walks it too);
//! - [`Scheduler`]: tasks (another thread's message, a click, a frame) run
//!   in a random order the seed fixes, on a fake clock that also drives
//!   `gui`'s timers, until nothing is ready ([`Scheduler::run_until_parked`]);
//! - [`RandomTree`]: a seeded element tree in four shapes, and seeded edits
//!   to it, for tests (incremental equals cold) and benches.

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;

use crate::clock;
use crate::color::Color;
use crate::element::{Element, IntoElement, div, text};
use crate::prelude::*;

// ── Seeds ────────────────────────────────────────────────────────────────────

/// The seeds a randomized test or bench runs, in order: `0..iterations`
/// then `explicit` (the default single iteration dropped when there are
/// explicit seeds); with `$SEED` set, `SEED..SEED + iterations` alone.
/// `$ITERATIONS` replaces `iterations`. One `SEED` repeats either.
#[must_use]
pub fn seeds(iterations: u64, explicit: &[u64]) -> Vec<u64> {
    let iterations = env_number("ITERATIONS").unwrap_or(iterations);
    seeds_from(iterations, explicit, env_number("SEED"))
}

fn env_number(name: &str) -> Option<u64> {
    let value = std::env::var(name).ok()?;
    match value.trim().parse() {
        Ok(n) => Some(n),
        Err(_) => {
            eprintln!("{name}={value:?} is not a number; ignored");
            None
        }
    }
}

fn seeds_from(iterations: u64, explicit: &[u64], env_seed: Option<u64>) -> Vec<u64> {
    match env_seed {
        Some(seed) => (seed..seed.saturating_add(iterations)).collect(),
        None if iterations == 1 && !explicit.is_empty() => explicit.to_vec(),
        None => (0..iterations).chain(explicit.iter().copied()).collect(),
    }
}

/// Run `test` once per seed of [`seeds`]`(iterations, explicit)`; a seed
/// that fails is printed with how to run it alone, and its panic goes on.
pub fn run_seeded(iterations: u64, explicit: &[u64], mut test: impl FnMut(u64)) {
    let all = seeds(iterations, explicit);
    let many = all.len() > 1;
    for seed in all {
        if let Err(panic) = panic::catch_unwind(AssertUnwindSafe(|| test(seed))) {
            if many {
                eprintln!("failing seed: {seed}; run it alone with SEED={seed} ITERATIONS=1");
            }
            panic::resume_unwind(panic);
        }
    }
}

// ── Measured and cold passes ─────────────────────────────────────────────────

/// Which of a measuring test's two runs this is. A test that counts what a
/// frame allocates or visits cannot run with the cold check on (the check
/// allocates and visits too), so [`both_passes`] runs its body twice: the
/// [`Pass::Measured`] run with the check off, asserting the measurements,
/// then the [`Pass::Cold`] run with every frame checked against one built
/// cold (ADR-0013), the measurement asserts skipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Measured,
    Cold,
}

impl Pass {
    /// Set `s` up for this pass: cold check off for the measured one, on for
    /// the cold one (nothing in a release build, which has no check).
    pub fn scenario(self, s: &mut crate::Scenario) {
        s.check_cold(self == Self::Cold);
    }

    /// Run `assert` (a measurement's assertions) in the measured pass only.
    pub fn measured(self, assert: impl FnOnce()) {
        if self == Self::Measured {
            assert();
        }
    }
}

/// Run `body` as the measured pass, then, in a build with the cold check,
/// as the cold one.
pub fn both_passes(body: impl Fn(Pass)) {
    body(Pass::Measured);
    if cfg!(debug_assertions) {
        body(Pass::Cold);
    }
}

// ── Rng ──────────────────────────────────────────────────────────────────────

/// A small seeded generator (xorshift64, its state spread from the seed by
/// splitmix64 so every seed, 0 included, walks its own way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        Self(z.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A number below `n` (0 when `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64().checked_rem(n).unwrap_or(0)
    }

    /// An index below `n` (0 when `n` is 0).
    pub fn index(&mut self, n: usize) -> usize {
        usize::try_from(self.below(u64::try_from(n).unwrap_or(u64::MAX))).unwrap_or(0)
    }

    /// A number in `0..1`.
    pub fn unit(&mut self) -> f32 {
        // 24 bits: what an f32 holds exactly.
        let bits = u32::try_from(self.next_u64() >> 40).unwrap_or(0);
        crate::convert::u32_to_f32(bits) / 16_777_216.0
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub fn pick<'a, T>(&mut self, from: &'a [T]) -> Option<&'a T> {
        let i = self.index(from.len());
        from.get(i)
    }
}

// ── Scheduler ────────────────────────────────────────────────────────────────

/// A task: what one step of a thread, an input or a frame does.
type Run = Box<dyn FnOnce(&mut Scheduler)>;

struct Task {
    name: String,
    run: Run,
}

/// Tasks run one at a time in an order the seed decides, on a fake clock.
///
/// A ready task is picked at random from all that are ready; a task made
/// with a delay is ready once the clock reaches it. The clock is `gui`'s
/// scripted clock ([`clock::script`]): moving it runs `gui`'s timers that
/// fall due on the way, at their own times. Tasks run on the calling thread,
/// so another thread's sends are tasks that call a mailbox's `Sender`; the
/// mail arrives where the test (or a `Scenario`'s frame) delivers it.
pub struct Scheduler {
    seed: u64,
    rng: Rng,
    ready: Vec<Task>,
    /// Tasks waiting for the clock: when due, in the order made.
    later: Vec<(Duration, u64, Task)>,
    now: Duration,
    made: u64,
    log: Vec<String>,
    /// How many tasks one [`Self::run_until_parked`] may run.
    limit: usize,
}

impl Scheduler {
    /// A scheduler walking `seed`'s order, taking over `gui`'s clock at zero.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        clock::cancel_all();
        clock::script_fresh(Duration::ZERO);
        Self {
            seed,
            rng: Rng::new(seed),
            ready: Vec::new(),
            later: Vec::new(),
            now: Duration::ZERO,
            made: 0,
            log: Vec::new(),
            limit: 100_000,
        }
    }

    #[must_use]
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The generator tasks draw from: what they choose is part of the seed's run.
    pub fn rng(&mut self) -> &mut Rng {
        &mut self.rng
    }

    /// The clock's time since the scheduler started.
    #[must_use]
    pub fn now(&self) -> Duration {
        self.now
    }

    /// Run at most `limit` tasks in one [`Self::run_until_parked`].
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    /// Make `run` ready now.
    pub fn spawn(&mut self, name: impl Into<String>, run: impl FnOnce(&mut Self) + 'static) {
        self.ready.push(Task {
            name: name.into(),
            run: Box::new(run),
        });
    }

    /// Make `run` ready once the clock has moved `delay` on.
    pub fn spawn_after(&mut self, delay: Duration, name: impl Into<String>, run: impl FnOnce(&mut Self) + 'static) {
        if delay.is_zero() {
            return self.spawn(name, run);
        }
        self.made = self.made.saturating_add(1);
        let task = Task {
            name: name.into(),
            run: Box::new(run),
        };
        self.later.push((self.now.saturating_add(delay), self.made, task));
    }

    /// A delay below `max`, drawn from the seed (a thread's random latency).
    pub fn random_delay(&mut self, max: Duration) -> Duration {
        let micros = u64::try_from(max.as_micros()).unwrap_or(u64::MAX);
        Duration::from_micros(self.rng.below(micros.max(1)))
    }

    /// Run one ready task, picked at random; whether there was one.
    pub fn tick(&mut self) -> bool {
        if self.ready.is_empty() {
            return false;
        }
        let i = self.rng.index(self.ready.len());
        let task = self.ready.swap_remove(i);
        self.log
            .push(format!("{:>9.3}ms {}", self.now.as_secs_f64() * 1000.0, task.name));
        (task.run)(self);
        true
    }

    /// Run ready tasks (and those they make ready) until none is left; the
    /// clock does not move. Panics past the task limit, naming the seed: a
    /// run that never parks is a livelock.
    pub fn run_until_parked(&mut self) {
        let mut ran = 0_usize;
        while self.tick() {
            ran = ran.saturating_add(1);
            assert!(
                ran <= self.limit,
                "seed {}: did not park after {} tasks; the last ran:\n{}",
                self.seed,
                self.limit,
                self.log
                    .get(self.log.len().saturating_sub(12)..)
                    .unwrap_or_default()
                    .join("\n")
            );
        }
    }

    /// Move the clock `by` on: at each time a task falls due on the way,
    /// `gui`'s timers due by then run, the task becomes ready, and everything
    /// ready runs until parked.
    pub fn advance_clock(&mut self, by: Duration) {
        let to = self.now.saturating_add(by);
        while let Some(due) = self.next_due().filter(|&d| d <= to) {
            self.move_to(due);
            self.run_until_parked();
        }
        self.move_to(to);
        self.run_until_parked();
    }

    /// Move the clock to the next task that waits for it, and run until
    /// parked; whether one waited.
    pub fn advance_to_next(&mut self) -> bool {
        let Some(due) = self.next_due() else {
            return false;
        };
        self.move_to(due);
        self.run_until_parked();
        true
    }

    /// Run until parked, then follow the clock until no task waits for it.
    pub fn run_to_end(&mut self) {
        self.run_until_parked();
        while self.advance_to_next() {}
    }

    fn next_due(&self) -> Option<Duration> {
        self.later.iter().map(|(due, _, _)| *due).min()
    }

    fn move_to(&mut self, to: Duration) {
        self.now = self.now.max(to);
        clock::advance_to(self.now);
        let now = self.now;
        let mut due: Vec<(Duration, u64, Task)> = Vec::new();
        let mut i = 0;
        while i < self.later.len() {
            if self.later.get(i).is_some_and(|(d, _, _)| *d <= now) {
                due.push(self.later.swap_remove(i));
            } else {
                i = i.saturating_add(1);
            }
        }
        due.sort_by_key(|(d, n, _)| (*d, *n));
        self.ready.extend(due.into_iter().map(|(_, _, t)| t));
    }

    /// Whether nothing is ready and nothing waits for the clock.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.ready.is_empty() && self.later.is_empty()
    }

    /// The tasks run so far, each with the clock's time: the same seed
    /// gives the same log.
    #[must_use]
    pub fn log(&self) -> &[String] {
        &self.log
    }
}

// ── Random element trees ─────────────────────────────────────────────────────

/// How a [`RandomTree`] grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topology {
    /// Every node a child of the root.
    Wide,
    /// Every node a child of the one before it (a chain, capped at [`MAX_DEPTH`]).
    Deep,
    /// Each node under a node picked anywhere in the tree.
    Mixed,
    /// Mostly one deep branch, now and then a child of the root.
    Narrow,
}

impl Topology {
    pub const ALL: [Self; 4] = [Self::Wide, Self::Deep, Self::Mixed, Self::Narrow];

    /// The topology a seed picks.
    #[must_use]
    pub fn of_seed(seed: u64) -> Self {
        match seed.checked_rem(4).unwrap_or(0) {
            0 => Self::Wide,
            1 => Self::Deep,
            2 => Self::Mixed,
            _ => Self::Narrow,
        }
    }
}

/// How deep a [`RandomTree`] nests: deeper would be a recursion test, not a
/// layout one (building and laying out a node recurses into its children).
pub const MAX_DEPTH: usize = 48;

/// What a generated node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Column,
    Row,
    Wrap,
    Grid,
    Text,
    Fixed,
}

#[derive(Clone, Debug, PartialEq)]
struct Node {
    key: u64,
    parent: Option<usize>,
    depth: usize,
    kind: Kind,
    /// A size in px when fixed (a box), else sized by content.
    size: Option<(f32, f32)>,
    pad: f32,
    gap: f32,
    grow: bool,
    color: u32,
    words: usize,
}

const PALETTE: [u32; 6] = [
    0x00e2_e8f0,
    0x00c7_d2fe,
    0x00fd_e68a,
    0x00bb_f7d0,
    0x00fe_cdd3,
    0x00e9_d5ff,
];
const WORDS: [&str; 12] = [
    "alpha", "layout", "frame", "signal", "paint", "chunk", "glyph", "scroll", "list", "row", "text", "node",
];

/// A seeded element tree of a given size and shape, and seeded edits to it.
///
/// The tree is a model (a list of nodes with parents); [`Self::element`]
/// builds `gui` elements from it, each keyed, so an edit reads to the
/// pipeline as the same edit a real app makes.
#[derive(Clone, Debug, PartialEq)]
pub struct RandomTree {
    nodes: Vec<Node>,
    next_key: u64,
    topology: Topology,
}

impl RandomTree {
    /// `count` nodes under a root, shaped by `topology`, drawn from `seed`.
    #[must_use]
    pub fn new(seed: u64, count: usize, topology: Topology) -> Self {
        let mut rng = Rng::new(seed);
        let mut tree = Self {
            nodes: vec![Node {
                key: 0,
                parent: None,
                depth: 0,
                kind: Kind::Column,
                size: None,
                pad: 4.0,
                gap: 2.0,
                grow: false,
                color: 0x00ff_ffff,
                words: 0,
            }],
            next_key: 1,
            topology,
        };
        for _ in 0..count {
            let parent = tree.parent_for_new(&mut rng);
            tree.add(&mut rng, parent);
        }
        tree
    }

    /// Nodes, the root included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn container(&self, i: usize) -> bool {
        self.nodes
            .get(i)
            .is_some_and(|n| !matches!(n.kind, Kind::Text | Kind::Fixed) && n.depth < MAX_DEPTH)
    }

    fn parent_for_new(&self, rng: &mut Rng) -> usize {
        let last = self.nodes.len().saturating_sub(1);
        let wanted = match self.topology {
            Topology::Wide => 0,
            Topology::Deep => last,
            Topology::Mixed => rng.index(self.nodes.len()),
            Topology::Narrow => {
                if rng.chance(0.8) {
                    last
                } else {
                    0
                }
            }
        };
        // Up to the nearest node that takes children.
        let mut at = Some(wanted);
        while let Some(i) = at {
            if self.container(i) {
                return i;
            }
            at = self.nodes.get(i).and_then(|n| n.parent);
        }
        0
    }

    fn add(&mut self, rng: &mut Rng, parent: usize) {
        let depth = self.nodes.get(parent).map_or(0, |p| p.depth.saturating_add(1));
        let kind = *rng
            .pick(&[
                Kind::Column,
                Kind::Row,
                Kind::Wrap,
                Kind::Grid,
                Kind::Text,
                Kind::Text,
                Kind::Fixed,
            ])
            .unwrap_or(&Kind::Text);
        let size = (kind == Kind::Fixed).then(|| {
            let w = crate::convert::u32_to_f32(u32::try_from(rng.below(120)).unwrap_or(0).saturating_add(8));
            let h = crate::convert::u32_to_f32(u32::try_from(rng.below(48)).unwrap_or(0).saturating_add(8));
            (w, h)
        });
        let node = Node {
            key: self.next_key,
            parent: Some(parent),
            depth,
            kind,
            size,
            pad: crate::convert::u32_to_f32(u32::try_from(rng.below(4)).unwrap_or(0).saturating_mul(2)),
            gap: crate::convert::u32_to_f32(u32::try_from(rng.below(3)).unwrap_or(0).saturating_mul(2)),
            grow: rng.chance(0.25),
            color: *rng.pick(&PALETTE).unwrap_or(&PALETTE[0]),
            words: usize::try_from(rng.below(6).saturating_add(1)).unwrap_or(1),
        };
        self.next_key = self.next_key.saturating_add(1);
        self.nodes.push(node);
    }

    /// One seeded edit: add a node, remove a subtree, restyle, retext or
    /// move a node; what it did, to log.
    pub fn mutate(&mut self, rng: &mut Rng) -> String {
        let n = self.nodes.len();
        let pick = if n > 1 {
            rng.index(n.saturating_sub(1)).saturating_add(1)
        } else {
            0
        };
        match rng.below(5) {
            0 | 1 if n > 1 || pick == 0 => {
                let parent = self.parent_for_new(rng);
                self.add(rng, parent);
                format!("add under #{}", self.nodes.get(parent).map_or(0, |p| p.key))
            }
            2 if n > 2 => {
                let key = self.nodes.get(pick).map_or(0, |x| x.key);
                let gone = self.remove(pick);
                format!("remove #{key} ({gone} nodes)")
            }
            3 if n > 1 => {
                let Some(node) = self.nodes.get_mut(pick) else {
                    return "nothing".to_owned();
                };
                node.words = usize::try_from(rng.below(8).saturating_add(1)).unwrap_or(1);
                node.pad = crate::convert::u32_to_f32(u32::try_from(rng.below(4)).unwrap_or(0).saturating_mul(2));
                format!("retext #{}", node.key)
            }
            _ if n > 1 => {
                let color = *rng.pick(&PALETTE).unwrap_or(&PALETTE[0]);
                let grow = rng.chance(0.5);
                let Some(node) = self.nodes.get_mut(pick) else {
                    return "nothing".to_owned();
                };
                node.color = color;
                node.grow = grow;
                if let Some((w, h)) = node.size.as_mut() {
                    *w = (*w + 8.0).min(160.0);
                    *h = (*h + 4.0).min(64.0);
                }
                format!("restyle #{}", node.key)
            }
            _ => {
                self.add(rng, 0);
                "add under #0".to_owned()
            }
        }
    }

    /// Remove node `i` and everything under it; how many went.
    fn remove(&mut self, i: usize) -> usize {
        let mut gone = vec![false; self.nodes.len()];
        if let Some(g) = gone.get_mut(i) {
            *g = true;
        }
        // Parents come before their children in the list.
        for j in i.saturating_add(1)..self.nodes.len() {
            let under = self
                .nodes
                .get(j)
                .and_then(|n| n.parent)
                .is_some_and(|p| gone.get(p).copied().unwrap_or(false));
            if let (true, Some(g)) = (under, gone.get_mut(j)) {
                *g = true;
            }
        }
        let mut new_index = vec![0_usize; self.nodes.len()];
        let mut kept = Vec::with_capacity(self.nodes.len());
        for (j, node) in std::mem::take(&mut self.nodes).into_iter().enumerate() {
            if gone.get(j).copied().unwrap_or(false) {
                continue;
            }
            if let Some(slot) = new_index.get_mut(j) {
                *slot = kept.len();
            }
            kept.push(node);
        }
        for node in &mut kept {
            node.parent = node.parent.and_then(|p| new_index.get(p).copied());
        }
        let removed = gone.iter().filter(|g| **g).count();
        self.nodes = kept;
        removed
    }

    /// The tree as `gui` elements.
    #[must_use]
    pub fn element(&self) -> Element {
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let Some(list) = n.parent.and_then(|p| children.get_mut(p)) {
                list.push(i);
            }
        }
        self.build(0, &children)
    }

    fn build(&self, i: usize, children: &[Vec<usize>]) -> Element {
        let Some(n) = self.nodes.get(i) else {
            return Element::Empty;
        };
        if n.kind == Kind::Text {
            let words: Vec<&str> = (0..n.words)
                .map(|w| {
                    WORDS
                        .get((w.saturating_add(i)).checked_rem(WORDS.len()).unwrap_or(0))
                        .copied()
                        .unwrap_or("x")
                })
                .collect();
            return div()
                .key(n.key)
                .padding(n.pad)
                .when(n.grow, |d| d.flex(Flex::grow(1.0)))
                .child(text(words.join(" ")).text_size(14.0).line_height(20.0))
                .into_element();
        }
        let mut d = div()
            .key(n.key)
            .background(Color::hex(n.color))
            .padding(n.pad)
            .gap(n.gap);
        d = match n.kind {
            Kind::Row => d.display(Display::Flex).flex_direction(FlexDirection::Row),
            Kind::Wrap => d
                .display(Display::Flex)
                .flex_direction(FlexDirection::Row)
                .flex_wrap(true),
            Kind::Grid => d.display(Display::Grid).grid_columns(Tracks::repeat(3, Track::Fr(1.0))),
            Kind::Fixed => match n.size {
                Some((w, h)) => d.width(w).height(h),
                None => d,
            },
            Kind::Column | Kind::Text => d.display(Display::Flex).flex_direction(FlexDirection::Column),
        };
        if n.grow {
            d = d.flex(Flex::grow(1.0));
        }
        let kids: Vec<Element> = children
            .get(i)
            .map(|list| list.iter().map(|&c| self.build(c, children)).collect())
            .unwrap_or_default();
        d.children(kids).into_element()
    }
}

#[cfg(test)]
#[expect(clippy::arithmetic_side_effects, reason = "test values are small")]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn seeds_follow_iterations_explicit_seeds_and_the_seed_variable() {
        assert_eq!(seeds_from(1, &[], None), [0]);
        assert_eq!(seeds_from(3, &[], None), [0, 1, 2]);
        assert_eq!(seeds_from(1, &[10, 20], None), [10, 20]);
        assert_eq!(seeds_from(2, &[10], None), [0, 1, 10]);
        assert_eq!(seeds_from(1, &[10], Some(7)), [7]);
        assert_eq!(
            seeds_from(2, &[10], Some(7)),
            [7, 8],
            "SEED starts the iterations, alone"
        );
    }

    #[test]
    fn every_seed_walks_its_own_way_and_the_same_seed_the_same() {
        let walk = |seed| {
            let mut r = Rng::new(seed);
            (0..8).map(|_| r.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(walk(0), walk(0));
        assert_ne!(walk(0), walk(1));
        assert!(walk(0).iter().all(|&x| x != 0), "seed 0 is not stuck at zero");
    }

    /// A run of `seed`: producers that send with random delays, the log of what ran.
    fn produce(seed: u64) -> (Vec<String>, Vec<u32>) {
        let got = Rc::new(RefCell::new(Vec::new()));
        let mut s = Scheduler::new(seed);
        for p in 0..4_u32 {
            let got = Rc::clone(&got);
            s.spawn(format!("producer {p}"), move |s| step(s, p, 0, got));
        }
        fn step(s: &mut Scheduler, p: u32, n: u32, got: Rc<RefCell<Vec<u32>>>) {
            got.borrow_mut().push(p * 100 + n);
            if n < 5 {
                let delay = s.random_delay(Duration::from_millis(20));
                s.spawn_after(delay, format!("producer {p} step {}", n + 1), move |s| {
                    step(s, p, n + 1, got);
                });
            }
        }
        s.run_to_end();
        assert!(s.is_idle());
        let order = got.borrow().clone();
        (s.log().to_vec(), order)
    }

    #[test]
    fn a_seed_repeats_its_order_and_another_seed_takes_another() {
        let (log, order) = produce(3);
        assert_eq!(produce(3), (log, order.clone()), "the same seed, the same run");
        assert_eq!(order.len(), 24);
        let differs = (4..12).any(|seed| produce(seed).1 != order);
        assert!(differs, "other seeds interleave the producers otherwise");
        // Whatever the order, each producer's own steps stay in order.
        for p in 0..4 {
            let mine: Vec<u32> = order.iter().copied().filter(|v| v / 100 == p).collect();
            assert!(mine.windows(2).all(|w| w[0] < w[1]), "{mine:?}");
        }
    }

    #[test]
    fn the_clock_runs_gui_timers_at_their_times_among_the_tasks() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut s = Scheduler::new(9);
        let t = Rc::clone(&seen);
        clock::after(Duration::from_millis(15), move || {
            t.borrow_mut().push("timer".to_owned())
        });
        for at in [10_u64, 20] {
            let t = Rc::clone(&seen);
            s.spawn_after(Duration::from_millis(at), format!("task {at}"), move |_| {
                t.borrow_mut().push(format!("task {at}"));
            });
        }
        s.advance_clock(Duration::from_millis(30));
        assert_eq!(*seen.borrow(), ["task 10", "timer", "task 20"]);
        assert_eq!(s.now(), Duration::from_millis(30));
    }

    #[test]
    fn mail_from_producers_in_any_order_is_seen_glitch_free_and_whole() {
        use crate::reactive::{Computed, Effect, Signal, batch};
        run_seeded(24, &[], |seed| {
            let scope = crate::Scope::new();
            let (items, total, glitches) = scope.run(|| {
                let items = Signal::new(Vec::<u32>::new());
                let total = Signal::new(0_u64);
                let sum = Computed::new(move || items.with(|v| v.iter().map(|&x| u64::from(x)).sum::<u64>()));
                let glitches = Rc::new(RefCell::new(0_u32));
                let g = Rc::clone(&glitches);
                // An effect that sees the list and the total together: they
                // must agree whenever it runs.
                Effect::new(move || {
                    if sum.get() != total.get() {
                        *g.borrow_mut() += 1;
                    }
                });
                (items, total, glitches)
            });
            let sender = scope.run(|| {
                crate::mailbox(move |v: u32| {
                    batch(|| {
                        items.update(|list| list.push(v));
                        total.update(|t| *t += u64::from(v));
                    });
                })
            });
            let mut s = Scheduler::new(seed);
            for p in 0..3_u32 {
                for n in 0..4_u32 {
                    let tx = sender.clone();
                    let delay = s.random_delay(Duration::from_millis(30));
                    s.spawn_after(delay, format!("send {p}.{n}"), move |_| {
                        assert!(tx.send(p * 10 + n + 1));
                    });
                }
            }
            // Mail is delivered at random points between sends, as frames fall.
            for k in 0..6 {
                let delay = s.random_delay(Duration::from_millis(30));
                s.spawn_after(delay, format!("deliver {k}"), |_| {
                    crate::mailbox::deliver();
                });
            }
            s.run_to_end();
            crate::mailbox::deliver();
            assert_eq!(*glitches.borrow(), 0, "seed {seed}: an effect saw old and new mixed");
            assert_eq!(items.with(Vec::len), 12, "seed {seed}: every message arrived");
            assert_eq!(
                total.get(),
                (0..3_u64)
                    .flat_map(|p| (0..4_u64).map(move |n| p * 10 + n + 1))
                    .sum::<u64>()
            );
            scope.dispose();
        });
    }

    #[test]
    fn a_random_tree_has_its_size_and_shape_and_edits_repeat_by_seed() {
        for topology in Topology::ALL {
            let tree = RandomTree::new(5, 300, topology);
            assert_eq!(tree.len(), 301);
            let deepest = tree.nodes.iter().map(|n| n.depth).max().unwrap_or(0);
            assert!(deepest <= MAX_DEPTH, "{topology:?} nests {deepest}");
            if topology == Topology::Wide {
                assert_eq!(deepest, 1);
            }
            let mut a = tree.clone();
            let mut b = tree.clone();
            let (mut ra, mut rb) = (Rng::new(11), Rng::new(11));
            for _ in 0..50 {
                assert_eq!(a.mutate(&mut ra), b.mutate(&mut rb));
            }
            assert_eq!(a, b);
            assert!(
                a.nodes
                    .iter()
                    .skip(1)
                    .all(|n| n.parent.is_some_and(|p| p < a.nodes.len()))
            );
        }
        assert_ne!(
            RandomTree::new(1, 50, Topology::Mixed),
            RandomTree::new(2, 50, Topology::Mixed)
        );
    }
}
