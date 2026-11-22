//! The retained layout tree (ADR-0014): nodes kept across frames, laid out
//! by Taffy's algorithms through its low-level traits, each node holding its
//! own cache, and only what changed laid out again.
//!
//! - **Ownership is the island.** A node owns its children (`Vec<Node>`),
//!   so any set of disjoint subtrees can be borrowed `&mut` at once, which
//!   is what lets layout run in parallel in safe Rust, with no lock or
//!   atomic per node. Taffy's algorithms see one node and its children at a
//!   time (`TraversePartialTree`): a node's id is its index in its parent,
//!   [`SELF`] the node itself, and out-of-flow descendants bubbling up from
//!   a child get ids of their own in the node's `foreign` table.
//! - **Style through traits.** A node's style type implements Taffy's style
//!   traits ([`NodeStyle`]); nothing is converted to `taffy::Style`.
//! - **Cache keyed by what was read** ([`cache::NodeCache`]).
//! - **Relayout boundaries by Flutter's rule.** A parent that, since its
//!   cache was last cleared, asked a child only for a final layout at a size
//!   it fixed (both known dimensions, no baseline) never used the child's
//!   content. A change inside such a child then lays the child out alone
//!   with that input, and goes no further up ([`Node::settle`]). Whether it
//!   holds is decided from what the algorithms did, each layout, not from
//!   style, so a wrong boundary cannot be declared.
//! - **Parallel.** Dirty boundaries lay out as parallel jobs, shallowest
//!   first; and when a parent's algorithm asks one of many alike children
//!   for something no cache holds, the same question is put to all of them
//!   at once on the pool (siblings in a list or table are asked the same).
//!   Every result is a pure function of the subtree and its input, so the
//!   layout is bit-identical whatever the thread count or job order
//!   ([`Schedule`]).

mod axes;
mod balance;
mod block;
pub mod cache;
pub mod calc;
mod flex;
mod grid;
mod inline;
mod leaf;
mod row;
mod sizing;
mod stack;

pub use leaf::{Measured, compute as compute_leaf};
pub use stack::set_memo_min;

use taffy::{
    AvailableSpace, BlockContainerStyle, BlockItemStyle, CacheTree, CheapCloneStr, CoreStyle, Display,
    FlexboxContainerStyle, FlexboxItemStyle, GridContainerStyle, GridItemStyle, Layout, LayoutContainingBlock,
    LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree, NodeId, OofItemStyle, RunMode, Size,
    TraversePartialTree,
};

use block::{BlockContext, BlockFormattingContext};
use cache::NodeCache;

/// A node's own id in its view.
pub const SELF: u64 = u64::MAX;
/// Ids from here on name out-of-flow descendants a child passed up.
const FOREIGN: u64 = 1 << 48;

/// What Taffy reads a node's style through.
pub trait NodeStyle: Sized + 'static {
    type Ident: CheapCloneStr + Send + Sync;
    type Ref<'a>: CoreStyle<CustomIdent = Self::Ident>
        + BlockContainerStyle
        + BlockItemStyle
        + FlexboxContainerStyle
        + FlexboxItemStyle
        + GridContainerStyle
        + GridItemStyle
        + OofItemStyle
    where
        Self: 'a;

    fn view(&self) -> Self::Ref<'_>;
    /// Which algorithm lays its children out.
    fn display(&self) -> Display;
    /// Which axes of its parent's size its own percentages read.
    fn reads_parent(&self) -> Size<bool>;
    /// Whether, as a container, it aligns its children on their baselines,
    /// or, as an item, it is aligned on its own.
    fn baseline(&self) -> bool;
    /// The value of a `calc()` handle this style type made.
    fn calc(handle: *const (), basis: f32) -> f32;

    /// Run `run` on each node of `groups`: on the calling thread by default
    /// (a style that may not leave its thread), else each group on the
    /// layout pool with [`parallel`]. Results are summed either way.
    fn run_jobs<L: Measure>(groups: &mut [Vec<&mut Node<Self, L>>], cx: &mut L::Cx, run: &Job<'_, Self, L>) -> Stats {
        let mut stats = Stats::default();
        for job in groups.iter_mut().flatten() {
            stats.add(run(job, cx));
        }
        stats
    }
}

/// One parallel job: lay out a node with a thread's measuring context.
pub type Job<'a, S, L> = dyn Fn(&mut Node<S, L>, &mut <L as Measure>::Cx) -> Stats + Sync + 'a;

/// Run `run` on each node of `groups`, a group to a job on the layout
/// pool, with that thread's measuring context ([`NodeStyle::run_jobs`] for
/// a style that can move between threads). Called from a job already on
/// the pool (a batch inside a subtree a batch laid out), the groups run
/// here in order with that job's context, `cx`: the pool is busy with the
/// outer batch, and a nested job stolen by a thread whose context its own
/// job holds would need a context made for it.
pub fn parallel<S: NodeStyle + Send, L: Measure>(
    groups: &mut [Vec<&mut Node<S, L>>],
    cx: &mut L::Cx,
    run: &Job<'_, S, L>,
) -> Stats {
    use rayon::prelude::*;
    let pool = pool();
    if pool.current_thread_index().is_some() {
        let mut stats = Stats::default();
        for job in groups.iter_mut().flatten() {
            stats.add(run(job, cx));
        }
        return stats;
    }
    pool.install(|| {
        groups
            .par_iter_mut()
            .map(|group| {
                L::Cx::with_worker(|cx| {
                    let mut stats = Stats::default();
                    for job in group.iter_mut() {
                        stats.add(run(job, cx));
                    }
                    stats.work = stats.work.saturating_add(cx.take_work());
                    row::release_scratch();
                    grid::release_scratch();
                    stats
                })
            })
            .reduce(Stats::default, |mut a, b| {
                a.add(b);
                a
            })
    })
}

/// What a leaf's measure reads of the space it is given, besides its width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeasureReads {
    /// Its answer reads the available height (definite, min- or max-content).
    pub available_height: bool,
    /// Its width reads the height it is given, known or available (an
    /// image keeps its ratio).
    pub width_reads_height: bool,
    /// Its whole answer is a function of the width it is given alone: the
    /// known width, else the available one, the two read alike. The leaf
    /// then measures once per width (a size probe at a width and the final
    /// layout at it ask the same), [`cache::NodeCache::measured`].
    pub width_alone: bool,
}

