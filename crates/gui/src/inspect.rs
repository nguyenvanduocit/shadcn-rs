//! Debug views of a laid-out frame: a text dump of the tree, the difference
//! between two frames, and the report of one frame.
//!
//! Pure functions over [`FrameNode`]s; painting them is the overlay's
//! job ([`crate::overlay::record_overlay`]).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

pub use crate::geometry::Bounds;
use crate::geometry::NodePath;
use crate::heap::{Heap, ThreadHeap};
pub use crate::layout::Stats as LayoutStats;
use crate::layout::{Content, FrameNode};
use crate::reactive::{Graph, ReactiveTrace};

/// Positions closer than this are treated as equal, so float noise from the
/// layout engine never reads as a shift.
const EPSILON: f32 = 0.5;

fn moved(after: &Bounds, before: &Bounds) -> bool {
    (after.x - before.x).abs() > EPSILON || (after.y - before.y).abs() > EPSILON
}

fn resized(after: &Bounds, before: &Bounds) -> bool {
    (after.width - before.width).abs() > EPSILON || (after.height - before.height).abs() > EPSILON
}

/// How one node differs between two frames.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Added {
        after: Bounds,
    },
    Removed {
        before: Bounds,
    },
    /// The node is in both frames and moved, resized or paints differently.
    Updated {
        before: Bounds,
        after: Bounds,
        repainted: bool,
    },
}

impl Change {
    #[must_use]
    pub fn moved(&self) -> bool {
        matches!(self, Self::Updated { before, after, .. } if moved(after, before))
    }

    #[must_use]
    pub fn resized(&self) -> bool {
        matches!(self, Self::Updated { before, after, .. } if resized(after, before))
    }

    #[must_use]
    pub fn repainted(&self) -> bool {
        matches!(self, Self::Updated { repainted: true, .. })
    }
}

/// One changed node: where it is, what it is, and how it changed.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeChange {
    pub path: NodePath,
    pub what: String,
    pub change: Change,
}

/// Every node that differs between two frames, matched by [`NodePath`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutDiff {
    /// Shared: the pipeline keeps the last one and each frame report its own.
    pub changes: std::rc::Rc<[NodeChange]>,
    /// Nodes in the new frame.
    pub nodes: usize,
}

impl LayoutDiff {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    fn count(&self, f: impl Fn(&Change) -> bool) -> usize {
        self.changes.iter().filter(|c| f(&c.change)).count()
    }

    /// The largest distance any node that stayed in the frame moved, in pixels.
    #[must_use]
    pub fn largest_shift(&self) -> f32 {
        self.changes
            .iter()
            .filter_map(|c| match c.change {
                Change::Updated { before, after, .. } => {
                    Some((after.x - before.x).abs().max((after.y - before.y).abs()))
                }
                _ => None,
            })
            .fold(0.0, f32::max)
    }
}

impl fmt::Display for LayoutDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let added = self.count(|c| matches!(c, Change::Added { .. }));
        let removed = self.count(|c| matches!(c, Change::Removed { .. }));
        let shifted = self.count(Change::moved);
        let grown = self.count(Change::resized);
        let repainted = self.count(Change::repainted);
        write!(
            f,
            "{} nodes, {} changed: {added} added, {removed} removed, {shifted} moved, {grown} resized, {repainted} repainted",
            self.nodes,
            self.changes.len()
        )?;
        if shifted > 0 {
            write!(f, " (largest shift {:.0}px)", self.largest_shift())?;
        }
        for c in self.changes.iter() {
            write!(f, "\n  {} {:<14} {:<28} ", sign(&c.change), c.path.to_string(), c.what)?;
            match &c.change {
                Change::Added { after } => write!(f, "{after}")?,
                Change::Removed { before } => write!(f, "was {before}")?,
                Change::Updated {
                    before,
                    after,
                    repainted,
                } => {
                    let mut parts = Vec::new();
                    if moved(after, before) {
                        parts.push(format!("moved ({:+.0},{:+.0})", after.x - before.x, after.y - before.y));
                    }
                    if resized(after, before) {
                        parts.push(format!(
                            "resized {:.0}x{:.0} -> {:.0}x{:.0}",
                            before.width, before.height, after.width, after.height
                        ));
                    }
                    if *repainted {
                        parts.push("repainted".to_owned());
                    }
                    f.write_str(&parts.join(", "))?;
                }
            }
        }
        Ok(())
    }
}

