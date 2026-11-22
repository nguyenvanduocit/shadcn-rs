//! Block layout on the retained tree (ADR-0014, `layout::engine::block`,
//! Taffy 0.15.1's block algorithm ported) against Taffy's own, laid out anew
//! ([`Oracle`]: `TaffyTree` without its float-blind cache hit):
//! random block flow (margins that collapse through and between boxes,
//! negative and `auto` ones, padding and borders that stop them, floats
//! left and right with `clear`, boxes that make a new formatting context,
//! `align-content`, `justify-self`, legacy `text-align`, right to left,
//! minimum and maximum sizes, box sizing, sizing keywords, relative and
//! absolute boxes), with text leaves and nested blocks, flex rows and grids
//! among them, under definite, min- and max-content space; then random
//! changes to their text and their style, each laid out and compared box
//! by box, bit for bit (place, size, scrollable overflow), serially, in
//! parallel and in reverse. An aspect ratio is not generated: gui resolves
//! it as Chrome does, not as Taffy (`engine/sizing.rs`), and is compared
//! with Chrome in `chrome_layout`.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::indexing_slicing,
    reason = "a test harness: small seeded numbers, and a tree indexed by the ids it hands out"
)]

use gui::layout::engine::{Change, Measure, MeasureReads, Measured, Node, Schedule};
use taffy::prelude::*;
use taffy::{
    BlockContext, Cache, CacheTree, Clear, DetailedGridInfo, DetailedLayoutInfo, Float, Layout, LayoutBlockContainer,
    LayoutContainingBlock, LayoutFlexboxContainer, LayoutGridContainer, LayoutInput, LayoutOutput, LayoutPartialTree,
    Overflow, Point, RunMode, TextAlign, TraversePartialTree, TraverseTree, compute_block_layout,
    compute_cached_layout, compute_flexbox_layout, compute_grid_layout, compute_hidden_layout, compute_oof_layout,
    compute_root_layout,
};

type Style = taffy::Style<String>;

/// Text 7 px a character, 20 px a line, wrapped at spaces.
#[derive(Clone, Debug, PartialEq)]
struct Text(String);

impl Text {
    fn size(&self, known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Size<f32> {
        let words: Vec<usize> = self.0.split(' ').map(str::len).collect();
        let longest = words.iter().copied().max().unwrap_or(0) as f32 * 7.0;
        let all = self.0.len() as f32 * 7.0;
        let width = known.width.unwrap_or(match available.width {
            AvailableSpace::MinContent => longest,
            AvailableSpace::MaxContent => all,
            AvailableSpace::Definite(w) => w.min(all).max(longest),
        });
        let mut lines = 1_u32;
        let mut line = 0.0_f32;
        for w in words {
            let w = w as f32 * 7.0;
            if line > 0.0 && line + 7.0 + w > width + 0.01 {
                lines += 1;
                line = w;
            } else {
                line += if line > 0.0 { 7.0 + w } else { w };
            }
        }
        Size {
            width,
            height: known.height.unwrap_or(lines as f32 * 20.0),
        }
    }
}

impl Measure for Text {
    type Cx = ();
    fn reads(&self) -> MeasureReads {
        MeasureReads::WIDTH_ONLY
    }
    fn measure(&self, (): &mut (), known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        self.size(known, available).into()
    }
}

type Ours = Node<Style, Text>;

#[derive(Clone, Debug)]
struct Desc {
    style: Style,
    text: Option<Text>,
    children: Vec<Desc>,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len())]
    }
}

fn words(r: &mut Rng) -> Text {
    let n = 1 + r.below(6);
    let w: Vec<String> = (0..n).map(|_| "x".repeat(1 + r.below(9))).collect();
    Text(w.join(" "))
}

fn px(v: f32) -> LengthPercentage {
    LengthPercentage::length(v)
}

fn rect(v: f32) -> Rect<LengthPercentage> {
    Rect {
        left: px(v),
        right: px(v),
        top: px(v),
        bottom: px(v),
    }
}

fn margin(r: &mut Rng) -> LengthPercentageAuto {
    r.pick(&[
        LengthPercentageAuto::length(0.0),
        LengthPercentageAuto::length(0.0),
        LengthPercentageAuto::length(8.0),
        LengthPercentageAuto::length(12.0),
        LengthPercentageAuto::length(-5.0),
        LengthPercentageAuto::percent(0.05),
        LengthPercentageAuto::auto(),
    ])
}