impl MeasureReads {
    /// It may read anything.
    pub const ALL: Self = Self {
        available_height: true,
        width_reads_height: true,
        width_alone: false,
    };
    /// It reads no height: neither the available one, nor, for its width,
    /// the one it is given.
    pub const WIDTH_ONLY: Self = Self {
        available_height: false,
        width_reads_height: false,
        width_alone: false,
    };
    /// Text: it breaks lines in the width it is given, and that is all. A
    /// known width and a definite available one are the same to it, and it
    /// reads no height at all, known or available.
    pub const TEXT: Self = Self {
        available_height: false,
        width_reads_height: false,
        width_alone: true,
    };
}

/// What measures a leaf: text, an image, an input.
pub trait Measure: Send {
    type Cx: Worker;
    /// Its content size, given the content size known and the space its
    /// content box has, and its baselines from the top of its content box.
    fn measure(&self, cx: &mut Self::Cx, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured;

    /// What [`Measure::measure`] reads (everything, unless it says less):
    /// the layout cache lets one answer stand for every input that differs
    /// only in what it does not read.
    fn reads(&self) -> MeasureReads {
        MeasureReads::ALL
    }

    /// A replaced element (an image): a size its aspect ratio decides is
    /// not made larger by its content.
    fn replaced(&self) -> bool {
        false
    }

    /// [`Measure::measure`] for a leaf with children, laid in its content
    /// as atomic inlines ([`inline`]): `boxes` are their margin boxes, one
    /// to a child, in order, at the size they take in this space. A leaf
    /// that lays none out ignores them.
    fn measure_inline(
        &self,
        cx: &mut Self::Cx,
        known: Size<Option<f32>>,
        available: Size<AvailableSpace>,
        boxes: &[InlineBox],
    ) -> Measured {
        let _ = boxes;
        self.measure(cx, known, available)
    }

    /// Where each of `boxes` (as [`Measure::measure_inline`] has them) has
    /// its margin box's top-left corner in a content box `width` wide, one
    /// to a box; the content box's corner for each by default.
    fn place_inline(&self, cx: &mut Self::Cx, width: f32, boxes: &[InlineBox]) -> Vec<taffy::Point<f32>> {
        let _ = (cx, width);
        vec![taffy::Point::ZERO; boxes.len()]
    }
}

/// A child laid in its leaf parent's lines as a glyph is (CSS's atomic
/// inline, an `inline-flex` button in a `<p>`): its margin box, and its
/// baseline below that box's top; none puts its bottom margin edge on the
/// line's baseline.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InlineBox {
    pub width: f32,
    pub height: f32,
    pub baseline: Option<f32>,
}

/// A measuring context, one per thread.
pub trait Worker {
    /// Run `f` with this thread's context (on a pool thread).
    fn with_worker<R>(f: impl FnOnce(&mut Self) -> R) -> R;

    /// What this context did since asked last, counted into the pass's
    /// [`Stats::work`] (a text system: the pieces it shaped).
    fn take_work(&mut self) -> u64 {
        0
    }
}

impl Worker for () {
    fn with_worker<R>(f: impl FnOnce(&mut Self) -> R) -> R {
        f(&mut ())
    }
}

/// How a layout is spread over threads.
#[derive(Clone, Copy, Debug, Default)]
pub struct Schedule {
    /// Run on the layout pool; else everything on the calling thread.
    pub parallel: bool,
    /// Take sequential jobs last first (the determinism check).
    pub reverse: bool,
}

impl Schedule {
    /// The pipeline's schedule: parallel unless `GUI_LAYOUT_THREADS=1`.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            parallel: pool().current_num_threads() > 1,
            reverse: false,
        }
    }
}

/// The layout pool: ¾ of the logical cores, at most 8 (as Ladybird's
/// `style_layout_pool`), or `GUI_LAYOUT_THREADS`.
pub fn pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let wanted = std::env::var("GUI_LAYOUT_THREADS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| (cores.saturating_mul(3) / 4).clamp(1, 8));
        rayon::ThreadPoolBuilder::new()
            .num_threads(wanted.max(1))
            .thread_name(|i| format!("gui-layout-{i}"))
            .stack_size(super::STACK)
            .build()
            .or_else(|_| rayon::ThreadPoolBuilder::new().num_threads(1).build())
            .unwrap_or_else(|_| std::process::abort())
    })
}

/// What one layout did: `compute_child_layout` calls, cache misses (an
/// algorithm ran), leaf measures, final layouts that placed children, and
/// parallel jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub calls: u64,
    pub misses: u64,
    pub measures: u64,
    /// Runs of a container's algorithm for a final layout: each placed the
    /// container's children ([`cache::NodeCache::unplace`]).
    pub placements: u64,
    pub jobs: u64,
    /// What the pool threads' measuring contexts did ([`Worker::take_work`]).
    pub work: u64,
}

impl Stats {
    /// Add `o`'s counts to these.
    pub fn add(&mut self, o: Self) {
        self.calls = self.calls.saturating_add(o.calls);
        self.misses = self.misses.saturating_add(o.misses);
        self.measures = self.measures.saturating_add(o.measures);
        self.placements = self.placements.saturating_add(o.placements);
        self.jobs = self.jobs.saturating_add(o.jobs);
        self.work = self.work.saturating_add(o.work);
    }
}

/// How far a change inside a node reaches ([`Node::settle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Change {
    #[default]
    None,
    /// Something inside must lay out again, at a boundary below: the
    /// parent's own layout stands.
    Inner,
    /// The node's layout as its parent sees it may change: the parent lays out again.
    Outer,
}

/// Fewest nodes a subtree has to be worth a job of its own, and fewest in
/// all for a parallel batch (Ladybird's `parallel_layout` thresholds).
const JOB_NODES: u32 = 16;
const BATCH_NODES: u32 = 128;

