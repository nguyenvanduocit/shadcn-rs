//! Debug views of a laid-out frame: a text dump of the tree, the difference
//! between two frames, and the report of one frame.
//!
//! Pure functions over [`FrameNode`]s; painting them is the overlay's
//! job ([`crate::overlay::paint_overlay`]).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::time::Duration;

pub use crate::geometry::Bounds;
use crate::geometry::NodePath;
use crate::heap::Heap;
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
    pub changes: Vec<NodeChange>,
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
        for c in &self.changes {
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
    let old: HashMap<&NodePath, &FrameNode> = before.iter().map(|r| (&r.path, r)).collect();
    let mut seen: HashSet<&NodePath> = HashSet::with_capacity(after.len());
    let mut changes = Vec::new();

    for now in after {
        seen.insert(&now.path);
        let change = match old.get(&now.path) {
            None => Some(Change::Added { after: now.bounds }),
            Some(was) => {
                let repainted = was.look != now.look || was.content != now.content;
                (repainted || moved(&now.bounds, &was.bounds) || resized(&now.bounds, &was.bounds)).then_some(
                    Change::Updated {
                        before: was.bounds,
                        after: now.bounds,
                        repainted,
                    },
                )
            }
        };
        if let Some(change) = change {
            changes.push(NodeChange {
                path: now.path.clone(),
                what: describe(now),
                change,
            });
        }
    }
    for was in before.iter().filter(|r| !seen.contains(&r.path)) {
        changes.push(NodeChange {
            path: was.path.clone(),
            what: describe(was),
            change: Change::Removed { before: was.bounds },
        });
    }

    LayoutDiff {
        changes,
        nodes: after.len(),
    }
}

/// A short name for a node: its debug name, or what it shows
/// (`Div #ffffff`, `Text "Agents"`, `Icon chevron-down`, `Input "abc"`).
#[must_use]
pub fn describe(node: &FrameNode) -> String {
    let what = match &node.content {
        Content::Text { content, .. } => format!("Text {:?}", clip(content)),
        Content::Icon { icon, .. } => format!("Icon {}", icon.name()),
        Content::Input { edit, .. } => format!("Input {:?}", clip(&edit.text)),
        Content::Shapes(shapes) => format!("Shapes x{}", shapes.len()),
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
        Content::Icon { .. } | Content::Shapes(_) | Content::Image { .. } | Content::None => false,
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

impl Raster {
    pub(crate) fn of<'a>(pixmaps: impl IntoIterator<Item = &'a tiny_skia::Pixmap>) -> Self {
        pixmaps.into_iter().fold(Self::default(), |r, p| Self {
            count: r.count.saturating_add(1),
            bytes: r.bytes.saturating_add(p.data().len()),
        })
    }
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
    pub shadows: Raster,
    /// Images scaled to the size they are drawn at.
    pub images: Raster,
    /// Window-sized layers groups paint aside into.
    pub layers: Raster,
    pub graph: Graph,
    /// What this frame allocated (`allocations`, `allocated`) and what the
    /// heap holds after it (`live`).
    pub heap: Option<Heap>,
}

impl Footprint {
    /// The bytes of every image the caches hold.
    #[must_use]
    pub fn raster_bytes(&self) -> usize {
        [self.glyphs, self.icons, self.shadows, self.images, self.layers]
            .iter()
            .fold(0, |n, r| n.saturating_add(r.bytes))
    }
}

/// Everything one frame did: why it ran, what the reactive runtime did,
/// how long each stage took, how the layout changed and what it left held.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameReport {
    pub number: u64,
    /// What led to this frame (clicks, keys, resizes, explicit actions), in order.
    pub causes: Vec<String>,
    pub reactive: ReactiveTrace,
    pub timings: Timings,
    pub diff: LayoutDiff,
    pub footprint: Footprint,
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
            "  time     build {}  layout {}  paint {}  vsync {}",
            ms(self.timings.build),
            ms(self.timings.layout),
            ms(self.timings.paint),
            ms(self.timings.vsync)
        )?;
        writeln!(f, "  layout   {}", self.diff)?;
        write!(f, "  holds    {}", self.footprint)
    }
}

impl Footprint {
    /// What the caches hold, then the reactive graph and the heap: the two
    /// lines the HUD shows, which [`fmt::Display`] joins.
    #[must_use]
    pub fn lines(&self) -> [String; 2] {
        let g = self.graph;
        let heap = match self.heap {
            Some(h) => format!(
                "heap {} live, this frame {} allocations {}",
                bytes(h.live),
                h.allocations,
                bytes(h.allocated)
            ),
            None => "heap not counted".to_owned(),
        };
        [
            format!(
                "{} nodes  {} shaped  glyphs {}  icons {}  shadows {}  images {}  layers {}",
                self.nodes, self.shaped, self.glyphs, self.icons, self.shadows, self.images, self.layers
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
        div().key(key).h_px(40.0).child(text(label)).into_element()
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
        .expect("layout")
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