fn sign(change: &Change) -> char {
    match change {
        Change::Added { .. } => '+',
        Change::Removed { .. } => '-',
        Change::Updated { .. } => '~',
    }
}

/// Compare two frames node by node. A node is the same node in both frames
/// when it has the same [`NodePath`], so a keyed list keeps its items matched
/// when they reorder and an unkeyed list matches by position.
#[must_use]
pub fn diff(before: &[FrameNode], after: &[FrameNode]) -> LayoutDiff {
    // Both lists are in tree order: what lines up at the front and at the
    // back is the same node, compared without a lookup; only the middle,
    // where nodes came and went, is matched by path.
    let same = |(b, a): (&FrameNode, &FrameNode)| b.path == a.path;
    let front = before.iter().zip(after).take_while(|&p| same(p)).count();
    let (before_rest, after_rest) = (before.get(front..).unwrap_or(&[]), after.get(front..).unwrap_or(&[]));
    let back = before_rest
        .iter()
        .rev()
        .zip(after_rest.iter().rev())
        .take_while(|&p| same(p))
        .count();
    let cut = |l: &[FrameNode]| l.len().saturating_sub(back);
    let (old_mid, new_mid) = (
        before_rest.get(..cut(before_rest)).unwrap_or(&[]),
        after_rest.get(..cut(after_rest)).unwrap_or(&[]),
    );
    let mut changes = Vec::new();
    let mut push = |now: &FrameNode, change: Option<Change>| {
        if let Some(change) = change {
            changes.push(NodeChange {
                path: now.path.clone(),
                what: describe(now),
                change,
            });
        }
    };
    let pairs = |b: &[FrameNode], a: &[FrameNode], push: &mut dyn FnMut(&FrameNode, Option<Change>)| {
        for (was, now) in b.iter().zip(a) {
            push(now, updated(was, now));
        }
    };
    pairs(
        before.get(..front).unwrap_or(&[]),
        after.get(..front).unwrap_or(&[]),
        &mut push,
    );
    let old: HashMap<&NodePath, &FrameNode> = old_mid.iter().map(|r| (&r.path, r)).collect();
    let mut seen: HashSet<&NodePath> = HashSet::with_capacity(new_mid.len());
    for now in new_mid {
        seen.insert(&now.path);
        let change = match old.get(&now.path) {
            None => Some(Change::Added { after: now.bounds }),
            Some(was) => updated(was, now),
        };
        push(now, change);
    }
    pairs(
        before_rest.get(cut(before_rest)..).unwrap_or(&[]),
        after_rest.get(cut(after_rest)..).unwrap_or(&[]),
        &mut push,
    );
    for was in old_mid.iter().filter(|r| !seen.contains(&r.path)) {
        push(was, Some(Change::Removed { before: was.bounds }));
    }

    LayoutDiff {
        changes: changes.into(),
        nodes: after.len(),
    }
}

/// How a node that stayed changed, if it did.
/// The changes of the nodes a frame's passes wrote in place (`log`), the
/// first entry of a node giving its box before; nodes from `limit` on were
/// rebuilt since and are compared whole.
pub(crate) fn logged(frame: &crate::layout::Frame, log: &[crate::layout::Changed], limit: usize) -> Vec<NodeChange> {
    let mut all: Vec<&crate::layout::Changed> = log
        .iter()
        .filter(|c| usize::try_from(c.index).unwrap_or(usize::MAX) < limit)
        .collect();
    // By node, each node's entries in the order written: its first holds its box before.
    all.sort_by_key(|c| c.index);
    let mut out: Vec<(u32, Bounds, bool)> = Vec::new();
    for c in all {
        match out.last_mut() {
            Some((i, _, repainted)) if *i == c.index => *repainted |= c.repainted,
            _ => out.push((c.index, c.before, c.repainted)),
        }
    }
    out.into_iter()
        .filter_map(|(i, before, repainted)| {
            let now = frame.nodes.get(usize::try_from(i).ok()?)?;
            (repainted || moved(&now.bounds, &before) || resized(&now.bounds, &before)).then(|| NodeChange {
                path: now.path.clone(),
                what: describe(now),
                change: Change::Updated {
                    before,
                    after: now.bounds,
                    repainted,
                },
            })
        })
        .collect()
}

fn updated(was: &FrameNode, now: &FrameNode) -> Option<Change> {
    let repainted = was.look != now.look || was.content != now.content;
    (repainted || moved(&now.bounds, &was.bounds) || resized(&now.bounds, &was.bounds)).then_some(Change::Updated {
        before: was.bounds,
        after: now.bounds,
        repainted,
    })
}