/// A box's own style in block flow (a text, or a container).
fn box_style(r: &mut Rng) -> Style {
    let mut s = Style {
        display: Display::Block,
        ..Style::default()
    };
    s.margin = Rect {
        left: margin(r),
        right: margin(r),
        top: margin(r),
        bottom: margin(r),
    };
    if r.below(3) == 0 {
        s.margin.left = LengthPercentageAuto::length(0.0);
        s.margin.right = LengthPercentageAuto::length(0.0);
    }
    s.padding = rect(r.pick(&[0.0, 0.0, 0.0, 4.0, 2.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 0.0, 1.0]));
    if r.below(5) == 0 {
        s.padding.top = px(0.0);
        s.border.bottom = px(2.0);
    }
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(120.0),
        Dimension::percent(0.5),
        Dimension::fit_content(),
        Dimension::min_content(),
        Dimension::max_content(),
    ]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(0.0),
        Dimension::length(30.0),
        Dimension::percent(0.2),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(60.0),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(180.0),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(25.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(50.0),
    ]);
    s.box_sizing = r.pick(&[BoxSizing::BorderBox, BoxSizing::BorderBox, BoxSizing::ContentBox]);
    s.float = r.pick(&[
        Float::None,
        Float::None,
        Float::None,
        Float::None,
        Float::Left,
        Float::Right,
    ]);
    s.clear = r.pick(&[
        Clear::None,
        Clear::None,
        Clear::None,
        Clear::Left,
        Clear::Right,
        Clear::Both,
    ]);
    let o = r.pick(&[
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Hidden,
        Overflow::Scroll,
    ]);
    s.overflow = Point { x: o, y: o };
    s.scrollbar_width = 6.0;
    s.justify_self = r.pick(&[
        None,
        None,
        None,
        Some(AlignItems::START),
        Some(AlignItems::END),
        Some(AlignItems::CENTER),
        Some(AlignItems::SAFE_CENTER),
        Some(AlignItems::STRETCH),
    ]);
    match r.below(16) {
        0 => {
            s.position = Position::Relative;
            s.inset.top = LengthPercentageAuto::length(5.0);
            s.inset.left = LengthPercentageAuto::percent(0.1);
        }
        1 => {
            s.position = Position::Absolute;
            s.inset.right = LengthPercentageAuto::length(3.0);
        }
        2 => s.direction = taffy::Direction::Rtl,
        3 => s.display = Display::FlowRoot,
        4 => s.display = Display::None,
        _ => {}
    }
    s
}

/// A container's style: a block, or now and then something else in flow.
fn container_style(r: &mut Rng) -> Style {
    let mut s = box_style(r);
    s.align_content = r.pick(&[
        AlignContent::NORMAL,
        AlignContent::NORMAL,
        AlignContent::NORMAL,
        AlignContent::CENTER,
        AlignContent::END,
        AlignContent::SAFE_END,
    ]);
    s.justify_items = r.pick(&[
        AlignItems::NORMAL,
        AlignItems::NORMAL,
        AlignItems::CENTER,
        AlignItems::END,
    ]);
    s.text_align = r.pick(&[
        TextAlign::Auto,
        TextAlign::Auto,
        TextAlign::LegacyCenter,
        TextAlign::LegacyRight,
        TextAlign::LegacyLeft,
    ]);
    s.contain = r.pick(&[
        taffy::Contain::NONE,
        taffy::Contain::NONE,
        taffy::Contain::NONE,
        taffy::Contain::LAYOUT,
    ]);
    match r.below(12) {
        0 => {
            s.display = Display::Flex;
            s.flex_wrap = r.pick(&[FlexWrap::NoWrap, FlexWrap::Wrap]);
        }
        1 => {
            s.display = Display::Grid;
            s.grid_template_columns = vec![taffy::GridTemplateComponent::Single(taffy::TrackSizingFunction {
                min: taffy::MinTrackSizingFunction::auto(),
                max: taffy::MaxTrackSizingFunction::fr(1.0),
            })];
        }
        _ => {}
    }
    s
}

fn text(r: &mut Rng) -> Desc {
    Desc {
        style: box_style(r),
        text: Some(words(r)),
        children: Vec::new(),
    }
}