/// One retained layout node.
#[derive(Debug)]
pub struct Node<S: NodeStyle, L> {
    pub style: S,
    pub leaf: Option<L>,
    pub children: Vec<Node<S, L>>,
    /// The owner's handle on what it keeps beside the node.
    pub user: u32,
    cache: NodeCache,
    layout: Layout,
    rounded: Layout,
    hoisted: Vec<NodeId>,
    foreign: Vec<(usize, u64)>,
    /// The last final-layout input its parent gave it.
    input: Option<LayoutInput>,
    /// Its parent used only `input`, since the parent's cache was cleared.
    tight: bool,
    /// Lay out alone with `input` (a change inside a boundary).
    relayout: bool,
    /// A node below lays out alone.
    pending: bool,
    /// Nodes in its subtree, itself included.
    count: u32,
    /// Its layout changed since [`Node::take_moved`].
    moved: bool,
    /// Its current run of its own algorithm for a final layout.
    run: u64,
    /// The parent's run that last gave it its final layout.
    placed_in: u64,
    /// A float is in its subtree (as last settled).
    floats: bool,
    /// An out-of-flow box is in its subtree (as last settled): its output
    /// passes it up to a containing block, so its parent uses more of it
    /// than its size.
    oof: bool,
    /// Its content changed: lay it out again for each answer in `check`
    /// and compare (the parent's results stand if all agree).
    verify: bool,
    check: Vec<cache::Answer>,
    /// Its check failed: its parent's results built on it are stale.
    failed: bool,
    /// Its last layout was in its parent's block formatting context.
    in_flow: bool,
    /// The space it was last laid out in as a root.
    root_space: Option<Size<AvailableSpace>>,
    /// A grid's tracks as last laid out, where its out-of-flow children find their grid area.
    info: taffy::DetailedLayoutInfo<S::Ident>,
    /// A flex column's items' answers (its own style and children unchanged since).
    stack: Option<Box<stack::Memo>>,
    /// A grid's tracks and its items' contributions from its last run
    /// (its own style and children unchanged since).
    grid: grid::Memo,
    /// A leaf with children: its children's boxes in its lines, as its
    /// last final layout laid them ([`inline`]).
    inline: Vec<InlineBox>,
    /// What its answers read of the height it is given (as last settled,
    /// [`Node::heights_read`]).
    reads_available_height: bool,
    width_reads_height: bool,
    /// The answer its own algorithm just gave read no available height
    /// (found as it ran; taken when the answer is cached).
    free_answer: bool,
}

/// Names each final-layout run of an algorithm (compared, never ordered):
/// each thread takes a block of names at a time, so the threads of a
/// parallel layout do not contend for one counter.
static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// How many names a thread takes at once.
const RUN_BLOCK: u64 = 1 << 16;

thread_local! {
    /// This thread's next name and the end of its block.
    static NEXT_RUN: std::cell::Cell<(u64, u64)> = const { std::cell::Cell::new((0, 0)) };
}

/// A name no other run has had.
fn next_run() -> u64 {
    NEXT_RUN.with(|next| {
        let (mut at, mut end) = next.get();
        if at >= end {
            at = RUNS.fetch_add(RUN_BLOCK, std::sync::atomic::Ordering::Relaxed);
            end = at.saturating_add(RUN_BLOCK);
        }
        next.set((at.saturating_add(1), end));
        at
    })
}

impl<S: NodeStyle, L: Measure> Node<S, L> {
    pub fn new(style: S, leaf: Option<L>) -> Self {
        Self {
            style,
            leaf,
            children: Vec::new(),
            user: 0,
            cache: NodeCache::default(),
            layout: Layout::with_order(0),
            rounded: Layout::with_order(0),
            hoisted: Vec::new(),
            foreign: Vec::new(),
            input: None,
            tight: false,
            relayout: false,
            pending: false,
            count: 1,
            moved: true,
            run: 0,
            placed_in: 0,
            info: taffy::DetailedLayoutInfo::None,
            floats: false,
            oof: false,
            verify: false,
            check: Vec::new(),
            failed: false,
            in_flow: false,
            root_space: None,
            stack: None,
            grid: grid::Memo::default(),
            inline: Vec::new(),
            reads_available_height: true,
            width_reads_height: true,
            free_answer: false,
        }
    }

    /// What its answers read, for its cache ([`cache::Reads`]).
    fn reads(&self) -> cache::Reads {
        cache::Reads {
            parent: self.style.reads_parent(),
            available_height: self.reads_available_height,
            width_reads_height: self.width_reads_height,
        }
    }

    /// Whether its answers read the available height when no height is
    /// known, and whether its width probes read the height they are given,
    /// from its style, its leaf and its children as settled. Each is a
    /// property of the algorithm that lays it out, read from its source:
    ///
    /// - A leaf ([`leaf::compute`], Taffy's `compute_leaf_layout` ported)
    ///   passes the available height to the measure and nowhere else, and
    ///   its width is the measured width unless the style's aspect ratio
    ///   derives it from the height.
    /// - A leaf with children ([`inline`]) gives it to them: it reads it.
    /// - A flex row (`compute_flexbox_layout`, single-line): the available
    ///   height becomes each item's available height, and the known height
    ///   of a stretched item whose height is not a length, but only in its
    ///   width probes (flex base size, min-content size). Such a row reads it
    ///   only through items that read it: an item reading the available
    ///   height, or a stretched one whose width probes read the height.
    /// - Anything else reads it.
    fn heights_read(&self) -> (bool, bool) {
        let style = self.style.view();
        if self.children.is_empty() {
            let m = self.leaf.as_ref().map_or(MeasureReads::WIDTH_ONLY, Measure::reads);
            let width = m.width_reads_height || style.aspect_ratio().is_some();
            return (m.available_height, width);
        }
        // A leaf with children ([`inline`]) reads it: it gives it to them.
        if self.leaf.is_some()
            || self.style.display() != Display::Flex
            || style.aspect_ratio().is_some()
            || !matches!(
                style.flex_direction(),
                taffy::FlexDirection::Row | taffy::FlexDirection::RowReverse
            )
            || style.flex_wrap() != taffy::FlexWrap::NoWrap
        {
            return (true, true);
        }
        let align_items = FlexboxContainerStyle::align_items(&style);
        let reads = self.children.iter().any(|c| {
            let item = c.style.view();
            if item.position().is_out_of_flow() || c.style.display() == Display::None {
                return false;
            }
            let align = FlexboxItemStyle::align_self(&item).unwrap_or(align_items);
            let stretched = matches!(
                align.keyword,
                taffy::AlignItemsKeyword::Stretch | taffy::AlignItemsKeyword::Normal
            ) && !matches!(item.size().height.expand(), taffy::style::ExpandedDimension::Length(_));
            c.reads_available_height || (stretched && c.width_reads_height)
        });
        (reads, true)
    }