/// A short name for a node: its debug name, or what it shows
/// (`Div #ffffff`, `Text "Agents"`, `Icon chevron-down`, `Input "abc"`).
#[must_use]
pub fn describe(node: &FrameNode) -> String {
    let what = match &node.content {
        Content::Text { content, .. } => format!("Text {:?}", clip(&crate::text::readable(content))),
        Content::Icon { icon, .. } => format!("Icon {}", icon.name()),
        Content::Input { edit, .. } => format!("Input {:?}", clip(&edit.text)),
        Content::Figures(shapes) => format!("Figures x{}", shapes.len()),
        Content::Drawn(_) => "Drawing".to_owned(),
        Content::Image { source, .. } => {
            let (w, h) = source.size();
            format!("Image {w}x{h}")
        }
        Content::None => match node.look.background {
            Some(bg) => format!("Div {bg}"),
            None => "Div".to_owned(),
        },
    };
    match node.name {
        Some(name) => format!("{name}: {what}"),
        None => what,
    }
}

fn clip(s: &str) -> String {
    const MAX: usize = 24;
    if s.chars().count() <= MAX {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(MAX).collect();
    out.push('…');
    out
}

/// The whole frame as an indented tree, one node per line, in paint order.
#[must_use]
pub fn dump(nodes: &[FrameNode]) -> String {
    let mut out = String::new();
    for r in nodes {
        let indent = "  ".repeat(r.path.depth());
        let detail = match &r.content {
            Content::Text { text, .. } => format!("  {}px w{} {}", text.size, text.weight, text.color),
            _ => String::new(),
        };
        let layer = if r.layer > 0 {
            format!(" layer {}", r.layer)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{indent}{:<12} {} {}{detail}{layer}\n",
            r.path.to_string(),
            describe(r),
            r.bounds
        ));
    }
    out
}

/// The first node that shows exactly `text`: a text node, or an input
/// showing it as its value or placeholder.
#[must_use]
pub fn find_text<'a>(nodes: &'a [FrameNode], text: &str) -> Option<&'a FrameNode> {
    nodes.iter().find(|r| match &r.content {
        Content::Text { content, .. } => content == text,
        Content::Input { edit, placeholder, .. } => edit.text == text || (edit.text.is_empty() && placeholder == text),
        Content::Icon { .. } | Content::Figures(_) | Content::Drawn(_) | Content::Image { .. } | Content::None => false,
    })
}

/// Time spent in each stage of one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timings {
    /// Running the component to produce the element tree.
    pub build: Duration,
    pub layout: Duration,
    /// Recording the scene and handing it to the renderer.
    pub paint: Duration,
    /// Waiting for the display to take a frame (a GPU window waits for the
    /// next vertical blank): the frame's pace, not work, so not in [`Self::total`].
    pub vsync: Duration,
}

impl Timings {
    /// The whole frame: build, layout and paint.
    #[must_use]
    pub fn total(&self) -> Duration {
        self.build.saturating_add(self.layout).saturating_add(self.paint)
    }
}

/// How a run of values (frame times, allocations) spreads: its median,
/// 95th percentile and worst.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spread<T = Duration> {
    pub p50: T,
    pub p95: T,
    pub max: T,
}

impl<T: Ord + Copy + Default> Spread<T> {
    /// The spread of `values`; all default (zero) when there are none.
    #[must_use]
    pub fn of(values: impl IntoIterator<Item = T>) -> Self {
        let mut sorted: Vec<T> = values.into_iter().collect();
        sorted.sort_unstable();
        // The nearest-rank percentile: the smallest value at least `p` of them do not exceed.
        let at = |p: usize| {
            let rank = sorted.len().saturating_mul(p).div_ceil(100);
            sorted.get(rank.saturating_sub(1)).copied().unwrap_or_default()
        };
        Self {
            p50: at(50),
            p95: at(95),
            max: sorted.last().copied().unwrap_or_default(),
        }
    }
}

/// Images held: how many and their pixels' bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Raster {
    pub count: usize,
    pub bytes: usize,
}