fn container(r: &mut Rng, depth: u32) -> Desc {
    let n = match r.below(5) {
        0 => 0,
        1 => 1,
        _ => 1 + r.below(6),
    };
    let children = (0..n)
        .map(|_| {
            if depth > 0 && r.below(3) == 0 {
                container(r, depth - 1)
            } else {
                text(r)
            }
        })
        .collect();
    Desc {
        style: container_style(r),
        text: None,
        children,
    }
}

fn page(r: &mut Rng) -> Desc {
    let n = 1 + r.below(8);
    let children = (0..n).map(|_| container(r, 3)).collect();
    let mut style = container_style(r);
    style.position = Position::Relative;
    style.float = Float::None;
    style.display = Display::Block;
    Desc {
        style,
        text: None,
        children,
    }
}

fn build_ours(d: &Desc) -> Ours {
    let mut n = Node::new(d.style.clone(), d.text.clone());
    n.children = d.children.iter().map(build_ours).collect();
    n
}

/// Taffy's algorithms on a tree of their own: `TaffyTree` as it is, except
/// that its cache answers no final layout, and nothing for a block laid out
/// in its parent's formatting context. Its cache is keyed on a box's inputs
/// alone, which gets two things wrong:
///
/// - A block in its parent's formatting context also reads the floats
///   placed before it and places its own (and its descendants') for the
///   boxes after it: answered from the cache, it places none, and a `clear`
///   after it clears only the floats beside it, not one inside it (CSS 2
///   §9.5.2: clearance is past every earlier float of the formatting
///   context; Chrome does so).
/// - A final layout answered from the cache leaves the subtree as the last
///   run placed it, and a size probe that lays a descendant out for a final
///   layout of its own (a block placing its floats, flexbox and grid
///   finding baselines) places it for the probe: the cached answer's size
///   comes back over children placed for another one.
///
/// gui's engine lays such a block out each time, and a final layout a probe
/// placed over answers no final layout (`NodeCache::unplace`).
struct Oracle {
    nodes: Vec<OracleNode>,
}

struct OracleNode {
    style: Style,
    text: Option<Text>,
    children: Vec<NodeId>,
    cache: Cache,
    layout: Layout,
    hoisted: Vec<NodeId>,
    info: DetailedLayoutInfo<String>,
}

impl Oracle {
    fn build(d: &Desc) -> (Self, NodeId) {
        let mut tree = Self { nodes: Vec::new() };
        let root = tree.add(d);
        (tree, root)
    }

    fn add(&mut self, d: &Desc) -> NodeId {
        let children = d.children.iter().map(|c| self.add(c)).collect();
        self.nodes.push(OracleNode {
            style: d.style.clone(),
            text: d.text.clone(),
            children,
            cache: Cache::new(),
            layout: Layout::new(),
            hoisted: Vec::new(),
            info: DetailedLayoutInfo::None,
        });
        NodeId::from(self.nodes.len() - 1)
    }

    fn node(&self, id: NodeId) -> &OracleNode {
        &self.nodes[usize::from(id)]
    }

    fn node_mut(&mut self, id: NodeId) -> &mut OracleNode {
        &mut self.nodes[usize::from(id)]
    }

    fn layout_child(&mut self, id: NodeId, inputs: LayoutInput, block: Option<&mut BlockContext<'_>>) -> LayoutOutput {
        if inputs.run_mode == RunMode::PerformHiddenLayout {
            self.node_mut(id).layout = Layout::with_order(0);
            return compute_hidden_layout(self, id);
        }
        if block.is_some() || inputs.run_mode == RunMode::PerformLayout {
            self.node_mut(id).cache.clear();
        }
        compute_cached_layout(self, id, inputs, |tree, id, inputs| {
            let node = tree.node(id);
            let mut output = match (node.style.display, node.children.is_empty()) {
                (Display::None, _) => compute_hidden_layout(tree, id),
                (Display::Block, false) => compute_block_layout(tree, id, inputs, block),
                (Display::FlowRoot, false) => compute_block_layout(tree, id, inputs, None),
                (Display::Flex, false) => compute_flexbox_layout(tree, id, inputs),
                (Display::Grid, false) => compute_grid_layout(tree, id, inputs),
                (_, true) => {
                    let node = tree.node_mut(id);
                    measure(inputs, id, node.text.as_mut(), &node.style)
                }
            };
            if inputs.run_mode == RunMode::PerformLayout {
                compute_oof_layout(tree, id, &mut output);
            }
            output
        })
    }
}