    /// A leaf with children: their boxes in its lines as its last final
    /// layout laid them, one to a child ([`Measure::measure_inline`]).
    #[must_use]
    pub fn inline_boxes(&self) -> &[InlineBox] {
        &self.inline
    }

    /// Its layout: size and place in its parent (the containing block for
    /// an out-of-flow box), fractional.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Its layout as [`Node::round`] snapped it.
    #[must_use]
    pub fn rounded(&self) -> &Layout {
        &self.rounded
    }

    /// Whether its layout changed since the last call.
    pub fn take_moved(&mut self) -> bool {
        std::mem::take(&mut self.moved)
    }

    /// Whether its layout changed since [`Node::take_moved`] was last called.
    #[must_use]
    pub fn moved(&self) -> bool {
        self.moved
    }

    /// The run of its own algorithm that last placed its children: another
    /// than the one seen means its children may have moved.
    #[must_use]
    pub fn run(&self) -> u64 {
        self.run
    }

    /// Nodes in its subtree, as last settled.
    #[must_use]
    pub fn count(&self) -> u32 {
        self.count
    }

    /// Whether its parent fixed its size and used nothing else of it.
    #[must_use]
    pub fn is_boundary(&self) -> bool {
        self.tight && self.input.is_some()
    }

    /// Drop its cached results (its own layout must run again).
    pub fn invalidate(&mut self) {
        self.cache.clear();
        // What the children's results were used for went with the cache.
        for c in &mut self.children {
            c.tight = true;
        }
    }

    /// Settle a node its owner has patched: `style` whether its own style
    /// changed, `content` whether its leaf or its list of children did,
    /// `children` the furthest any child's change reaches. Returns how far
    /// this node's reaches, for its parent.
    ///
    /// A changed style may change what the parent reads of the node besides
    /// its answers (margins, flex factors, alignment): the parent lays out
    /// again. A change inside only changes its answers: the node is laid
    /// out again alone, at the size its parent fixed when it used nothing
    /// else (Flutter's relayout boundary), else for every answer it gave,
    /// compared with the old ones, and the parent lays out again only if
    /// one differs (as Blink stops at a box whose size did not change).
    pub fn settle(&mut self, style: bool, content: bool, children: Change) -> Change {
        self.count = self.children.iter().fold(1_u32, |n, c| n.saturating_add(c.count));
        // What its answers read changed: its cached answers, and its
        // parent's built on them, were matched on what it read before.
        let reads = self.heights_read();
        let style = style || reads != (self.reads_available_height, self.width_reads_height);
        (self.reads_available_height, self.width_reads_height) = reads;
        self.floats = self.style.view().float().is_floated() || self.children.iter().any(|c| c.floats);
        // Out-of-flow boxes inside it before or after the change: one that
        // left or joined its containing block's list is not laid out alone.
        let had_oof = self.oof;
        self.oof = self.style.view().position().is_out_of_flow() || self.children.iter().any(|c| c.oof);
        let oof = had_oof || self.oof;
        // Whatever this node's own change, a child that changed inside is
        // laid out (or checked) before it, else its flags would outlive the pass.
        if children != Change::None {
            self.pending = true;
        }
        if style || content {
            // Its items' answers were kept for this style and these children.
            self.stack = None;
            self.grid.clear();
        }
        // A box in its parent's block formatting context with a float in it
        // places that float for its siblings (and alone, in a context of
        // its own, would place it elsewhere), and one an out-of-flow box
        // joined or left changes its containing block's list: either way
        // its parent lays out again.
        let goes_up = (self.floats && self.in_flow) || had_oof != self.oof;
        if style || content || children == Change::Outer {
            if goes_up {
                self.invalidate();
                self.verify = false;
                self.check.clear();
                return Change::Outer;
            }
            if !style && !oof && self.is_boundary() {
                self.invalidate();
                self.relayout = true;
                return Change::Inner;
            }
            // An out-of-flow box inside is laid out by its containing block,
            // whose answers carry its overflow: the node's own answers
            // agreeing says nothing about it, so the change goes up.
            if !style && !self.verify {
                if let Some(check) = self.cache.take_check() {
                    self.invalidate();
                    self.check = check;
                    self.verify = true;
                    return Change::Inner;
                }
            } else if !style {
                // Checked already this pass: the old answers stand as they were.
                self.invalidate();
                return Change::Inner;
            }
            self.invalidate();
            self.verify = false;
            self.check.clear();
            return Change::Outer;
        }
        if children == Change::Inner {
            return Change::Inner;
        }
        Change::None
    }

    /// Recount every subtree (a tree built directly, not settled).
    pub fn recount(&mut self) -> u32 {
        self.count = self
            .children
            .iter_mut()
            .fold(1_u32, |n, c| n.saturating_add(c.recount()));
        let reads = self.heights_read();
        if reads != (self.reads_available_height, self.width_reads_height) {
            // Its answers were matched on what it read before.
            self.invalidate();
            (self.reads_available_height, self.width_reads_height) = reads;
        }
        self.floats = self.style.view().float().is_floated() || self.children.iter().any(|c| c.floats);
        self.oof = self.style.view().position().is_out_of_flow() || self.children.iter().any(|c| c.oof);
        self.count
    }

    /// Lay this tree out as a root in `available`: first what changed
    /// inside, bottom up (boundaries laid out alone, changed nodes checked
    /// against their old answers), then from the root down whatever that
    /// left without a cached result.
    pub fn compute_root(&mut self, available: Size<AvailableSpace>, cx: &mut L::Cx, schedule: Schedule) -> Stats {
        let mut stats = Stats::default();
        // Nothing changed: nothing to ask.
        let clean = !(self.pending || self.verify || self.relayout);
        if clean && self.cache.has_layout() && self.root_space == Some(available) {
            return stats;
        }
        self.root_space = Some(available);
        self.tight = false;
        if relayout(self, cx, schedule, &mut stats) {
            self.invalidate();
        }
        let mut view = View {
            node: &mut *self,
            cx: &mut *cx,
            schedule,
            stats: &mut stats,
        };
        taffy::compute_root_layout(&mut view, NodeId::new(SELF), available);
        stats
    }

    /// The node `id` names in this node's view.
    #[must_use]
    pub fn resolve(&self, id: NodeId) -> Option<&Self> {
        let id = u64::from(id);
        if id == SELF {
            return Some(self);
        }
        if id >= FOREIGN {
            let &(child, inner) = self.foreign.get(usize::try_from(id.saturating_sub(FOREIGN)).ok()?)?;
            return self.children.get(child)?.resolve(NodeId::new(inner));
        }
        self.children.get(usize::try_from(id).ok()?)
    }