/// What the app holds after a frame: the caches kept across frames, the
/// reactive graph, and the heap when it is counted ([`crate::heap`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    /// Nodes in the frame.
    pub nodes: usize,
    /// Paragraphs shaped.
    pub shaped: usize,
    pub glyphs: Raster,
    pub icons: Raster,
    /// Images and rasterised shapes, as textures.
    pub images: Raster,
    /// Paths binned on the GPU: their segment, tile and instance buffers.
    pub paths: Raster,
    /// Layer textures groups paint aside into.
    pub layers: Raster,
    /// Paint chunks whose instances the GPU keeps (ADR-0016).
    pub chunks: Raster,
    pub graph: Graph,
    /// What this frame allocated in the whole process (`allocations`,
    /// `allocated`) and what the heap holds after it (`live`, `peak`).
    pub heap: Option<Heap>,
    /// What the thread that built this frame allocated while building it
    /// (ADR-0023): the frame's allocation budget is judged by it.
    pub thread: Option<ThreadHeap>,
    /// What that thread allocated of it before paint recorded the frame:
    /// taking the input state in, running the bindings, laying out.
    pub built: Option<ThreadHeap>,
    /// What recording the frame's paint allocated of it: the display
    /// lists of the chunks recorded again and the commit that carries them.
    pub recorded: Option<ThreadHeap>,
    /// What handing the commit to its target allocated on this thread:
    /// the render thread's link sends it; a headless target composes and
    /// draws it here, the render thread's work in an app.
    pub committed: Option<ThreadHeap>,
}

impl Footprint {
    /// The bytes of every image the caches hold.
    #[must_use]
    pub fn raster_bytes(&self) -> usize {
        [self.glyphs, self.icons, self.images, self.paths, self.layers]
            .iter()
            .fold(0, |n, r| n.saturating_add(r.bytes))
    }
}

/// One owner in the memory report (ADR-0023): what it holds, in its own
/// unit, and the bytes it knows of. A heap owner counts its containers at
/// their capacity (what their elements own beyond them aside); a GPU owner
/// counts its textures and buffers exactly, outside the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub name: &'static str,
    pub count: usize,
    pub unit: &'static str,
    pub bytes: u64,
    pub gpu: bool,
}

impl Owner {
    #[must_use]
    pub fn heap(name: &'static str, count: usize, unit: &'static str, bytes: u64) -> Self {
        Self {
            name,
            count,
            unit,
            bytes,
            gpu: false,
        }
    }

    #[must_use]
    pub fn gpu(name: &'static str, held: Raster, unit: &'static str) -> Self {
        Self {
            name,
            count: held.count,
            unit,
            bytes: u64::try_from(held.bytes).unwrap_or(u64::MAX),
            gpu: true,
        }
    }
}

/// `n` values of `T`, in bytes.
#[must_use]
pub fn bytes_of<T>(n: usize) -> u64 {
    u64::try_from(n.saturating_mul(std::mem::size_of::<T>())).unwrap_or(u64::MAX)
}

/// Everything one frame did: why it ran, what the reactive runtime did,
/// how long each stage took, how the layout changed and what it left held.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameReport {
    pub number: u64,
    /// What led to this frame (clicks, keys, resizes, explicit actions), in order.
    pub causes: Vec<String>,
    pub reactive: ReactiveTrace,
    /// The component instances that rendered, in order (ADR-0013).
    pub rendered: Vec<crate::component::Rendered>,
    /// Bound closures run while laying out (ADR-0013's bindings by phase).
    pub bindings: u64,
    /// Pieces of text shaped (ADR-0019): none on a resize or a relayout,
    /// the changed one on a keystroke.
    pub shaped: u64,
    pub timings: Timings,
    pub diff: LayoutDiff,
    /// What layout did over the frame's passes (ADR-0014): calls, cache
    /// misses, measure calls, containers placed again (`placements`: none
    /// while something only scrolls, fades or moves) and parallel jobs.
    pub layout: LayoutStats,
    /// The nodes layout visited over the frame's passes (ADR-0013): resolved
    /// again, walked to what below needed it, frame nodes written.
    pub visits: crate::layout::Visits,
    /// What paint recorded (ADR-0016).
    pub paint: PaintStats,
    /// Where paint recorded again or only moved what it kept, in window
    /// coordinates: what the overlay flashes (ADR-0023).
    pub painted: Vec<Repaint>,
    /// Signals written in a phase after an earlier phase of the frame read
    /// them (ADR-0013's debug check; debug builds).
    pub late: Vec<crate::reactive::LateWrite>,
    /// What the renderer drew and uploaded.
    pub drawn: crate::scene::Drawn,
    pub footprint: Footprint,
}