impl TraversePartialTree for Oracle {
    type ChildIter<'a> = std::iter::Copied<std::slice::Iter<'a, NodeId>>;
    fn child_ids(&self, id: NodeId) -> Self::ChildIter<'_> {
        self.node(id).children.iter().copied()
    }
    fn child_count(&self, id: NodeId) -> usize {
        self.node(id).children.len()
    }
    fn get_child_id(&self, id: NodeId, index: usize) -> NodeId {
        self.node(id).children[index]
    }
}

impl TraverseTree for Oracle {}

impl LayoutPartialTree for Oracle {
    type CoreContainerStyle<'a> = &'a Style;
    type CustomIdent = String;
    fn get_core_container_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn set_unrounded_layout(&mut self, id: NodeId, layout: &Layout) {
        self.node_mut(id).layout = *layout;
    }
    fn compute_child_layout(&mut self, id: NodeId, inputs: LayoutInput) -> LayoutOutput {
        self.layout_child(id, inputs, None)
    }
}

impl LayoutContainingBlock for Oracle {
    type OofItemStyle<'a> = &'a Style;
    fn get_oof_item_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn clear_hoisted_children(&mut self, id: NodeId) {
        self.node_mut(id).hoisted.clear();
    }
    fn add_hoisted_children(&mut self, id: NodeId, hoisted: &[NodeId]) {
        self.node_mut(id).hoisted.extend_from_slice(hoisted);
    }
    fn get_detailed_layout_info(&self, id: NodeId) -> &DetailedLayoutInfo<String> {
        &self.node(id).info
    }
}

impl CacheTree for Oracle {
    fn cache_get(&mut self, id: NodeId, input: &LayoutInput) -> Option<LayoutOutput> {
        self.node_mut(id).cache.get(input)
    }
    fn cache_store(&mut self, id: NodeId, input: &LayoutInput, output: LayoutOutput) {
        self.node_mut(id).cache.store(input, output);
    }
    fn cache_clear(&mut self, id: NodeId) {
        self.node_mut(id).cache.clear();
    }
}

impl LayoutBlockContainer for Oracle {
    type BlockContainerStyle<'a> = &'a Style;
    type BlockItemStyle<'a> = &'a Style;
    fn get_block_container_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn get_block_child_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn compute_block_child_layout(
        &mut self,
        id: NodeId,
        inputs: LayoutInput,
        block: Option<&mut BlockContext<'_>>,
    ) -> LayoutOutput {
        self.layout_child(id, inputs, block)
    }
}

impl LayoutFlexboxContainer for Oracle {
    type FlexboxContainerStyle<'a> = &'a Style;
    type FlexboxItemStyle<'a> = &'a Style;
    fn get_flexbox_container_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn get_flexbox_child_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
}

impl LayoutGridContainer for Oracle {
    type GridContainerStyle<'a> = &'a Style;
    type GridItemStyle<'a> = &'a Style;
    fn get_grid_container_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn get_grid_child_style(&self, id: NodeId) -> &Style {
        &self.node(id).style
    }
    fn set_detailed_grid_info(&mut self, id: NodeId, info: DetailedGridInfo<String>) {
        self.node_mut(id).info = DetailedLayoutInfo::Grid(Box::new(info));
    }
}

fn measure(input: LayoutInput, _: NodeId, text: Option<&mut Text>, style: &Style) -> LayoutOutput {
    let text = text.map(|t| t.clone());
    gui::layout::engine::compute_leaf(
        input,
        style,
        |_, _| 0.0,
        false,
        |known, available| text.as_ref().map_or(Size::ZERO, |t| t.size(known, available)).into(),
    )
}

fn same(a: &Layout, b: &Layout) -> bool {
    a.location == b.location && a.size == b.size && a.scrollable_overflow_rect == b.scrollable_overflow_rect
}

fn compare(ours: &Ours, tree: &Oracle, id: NodeId, at: &str) -> Option<String> {
    let theirs = &tree.node(id).layout;
    if !same(ours.layout(), theirs) {
        return Some(format!("{at}: ours {:?}, Taffy's {:?}", ours.layout(), theirs));
    }
    ours.children
        .iter()
        .zip(&tree.node(id).children)
        .enumerate()
        .find_map(|(i, (c, &k))| compare(c, tree, k, &format!("{at}/{i}")))
}