    fn resolve_mut(&mut self, id: u64) -> Option<&mut Self> {
        if id == SELF {
            return Some(self);
        }
        if id >= FOREIGN {
            let &(child, inner) = self.foreign.get(usize::try_from(id.saturating_sub(FOREIGN)).ok()?)?;
            return self.children.get_mut(child)?.resolve_mut(inner);
        }
        self.children.get_mut(usize::try_from(id).ok()?)
    }

    /// The out-of-flow boxes it is the containing block of, as its view names them.
    #[must_use]
    pub fn hoisted(&self) -> &[NodeId] {
        &self.hoisted
    }

    fn out_of_flow(&self) -> bool {
        let s = self.style.view();
        s.position().is_out_of_flow() && self.style.display() != Display::None
    }

    /// Snap the tree's layout to whole pixels as Taffy's `round_layout`
    /// does, into [`Node::rounded`] (what Taffy's fixtures expect; `gui`
    /// itself lays out in fractions, ADR-0006).
    pub fn round(&mut self) {
        round(self, 0.0, 0.0);
    }
}

fn round<S: NodeStyle, L: Measure>(node: &mut Node<S, L>, x: f32, y: f32) {
    let u = node.layout;
    let mut l = u;
    let (cx, cy) = (x + u.location.x, y + u.location.y);
    l.location.x = cx.round() - x.round();
    l.location.y = cy.round() - y.round();
    l.size.width = (cx + u.size.width).round() - cx.round();
    l.size.height = (cy + u.size.height).round() - cy.round();
    node.rounded = l;
    for c in &mut node.children {
        if !c.out_of_flow() {
            round(c, cx, cy);
        }
    }
    let hoisted = node.hoisted.clone();
    for id in hoisted {
        if let Some(c) = node.resolve_mut(u64::from(id)) {
            round(c, cx, cy);
        }
    }
}