/// What paint did in one frame (ADR-0016): how many chunks it painted, how
/// many it recorded again and why (Blink's `StyleDifference`: a box that
/// was laid out again, `NeedsLayout`; paint that changed, `NeedsPaint`), and
/// how many nodes only moved, faded or were clipped anew (`NeedsProperty`),
/// which records nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintStats {
    pub chunks: usize,
    /// Chunks recorded this frame, new ones included.
    pub recorded: usize,
    /// Of them, chunks whose box changed size.
    pub relaid: usize,
    /// Of them, chunks whose paint changed in a box of the same size.
    pub repainted: usize,
    /// Nodes whose offset, opacity, transform or clip alone changed.
    pub moved: usize,
    /// Frame nodes the record visited (ADR-0013: only what changed, with the
    /// subtrees its property state moved).
    pub visited: usize,
}

/// One region a frame's paint touched.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Repaint {
    pub bounds: Bounds,
    /// Recorded again (`NeedsLayout`, `NeedsPaint`, or new), else only its
    /// offset, opacity, transform or clip changed (`NeedsProperty`).
    pub recorded: bool,
}

impl fmt::Display for PaintStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "chunks {}  recorded {} (layout {}, paint {})  moved {}  visited {}",
            self.chunks, self.recorded, self.relaid, self.repainted, self.moved, self.visited
        )
    }
}

impl fmt::Display for crate::scene::Drawn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let signed = format!(
            "signed {}{}",
            self.signed,
            if self.planned_whole { " (whole)" } else { "" }
        );
        if !self.submitted {
            return write!(f, "{signed}  nothing submitted");
        }
        write!(
            f,
            "{signed}  encoded {}  uploaded {}  damage {} px in {} rects  layers {} (kept {})  folded {}",
            self.encoded,
            bytes(self.uploaded),
            self.damage,
            self.damage_rects,
            self.layers,
            self.layers_kept,
            self.folded
        )
    }
}

impl fmt::Display for FrameReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let causes = if self.causes.is_empty() {
            "redraw".to_owned()
        } else {
            self.causes.join("; ")
        };
        writeln!(f, "frame {}: {causes}", self.number)?;
        writeln!(
            f,
            "  signals  wrote [{}]  read [{}]  effects run {}",
            join(&self.reactive.writes),
            join(&self.reactive.reads),
            self.reactive.effect_runs
        )?;
        writeln!(
            f,
            "  renders  {} [{}]  bindings {}",
            self.rendered.len(),
            rendered(&self.rendered),
            self.bindings
        )?;
        writeln!(
            f,
            "  time     build {}  layout {}  paint {}  vsync {}",
            ms(self.timings.build),
            ms(self.timings.layout),
            ms(self.timings.paint),
            ms(self.timings.vsync)
        )?;
        writeln!(
            f,
            "  layout   {}  {}  text shaped {}",
            self.diff,
            self.layout_counts(),
            self.shaped
        )?;
        writeln!(f, "  visits   {}", self.visits)?;
        writeln!(f, "  paint    {}  gpu {}", self.paint, self.drawn)?;
        for late in &self.late {
            writeln!(f, "  LATE     {late}")?;
        }
        write!(f, "  holds    {}", self.footprint)
    }
}

impl FrameReport {
    /// Layout's counters as one line: `calls 12 misses 3 measures 40 placed 2 jobs 0`.
    #[must_use]
    pub fn layout_counts(&self) -> String {
        let l = self.layout;
        format!(
            "calls {}  misses {}  measures {}  placed {}  jobs {}",
            l.calls, l.misses, l.measures, l.placements, l.jobs
        )
    }

    /// The area paint flashed: recorded again and moved, in square logical pixels.
    #[must_use]
    pub fn painted_area(&self) -> (f32, f32) {
        self.painted.iter().fold((0.0, 0.0), |(rec, mov), p| {
            let a = p.bounds.width.max(0.0) * p.bounds.height.max(0.0);
            if p.recorded { (rec + a, mov) } else { (rec, mov + a) }
        })
    }
}

/// Instances and why each rendered (`grid_button ← props ×3`): a run of
/// the same is counted, cut at 12 runs.
fn rendered(all: &[crate::component::Rendered]) -> String {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for r in all {
        let one = format!("{} ← {}", r.name, r.why);
        match runs.last_mut() {
            Some((last, k)) if *last == one => *k = k.saturating_add(1),
            _ => runs.push((one, 1)),
        }
    }
    let more = runs.len().saturating_sub(12);
    let mut out: Vec<String> = runs
        .iter()
        .take(12)
        .map(|(n, k)| if *k > 1 { format!("{n} ×{k}") } else { n.clone() })
        .collect();
    if more > 0 {
        out.push(format!("… {more} more"));
    }
    out.join(", ")
}