fn paths(d: &Desc, at: &mut Vec<usize>, leaves: &mut Vec<Vec<usize>>, boxes: &mut Vec<Vec<usize>>) {
    if d.children.is_empty() {
        leaves.push(at.clone());
    } else {
        boxes.push(at.clone());
    }
    for (i, c) in d.children.iter().enumerate() {
        at.push(i);
        paths(c, at, leaves, boxes);
        at.pop();
    }
}

fn desc_at<'a>(d: &'a mut Desc, path: &[usize]) -> &'a mut Desc {
    match path.split_first() {
        None => d,
        Some((&i, rest)) => desc_at(&mut d.children[i], rest),
    }
}

fn change(n: &mut Ours, path: &[usize], d: &Desc, style: bool) -> Change {
    match path.split_first() {
        None => {
            n.style = d.style.clone();
            n.leaf = d.text.clone();
            n.settle(style, !style, Change::None)
        }
        Some((&i, rest)) => {
            let reach = change(&mut n.children[i], rest, &d.children[i], style);
            n.settle(false, false, reach)
        }
    }
}

fn space(r: &mut Rng) -> Size<AvailableSpace> {
    Size {
        width: r.pick(&[
            AvailableSpace::Definite(300.0),
            AvailableSpace::Definite(520.0),
            AvailableSpace::MaxContent,
            AvailableSpace::MinContent,
        ]),
        height: r.pick(&[
            AvailableSpace::Definite(600.0),
            AvailableSpace::MaxContent,
            AvailableSpace::MinContent,
        ]),
    }
}

fn run(seed: u64, schedule: Schedule, steps: usize) -> Option<String> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut desc = page(&mut r);
    let mut space = space(&mut r);
    let mut ours = build_ours(&desc);
    ours.recount();
    let check = |ours: &Ours, desc: &Desc, space: Size<AvailableSpace>, what: &str| {
        let (mut tree, id) = Oracle::build(desc);
        compute_root_layout(&mut tree, id, space);
        compare(ours, &tree, id, "").map(|d| format!("seed {seed} {schedule:?} {what}{d}"))
    };
    ours.compute_root(space, &mut (), schedule);
    if let Some(d) = check(&ours, &desc, space, "cold") {
        return Some(d);
    }
    let (mut leaves, mut boxes) = (Vec::new(), Vec::new());
    paths(&desc, &mut Vec::new(), &mut leaves, &mut boxes);
    for step in 0..steps {
        let what = match r.below(8) {
            0 => {
                space = self::space(&mut r);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} (space {space:?})")
            }
            1 | 2 if boxes.len() > 1 => {
                let path = boxes[1 + r.below(boxes.len() - 1)].clone();
                desc_at(&mut desc, &path).style = container_style(&mut r);
                change(&mut ours, &path, &desc, true);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} (style at {path:?})")
            }
            k if !leaves.is_empty() => {
                let path = leaves[r.below(leaves.len())].clone();
                let style = k == 3;
                {
                    let d = desc_at(&mut desc, &path);
                    if style {
                        d.style = box_style(&mut r);
                    } else if d.text.is_some() {
                        d.text = Some(words(&mut r));
                    }
                }
                change(&mut ours, &path, &desc, style);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} ({} at {path:?})", if style { "style" } else { "text" })
            }
            _ => continue,
        };
        if let Some(d) = check(&ours, &desc, space, &what) {
            return Some(d);
        }
    }
    None
}

const SCHEDULES: [Schedule; 3] = [
    Schedule {
        parallel: false,
        reverse: false,
    },
    Schedule {
        parallel: true,
        reverse: false,
    },
    Schedule {
        parallel: false,
        reverse: true,
    },
];

#[test]
fn block_flow_lays_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    let seeds = if cfg!(debug_assertions) { 120 } else { 500 };
    let failed: Vec<String> = (0..seeds)
        .flat_map(|seed| SCHEDULES.iter().filter_map(move |&s| run(seed, s, 16)))
        .collect();
    assert!(
        failed.is_empty(),
        "{} runs differ:\n{}",
        failed.len(),
        failed.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}