/// Lay out what changed in `node`'s subtree, deepest first: children that
/// wait as jobs (in parallel when there are enough), then the node itself
/// if it changed or a child's answers did. Returns whether its own answers
/// may have changed (its parent's results are then stale).
fn relayout<S: NodeStyle, L: Measure>(
    node: &mut Node<S, L>,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> bool {
    let mut stale = false;
    if node.pending {
        node.pending = false;
        let mut jobs: Vec<&mut Node<S, L>> = node
            .children
            .iter_mut()
            .filter(|c| c.relayout || c.verify || c.pending)
            .collect();
        if schedule.reverse {
            jobs.reverse();
        }
        stats.add(jobs_run(jobs, cx, schedule, &|job, cx| {
            let mut s = Stats::default();
            job.failed = relayout(job, cx, schedule, &mut s);
            s
        }));
        for c in &mut node.children {
            stale |= std::mem::take(&mut c.failed);
        }
    }
    let mut outer = false;
    if node.verify || stale {
        let check = if node.verify {
            Some(std::mem::take(&mut node.check))
        } else {
            node.cache.take_check()
        };
        node.verify = false;
        node.invalidate();
        match check {
            // An out-of-flow box inside is laid out by its containing block,
            // whose answers carry its overflow: the node's own answers
            // agreeing says nothing about it, so the change goes up.
            Some(check) => outer = !verify(node, check, cx, schedule, stats) || node.oof,
            None if node.is_boundary() && !node.oof => node.relayout = true,
            None => outer = true,
        }
    }
    if node.relayout {
        node.relayout = false;
        if let Some(input) = node.input {
            let out = compute(node, input, None, cx, schedule, stats);
            // Its size stands, but its parent also takes its scrollable
            // overflow (and its own reaches the ancestors' while it shows):
            // when that moved, the parent lays out again, as Blink recalculates
            // overflow up the tree.
            if out.scrollable_overflow_rect != node.layout.scrollable_overflow_rect {
                outer = true;
            }
        }
    }
    outer
}

/// Lay `node` out again for each answer it gave; whether every new answer
/// equals the old, stopping at the first that does not. Final layouts run
/// last, so its children are left placed for the one its parent used.
fn verify<S: NodeStyle, L: Measure>(
    node: &mut Node<S, L>,
    mut check: Vec<cache::Answer>,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> bool {
    check.sort_by_key(|a| a.input.run_mode == RunMode::PerformLayout);
    for answer in check {
        let output = if answer.in_flow {
            // Laid out in a block formatting context that had no floats (a
            // box in one with floats is never cached): a fresh one is the same.
            let mut bfc = BlockFormattingContext::new();
            let mut root = bfc.root_block_context();
            let mut ctx = root.sub_context(0.0, [0.0, 0.0]);
            compute(node, answer.input, Some(&mut ctx), cx, schedule, stats)
        } else {
            compute(node, answer.input, None, cx, schedule, stats)
        };
        // An old answer that read no available height may have answered the
        // parent under other heights too: it stands only if the new one is as free.
        let same = match answer.input.run_mode {
            RunMode::ComputeSize => output.size == answer.output.size,
            _ => output == answer.output,
        } && (!answer.free_height || node.cache.is_free(&answer.input, node.reads()));
        // One answer differs: the parent lays out again, asking what it
        // needs (what this run found is cached for it).
        if !same {
            return false;
        }
    }
    true
}

/// Run `run` on each of `jobs`: on the layout pool when there are enough
/// of them, grouped so each job is worth sending, else here in order.
fn jobs_run<'a, S: NodeStyle, L: Measure>(
    jobs: Vec<&'a mut Node<S, L>>,
    cx: &mut L::Cx,
    schedule: Schedule,
    run: &Job<'_, S, L>,
) -> Stats {
    let total = jobs.iter().fold(0_u32, |n, j| n.saturating_add(j.count));
    if !schedule.parallel || jobs.len() < 2 || total < BATCH_NODES {
        let mut stats = Stats::default();
        for job in jobs {
            stats.add(run(job, cx));
        }
        return stats;
    }
    // Groups of at least `JOB_NODES` nodes, at most a few per thread.
    let threads = u32::try_from(pool().current_num_threads()).unwrap_or(1).max(1);
    let size = total.checked_div(threads.saturating_mul(4)).unwrap_or(0).max(JOB_NODES);
    let mut groups: Vec<Vec<&'a mut Node<S, L>>> = Vec::new();
    let mut current: Vec<&'a mut Node<S, L>> = Vec::new();
    let mut weight = 0_u32;
    for job in jobs {
        weight = weight.saturating_add(job.count);
        current.push(job);
        if weight >= size {
            groups.push(std::mem::take(&mut current));
            weight = 0;
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    let mut stats = S::run_jobs(&mut groups, cx, run);
    stats.jobs = stats
        .jobs
        .saturating_add(u64::try_from(groups.len()).unwrap_or(u64::MAX));
    stats
}

/// Lay `node` out for `input`, from its cache when it holds the answer.
fn compute<S: NodeStyle, L: Measure>(
    node: &mut Node<S, L>,
    input: LayoutInput,
    block: Option<&mut BlockContext<'_>>,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> LayoutOutput {
    stats.calls = stats.calls.saturating_add(1);
    let mut view = View {
        node,
        cx,
        schedule,
        stats,
    };
    if input.run_mode == RunMode::PerformHiddenLayout {
        view.node.layout = Layout::with_order(0);
        return taffy::compute_hidden_layout(&mut view, NodeId::new(SELF));
    }
    // In a block formatting context with floats, a box's layout reads where
    // they are, and one with floats in it places them for its siblings:
    // neither is in a cache key, so such a box lays out each time.
    view.node.in_flow = block.is_some();
    if block.as_ref().is_some_and(|b| b.has_floats() || view.node.floats) {
        view.node.cache.lose();
        // A final layout not kept places the subtree for itself: the one
        // kept from an earlier run no longer answers as a final layout.
        if input.run_mode == RunMode::PerformLayout {
            view.node.cache.unplace();
        }
        return run(&mut view, input, block);
    }
    taffy::compute_cached_layout(&mut view, NodeId::new(SELF), input, |view, _, input| {
        run(view, input, block)
    })
}

/// Lay `node` out for `input`, outside any block formatting context, its
/// cache just found not to hold the answer: its own algorithm runs and the
/// answer is kept (`compute` without looking it up a second time).
fn compute_missed<S: NodeStyle, L: Measure>(
    node: &mut Node<S, L>,
    input: LayoutInput,
    cx: &mut L::Cx,
    schedule: Schedule,
    stats: &mut Stats,
) -> LayoutOutput {
    stats.calls = stats.calls.saturating_add(1);
    let mut view = View {
        node,
        cx,
        schedule,
        stats,
    };
    view.node.in_flow = false;
    let output = run(&mut view, input, None);
    view.cache_store(NodeId::new(SELF), &input, output.clone());
    output
}

/// Run `view`'s node's own algorithm for `input` (a cache miss).
fn run<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    input: LayoutInput,
    block: Option<&mut BlockContext<'_>>,
) -> LayoutOutput {
    view.stats.misses = view.stats.misses.saturating_add(1);
    view.node.free_answer = false;
    let placements = view.stats.placements;
    if input.run_mode == RunMode::PerformLayout {
        // The ids out-of-flow descendants get here are named afresh by each
        // final layout. A size probe keeps them: the final layout cached
        // still hands them out, and a probe names nothing it keeps.
        view.node.foreign.clear();
        view.node.relayout = false;
        view.node.run = next_run();
    }
    let me = NodeId::new(SELF);
    let mut output = match (view.node.style.display(), view.node.children.is_empty()) {
        (Display::None, _) => taffy::compute_hidden_layout(view, me),
        // A leaf with children lays them in its content (a paragraph
        // holding elements): an inline formatting context.
        (_, false) if view.node.leaf.is_some() => inline::compute(view, input),
        (Display::Block, false) => block::compute(view, input, block),
        (Display::FlowRoot, false) => block::compute(view, input, None),
        // A container with its own ratio is the general algorithms' (its
        // size depends on its content: `sizing.rs`).
        (Display::Flex, false) if sizing::ratio_of(&view.node.style.view()).is_some() => flex::compute(view, input),
        (Display::Flex, false) => match row::compute(view, input) {
            Some(out) => out,
            None => match stack::compute(view, input) {
                Some(out) => out,
                None => flex::compute(view, input),
            },
        },
        (Display::Grid, false) if sizing::ratio_of(&view.node.style.view()).is_some() => grid::fallback(view, input),
        (Display::Grid, false) => match grid::compute(view, input) {
            Some(out) => out,
            None => grid::fallback(view, input),
        },
        (_, true) => {
            let View { node, cx, stats, .. } = view;
            let style = node.style.view();
            let cache = &mut node.cache;
            let leaf = node.leaf.as_ref();
            let alone = leaf.is_some_and(|l| l.reads().width_alone);
            leaf::compute(
                input,
                &style,
                S::calc,
                leaf.is_some_and(Measure::replaced),
                |known, available| match leaf {
                    Some(leaf) => {
                        let width = known.width.map_or(available.width, AvailableSpace::Definite);
                        if alone && let Some(m) = cache.measured(width) {
                            return m;
                        }
                        stats.measures = stats.measures.saturating_add(1);
                        let m = leaf.measure(cx, known, available);
                        if alone {
                            cache.keep_measured(width, m);
                        }
                        m
                    }
                    None => Measured::size(Size::ZERO),
                },
            )
        }
    };
    if input.run_mode == RunMode::PerformLayout {
        taffy::compute_oof_layout(view, me, &mut output);
        if !view.node.children.is_empty() {
            view.stats.placements = view.stats.placements.saturating_add(1);
        }
    } else if view.stats.placements != placements {
        // A probe that placed something below: the final layout cached
        // is no longer what the subtree shows.
        view.node.cache.unplace();
    }
    output
}

/// One node and its children, as Taffy's algorithms see them.
struct View<'a, S: NodeStyle, L: Measure> {
    node: &'a mut Node<S, L>,
    cx: &'a mut L::Cx,
    schedule: Schedule,
    stats: &'a mut Stats,
}

impl<S: NodeStyle, L: Measure> View<'_, S, L> {
    fn get(&self, id: NodeId) -> &Node<S, L> {
        self.node.resolve(id).unwrap_or(&*self.node)
    }

    /// The id, in this view, of `inner` as child `child`'s view names it.
    fn foreign_id(&mut self, child: usize, inner: u64) -> u64 {
        let at = self
            .node
            .foreign
            .iter()
            .position(|&e| e == (child, inner))
            .unwrap_or_else(|| {
                self.node.foreign.push((child, inner));
                self.node.foreign.len().saturating_sub(1)
            });
        FOREIGN.saturating_add(u64::try_from(at).unwrap_or(0))
    }

    /// Taffy's `LayoutPartialTreeExt::measure_child_size`.
    #[expect(clippy::too_many_arguments, reason = "Taffy's signature")]
    fn measure_child_size(
        &mut self,
        id: NodeId,
        known_dimensions: Size<Option<f32>>,
        parent_size: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
        sizing_mode: taffy::SizingMode,
        axis: taffy::AbsoluteAxis,
        vertical_margins_are_collapsible: taffy::Line<bool>,
    ) -> f32 {
        self.child_layout(
            id,
            LayoutInput {
                known_dimensions,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size,
                available_space,
                sizing_mode,
                axis: axis.into(),
                run_mode: RunMode::ComputeSize,
                vertical_margins_are_collapsible,
            },
            None,
        )
        .size
        .get_abs(axis)
    }

    /// Taffy's `LayoutPartialTreeExt::perform_child_layout`.
    fn perform_child_layout(
        &mut self,
        id: NodeId,
        known_dimensions: Size<Option<f32>>,
        parent_size: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
        sizing_mode: taffy::SizingMode,
        vertical_margins_are_collapsible: taffy::Line<bool>,
    ) -> LayoutOutput {
        self.child_layout(
            id,
            LayoutInput {
                known_dimensions,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size,
                available_space,
                sizing_mode,
                axis: taffy::RequestedAxis::Both,
                run_mode: RunMode::PerformLayout,
                vertical_margins_are_collapsible,
            },
            None,
        )
    }

    fn child_layout(&mut self, id: NodeId, input: LayoutInput, block: Option<&mut BlockContext<'_>>) -> LayoutOutput {
        let raw = u64::from(id);
        if raw == SELF {
            return compute(self.node, input, block, self.cx, self.schedule, self.stats);
        }
        if raw >= FOREIGN {
            let Some(&(child, inner)) = usize::try_from(raw.saturating_sub(FOREIGN))
                .ok()
                .and_then(|i| self.node.foreign.get(i))
            else {
                return LayoutOutput::HIDDEN;
            };
            let Some(c) = self.node.children.get_mut(child) else {
                return LayoutOutput::HIDDEN;
            };
            let mut view = View {
                node: c,
                cx: &mut *self.cx,
                schedule: self.schedule,
                stats: &mut *self.stats,
            };
            let mut out = view.child_layout(NodeId::new(inner), input, block);
            self.adopt(child, &mut out);
            return out;
        }
        let Ok(i) = usize::try_from(raw) else {
            return LayoutOutput::HIDDEN;
        };
        let baseline = self.node.style.baseline();
        let run = self.node.run;
        let Some(c) = self.node.children.get_mut(i) else {
            return LayoutOutput::HIDDEN;
        };
        // A block in its parent's formatting context also hands its parent
        // the margins that collapse through it: the parent uses more than
        // its size.
        let fixed = input.run_mode == RunMode::PerformLayout
            && input.known_dimensions.width.is_some()
            && input.known_dimensions.height.is_some()
            && !input.vertical_margins_are_collapsible.start
            && !input.vertical_margins_are_collapsible.end
            && !baseline
            && !c.style.baseline();
        c.tight &= fixed;
        if input.run_mode == RunMode::PerformLayout {
            c.input = Some(input);
            c.placed_in = run;
        }
        // Whether the child's cache was just found not to hold the answer
        // (and nothing has laid it out since): it is not looked up again.
        let mut missed = false;
        if block.is_none() && input.run_mode != RunMode::PerformHiddenLayout {
            let reads = c.reads();
            if let Some(mut out) = c.cache.get(&input, reads) {
                c.in_flow = false;
                self.stats.calls = self.stats.calls.saturating_add(1);
                self.adopt(i, &mut out);
                return out;
            }
            missed = !(self.schedule.parallel && self.speculate(i, &input));
        }
        let Some(c) = self.node.children.get_mut(i) else {
            return LayoutOutput::HIDDEN;
        };
        let mut out = if missed {
            compute_missed(c, input, self.cx, self.schedule, self.stats)
        } else {
            compute(c, input, block, self.cx, self.schedule, self.stats)
        };
        self.adopt(i, &mut out);
        out
    }

    /// Name the out-of-flow boxes child `child` passed up in this view.
    fn adopt(&mut self, child: usize, out: &mut LayoutOutput) {
        if out.oof_candidates.is_empty() {
            return;
        }
        let ids: Vec<u64> = out
            .oof_candidates
            .as_slice()
            .iter()
            .map(|c| self.foreign_id(child, u64::from(c.node)))
            .collect();
        for (c, id) in out.oof_candidates.as_mut_slice().iter_mut().zip(ids) {
            c.node = NodeId::new(id);
        }
    }

    /// Child `i` is asked `input`, which its cache does not hold: ask its
    /// alike siblings the same at once, in parallel, so the algorithm finds
    /// their answers cached when it comes to them.
    /// Whether it laid any out.
    fn speculate(&mut self, i: usize, input: &LayoutInput) -> bool {
        let kids = &self.node.children;
        // Too few nodes below for any batch to be worth sending.
        if kids.len() < 2 || self.node.count.saturating_sub(1) < BATCH_NODES {
            return false;
        }
        let Some(first) = kids.get(i) else { return false };
        if first.leaf.is_some() {
            return false;
        }
        // A final layout places a node's own children, so it is put only to
        // siblings this run of the algorithm has yet to lay out: each is then
        // asked this same input and finds it cached, or another and lays out
        // again over it. Those it laid out already keep theirs. And only to
        // siblings with no final layout cached (laid out for the first time,
        // or changed): one that has one was most likely placed at its own
        // size, which this sibling's need not be.
        let run = (input.run_mode == RunMode::PerformLayout).then_some(self.node.run);
        let same: Vec<bool> = kids
            .iter()
            .enumerate()
            .map(|(j, c)| {
                (j == i || run.is_none_or(|r| c.placed_in != r && !c.cache.has_layout()))
                    && c.leaf.is_none()
                    && c.style.view().position() == first.style.view().position()
                    && c.style.display() == first.style.display()
                    && !c.cache.has(input, c.reads())
            })
            .collect();
        let chosen: Vec<&mut Node<S, L>> = self
            .node
            .children
            .iter_mut()
            .zip(same)
            .filter_map(|(c, s)| s.then_some(c))
            .collect();
        let total = chosen.iter().fold(0_u32, |n, c| n.saturating_add(c.count));
        if chosen.len() < 2 || total < BATCH_NODES {
            return false;
        }
        let schedule = self.schedule;
        let input = *input;
        self.stats.add(jobs_run(chosen, self.cx, schedule, &|c, cx| {
            let mut s = Stats::default();
            compute(c, input, None, cx, schedule, &mut s);
            s
        }));
        true
    }
}