impl Footprint {
    /// What the caches hold, then the reactive graph and the heap: the two
    /// lines the HUD shows, which [`fmt::Display`] joins.
    #[must_use]
    pub fn lines(&self) -> [String; 2] {
        let g = self.graph;
        let heap = match (self.heap, self.thread) {
            (Some(h), Some(t)) => format!(
                "heap {} live, this frame {} allocations {} ({} on this thread, {})",
                bytes(h.live),
                h.allocations,
                bytes(h.allocated),
                t.allocations,
                bytes(t.allocated)
            ),
            (Some(h), None) => format!(
                "heap {} live, this frame {} allocations {}",
                bytes(h.live),
                h.allocations,
                bytes(h.allocated)
            ),
            (None, _) => "heap not counted".to_owned(),
        };
        [
            format!(
                "{} nodes  {} shaped  glyphs {}  icons {}  images {}  paths {}  layers {}  chunks {}",
                self.nodes, self.shaped, self.glyphs, self.icons, self.images, self.paths, self.layers, self.chunks
            ),
            format!(
                "signals {}  computed {}  effects {}  subscriptions {}  {heap}",
                g.signals, g.computeds, g.effects, g.subscriptions
            ),
        ]
    }
}

impl fmt::Display for Footprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [caches, graph] = self.lines();
        write!(f, "{caches}  {graph}")
    }
}

impl fmt::Display for Raster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({})",
            self.count,
            bytes(u64::try_from(self.bytes).unwrap_or(u64::MAX))
        )
    }
}

/// `n` bytes in the largest unit that keeps it above 1.
#[must_use]
pub fn bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = crate::convert::u64_to_f64(n);
    for unit in ["KB", "MB"] {
        v /= 1024.0;
        if v < 1024.0 {
            return format!("{v:.1} {unit}");
        }
    }
    format!("{:.1} GB", v / 1024.0)
}

fn join(items: &[impl fmt::Display]) -> String {
    items.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
}

/// `d` in milliseconds, two decimals.
#[must_use]
pub fn ms(d: Duration) -> String {
    format!("{:.2}ms", d.as_secs_f64() * 1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{Element, IntoElement, div, text};
    use crate::layout::{UiState, layout};
    use crate::styled::Styled;
    use crate::text::TextSystem;

    fn card(key: u64, label: &str) -> Element {
        div().key(key).height(40.0).child(text(label)).into_element()
    }

    fn frame(cards: Vec<Element>) -> Vec<FrameNode> {
        let mut ts = TextSystem::new();
        layout(
            &div().children(cards).into_element(),
            400.0,
            400.0,
            &UiState::default(),
            &mut crate::motion::Motion::default(),
            &mut ts,
        )
        .nodes
    }

    #[test]
    fn same_frame_has_no_changes() {
        let a = frame(vec![card(1, "one"), card(2, "two")]);
        assert!(diff(&a, &a).is_empty());
    }

    #[test]
    fn keyed_insert_reads_as_one_added_and_the_rest_moved() {
        let before = frame(vec![card(1, "one"), card(2, "two")]);
        let after = frame(vec![card(3, "new"), card(1, "one"), card(2, "two")]);
        let d = diff(&before, &after);

        let added = d.count(|c| matches!(c, Change::Added { .. }));
        assert_eq!(added, 2, "the new card and its text: {d}");
        assert_eq!(d.count(Change::repainted), 0, "keyed cards keep their identity: {d}");
        assert_eq!(
            d.count(Change::moved),
            4,
            "both old cards and their texts shift down: {d}"
        );
        assert!((d.largest_shift() - 40.0).abs() < EPSILON);
    }

    #[test]
    fn text_change_is_a_repaint_without_shift() {
        let before = frame(vec![card(1, "idle")]);
        let after = frame(vec![card(1, "busy")]);
        let d = diff(&before, &after);
        assert_eq!(d.count(Change::repainted), 1, "{d}");
        assert_eq!(d.count(Change::moved), 0);
    }

    #[test]
    fn removed_node_is_reported_at_its_old_place() {
        let before = frame(vec![card(1, "one"), card(2, "two")]);
        let after = frame(vec![card(2, "two")]);
        let d = diff(&before, &after);
        assert_eq!(d.count(|c| matches!(c, Change::Removed { .. })), 2, "{d}");
    }
}