struct Children(std::ops::Range<u64>);

impl Iterator for Children {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        self.0.next().map(NodeId::new)
    }
}

impl<S: NodeStyle, L: Measure> TraversePartialTree for View<'_, S, L> {
    type ChildIter<'a>
        = Children
    where
        Self: 'a;

    fn child_ids(&self, _: NodeId) -> Children {
        Children(0..u64::try_from(self.node.children.len()).unwrap_or(0))
    }

    fn child_count(&self, _: NodeId) -> usize {
        self.node.children.len()
    }

    fn get_child_id(&self, _: NodeId, index: usize) -> NodeId {
        NodeId::new(u64::try_from(index).unwrap_or(0))
    }
}

impl<S: NodeStyle, L: Measure> LayoutPartialTree for View<'_, S, L> {
    type CoreContainerStyle<'a>
        = S::Ref<'a>
    where
        Self: 'a;
    type CustomIdent = S::Ident;

    fn get_core_container_style(&self, id: NodeId) -> S::Ref<'_> {
        self.get(id).style.view()
    }

    fn resolve_calc_value(&self, val: *const (), basis: f32) -> f32 {
        S::calc(val, basis)
    }

    fn set_unrounded_layout(&mut self, id: NodeId, layout: &Layout) {
        if let Some(n) = self.node.resolve_mut(u64::from(id))
            && n.layout != *layout
        {
            n.layout = *layout;
            n.moved = true;
        }
    }

    fn compute_child_layout(&mut self, id: NodeId, input: LayoutInput) -> LayoutOutput {
        self.child_layout(id, input, None)
    }
}

impl<S: NodeStyle, L: Measure> LayoutContainingBlock for View<'_, S, L> {
    type OofItemStyle<'a>
        = S::Ref<'a>
    where
        Self: 'a;

    fn get_oof_item_style(&self, id: NodeId) -> S::Ref<'_> {
        self.get(id).style.view()
    }

    fn clear_hoisted_children(&mut self, id: NodeId) {
        if let Some(n) = self.node.resolve_mut(u64::from(id)) {
            n.hoisted.clear();
        }
    }

    fn add_hoisted_children(&mut self, id: NodeId, hoisted: &[NodeId]) {
        if let Some(n) = self.node.resolve_mut(u64::from(id)) {
            n.hoisted.extend_from_slice(hoisted);
        }
    }

    fn get_detailed_layout_info(&self, id: NodeId) -> &taffy::DetailedLayoutInfo<S::Ident> {
        &self.get(id).info
    }
}

impl<S: NodeStyle, L: Measure> CacheTree for View<'_, S, L> {
    fn cache_get(&mut self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        let n = self.node.resolve_mut(u64::from(id))?;
        let reads = n.reads();
        n.cache.get(input, reads)
    }

    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        if let Some(n) = self.node.resolve_mut(u64::from(id)) {
            let in_flow = n.in_flow;
            let free = std::mem::take(&mut n.free_answer);
            n.cache.store(input, &output, in_flow, free);
        }
    }

    fn cache_clear(&mut self, id: NodeId) {
        if let Some(n) = self.node.resolve_mut(u64::from(id)) {
            n.cache.clear();
        }
    }
}

impl<S: NodeStyle, L: Measure> LayoutGridContainer for View<'_, S, L> {
    type GridContainerStyle<'a>
        = S::Ref<'a>
    where
        Self: 'a;
    type GridItemStyle<'a>
        = S::Ref<'a>
    where
        Self: 'a;

    fn get_grid_container_style(&self, id: NodeId) -> S::Ref<'_> {
        self.get(id).style.view()
    }

    fn get_grid_child_style(&self, id: NodeId) -> S::Ref<'_> {
        self.get(id).style.view()
    }

    fn set_detailed_grid_info(&mut self, id: NodeId, info: taffy::DetailedGridInfo<S::Ident>) {
        if let Some(n) = self.node.resolve_mut(u64::from(id)) {
            n.info = taffy::DetailedLayoutInfo::Grid(Box::new(info));
        }
    }
}

/// Taffy's own style, read as it is: for its fixtures and benchmarks.
impl NodeStyle for taffy::Style<String> {
    type Ident = String;
    type Ref<'a> = &'a Self;

    fn view(&self) -> &Self {
        self
    }

    fn display(&self) -> Display {
        self.display
    }

    fn reads_parent(&self) -> Size<bool> {
        use taffy::style::CompactLength;
        let reads = |c: CompactLength| c.uses_percentage();
        let x = reads(self.size.width.into_raw())
            || reads(self.min_size.width.into_raw())
            || reads(self.max_size.width.into_raw())
            || reads(self.inset.left.into_raw())
            || reads(self.inset.right.into_raw())
            || [self.margin.left, self.margin.right, self.margin.top, self.margin.bottom]
                .iter()
                .any(|m| reads(m.into_raw()))
            || [
                self.padding.left,
                self.padding.right,
                self.padding.top,
                self.padding.bottom,
            ]
            .iter()
            .chain([self.border.left, self.border.right, self.border.top, self.border.bottom].iter())
            .any(|p| reads(p.into_raw()));
        let y = reads(self.size.height.into_raw())
            || reads(self.min_size.height.into_raw())
            || reads(self.max_size.height.into_raw())
            || reads(self.inset.top.into_raw())
            || reads(self.inset.bottom.into_raw());
        Size { width: x, height: y }
    }

    fn baseline(&self) -> bool {
        self.align_items == taffy::AlignItems::BASELINE
            || self.align_self == Some(taffy::AlignItems::BASELINE)
            || self.justify_items == taffy::AlignItems::BASELINE
            || self.justify_self == Some(taffy::AlignItems::BASELINE)
    }

    fn calc(_: *const (), _: f32) -> f32 {
        0.0
    }
}
