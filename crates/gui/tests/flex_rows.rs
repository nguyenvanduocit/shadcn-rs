//! Flex rows laid out by the retained tree's own row pass (ADR-0014,
//! `layout::engine::row`) against `TaffyTree` built anew: random rows of
//! the case the pass takes (`row` and `row-reverse`, left to right and right
//! to left, a known or unknown width, growing and shrinking items, `auto`
//! margins, every alignment, gaps, minimum and maximum sizes, box sizing,
//! scrollbars, relative offsets) and some just outside it (wrapping, a
//! sizing keyword, a column), with text leaves, nested rows
//! and columns, inside flex columns and rows, blocks and grids, under
//! definite, min- and max-content space; then random changes to their text
//! and their style, each laid out and compared box by box, bit for bit
//! (place, size, scrollable overflow), serially, in parallel and in reverse.
//! Rows with absolute items or baseline alignment are compared after every
//! change too. An aspect ratio is not generated: gui resolves it as Chrome
//! does, not as Taffy (`engine/sizing.rs`), and is compared with Chrome in
//! `chrome_layout`.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a test harness: small seeded numbers, and a tree that cannot be built is a failed test"
)]

use gui::layout::engine::{Change, Measure, MeasureReads, Measured, Node, Schedule};
use taffy::prelude::*;
use taffy::{
    GridTemplateComponent, Layout, LayoutInput, LayoutOutput, MaxTrackSizingFunction, MinTrackSizingFunction, Overflow,
    Point, TaffyTree, TrackSizingFunction,
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
        // A known width and a definite available one alike, no height read
        // (`MeasureReads::TEXT`, as `gui`'s text).
        let width = match known.width.map_or(available.width, AvailableSpace::Definite) {
            AvailableSpace::MinContent => longest,
            AvailableSpace::MaxContent => all,
            AvailableSpace::Definite(w) => w.min(all).max(longest),
        };
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
            height: lines as f32 * 20.0,
        }
    }
}

impl Measure for Text {
    type Cx = ();
    fn reads(&self) -> MeasureReads {
        MeasureReads::TEXT
    }
    fn measure(&self, (): &mut (), known: Size<Option<f32>>, available: Size<AvailableSpace>) -> Measured {
        self.size(known, available).into()
    }
}

type Ours = Node<Style, Text>;

/// A tree both sides are built from.
#[derive(Clone, Debug)]
struct Desc {
    style: Style,
    text: Option<Text>,
    children: Vec<Desc>,
}

thread_local! {
    /// What rows generate besides the pass's case: 1 absolute items, 2
    /// baseline alignment as well.
    static WILD: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

fn wild() -> bool {
    WILD.with(std::cell::Cell::get) >= 1
}

fn baseline() -> bool {
    WILD.with(std::cell::Cell::get) >= 2
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

fn lpa(r: &mut Rng, of: &[f32]) -> LengthPercentageAuto {
    LengthPercentageAuto::length(r.pick(of))
}

fn align(r: &mut Rng) -> Option<AlignItems> {
    r.pick(&[
        None,
        None,
        None,
        Some(AlignItems::START),
        Some(AlignItems::END),
        Some(AlignItems::CENTER),
        Some(AlignItems::CENTER),
        Some(AlignItems::STRETCH),
        Some(AlignItems::FLEX_START),
        Some(AlignItems::FLEX_END),
        Some(AlignItems::SAFE_CENTER),
        Some(AlignItems::SELF_END),
        Some(AlignItems::SELF_START),
    ])
}

/// An item's own style in its row.
fn item_style(r: &mut Rng) -> Style {
    let mut s = Style::default();
    let m = r.pick(&[0.0, 0.0, 3.0, -2.0, 1.5]);
    s.margin = Rect {
        left: lpa(r, &[0.0, 0.0, 4.0]),
        right: LengthPercentageAuto::length(r.pick(&[0.0, m])),
        top: LengthPercentageAuto::length(m),
        bottom: lpa(r, &[0.0, 2.0]),
    };
    match r.below(16) {
        0 => s.margin.left = LengthPercentageAuto::auto(),
        1 => s.margin.right = LengthPercentageAuto::auto(),
        2 => s.margin.top = LengthPercentageAuto::auto(),
        3 => {
            s.margin.top = LengthPercentageAuto::auto();
            s.margin.bottom = LengthPercentageAuto::auto();
        }
        4 => s.margin.left = LengthPercentageAuto::percent(0.05),
        _ => {}
    }
    s.padding = rect(r.pick(&[0.0, 0.0, 4.0, 2.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    if r.below(5) == 0 {
        s.padding.left = px(1.0);
        s.border.bottom = px(2.0);
    }
    s.align_self = align(r);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(50.0),
        Dimension::length(0.0),
        Dimension::percent(0.3),
    ]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(25.0),
        Dimension::percent(0.5),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(40.0),
        LengthPercentageAuto::length(0.0),
        LengthPercentageAuto::percent(0.1),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(70.0),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
        LengthPercentageAuto::percent(0.8),
    ]);
    s.flex_grow = r.pick(&[0.0, 0.0, 0.0, 1.0, 2.0, 0.5]);
    s.flex_shrink = r.pick(&[1.0, 1.0, 0.0, 2.0, 0.3]);
    s.flex_basis = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(0.0),
        Dimension::length(60.0),
        Dimension::percent(0.25),
        Dimension::content(),
    ]);
    let o = r.pick(&[
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Hidden,
        Overflow::Scroll,
    ]);
    s.overflow = Point { x: o, y: o };
    s.scrollbar_width = 6.0;
    s.box_sizing = r.pick(&[BoxSizing::BorderBox, BoxSizing::BorderBox, BoxSizing::ContentBox]);
    if r.below(8) == 0 {
        s.position = Position::Relative;
        s.inset.top = LengthPercentageAuto::length(5.0);
        s.inset.left = LengthPercentageAuto::percent(0.1);
        if r.below(2) == 0 {
            s.inset.right = LengthPercentageAuto::length(3.0);
        }
    }
    if r.below(10) == 0 {
        s.direction = taffy::Direction::Rtl;
    }
    // Now and then an item the pass does not take, or hides.
    match r.below(40) {
        0 => s.size.height = Dimension::max_content(),
        1 => s.size.width = Dimension::min_content(),
        2 if wild() => s.position = Position::Absolute,
        3 if baseline() => s.align_self = Some(AlignItems::BASELINE),
        4 | 5 => s.display = Display::None,
        _ => {}
    }
    s
}

/// A flex row's own style: mostly what the pass takes, sometimes not.
fn row_style(r: &mut Rng) -> Style {
    let mut s = Style {
        display: Display::Flex,
        flex_direction: r.pick(&[
            FlexDirection::Row,
            FlexDirection::Row,
            FlexDirection::Row,
            FlexDirection::RowReverse,
        ]),
        ..Style::default()
    };
    if r.below(4) == 0 {
        s.direction = taffy::Direction::Rtl;
    }
    s.gap = Size {
        width: r.pick(&[px(0.0), px(0.0), px(8.0), px(2.5), LengthPercentage::percent(0.02)]),
        height: px(r.pick(&[0.0, 4.0])),
    };
    s.padding = rect(r.pick(&[0.0, 0.0, 6.0, 1.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    if r.below(4) == 0 {
        s.border.left = px(2.0);
        s.padding.right = px(3.0);
    }
    s.align_items = r.pick(&[
        AlignItems::NORMAL,
        AlignItems::CENTER,
        AlignItems::CENTER,
        AlignItems::START,
        AlignItems::END,
        AlignItems::FLEX_END,
        AlignItems::STRETCH,
        AlignItems::SAFE_CENTER,
    ]);
    s.justify_content = r.pick(&[
        JustifyContent::NORMAL,
        JustifyContent::NORMAL,
        JustifyContent::CENTER,
        JustifyContent::END,
        JustifyContent::FLEX_END,
        JustifyContent::SPACE_BETWEEN,
        JustifyContent::SPACE_EVENLY,
        JustifyContent::SPACE_AROUND,
        JustifyContent::START,
        JustifyContent::SAFE_CENTER,
    ]);
    s.align_content = r.pick(&[
        AlignContent::NORMAL,
        AlignContent::NORMAL,
        AlignContent::CENTER,
        AlignContent::END,
        AlignContent::SPACE_BETWEEN,
        AlignContent::STRETCH,
        AlignContent::SAFE_END,
    ]);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(260.0),
        Dimension::percent(0.6),
    ]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(48.0),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(120.0),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(200.0),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(70.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(40.0),
    ]);
    s.margin.top = LengthPercentageAuto::length(r.pick(&[0.0, 0.0, 2.0]));
    let o = r.pick(&[
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Visible,
        Overflow::Hidden,
        Overflow::Scroll,
    ]);
    s.overflow = Point { x: o, y: o };
    s.scrollbar_width = 8.0;
    s.box_sizing = r.pick(&[BoxSizing::BorderBox, BoxSizing::BorderBox, BoxSizing::ContentBox]);
    s.contain = r.pick(&[
        taffy::Contain::NONE,
        taffy::Contain::NONE,
        taffy::Contain::NONE,
        taffy::Contain::LAYOUT,
    ]);
    match r.below(30) {
        0 => s.flex_wrap = FlexWrap::Wrap,
        1 => s.flex_direction = FlexDirection::Column,
        2 => s.flex_wrap = FlexWrap::WrapReverse,
        3 if baseline() => s.align_items = AlignItems::BASELINE,
        4 => s.position = Position::Relative,
        _ => {}
    }
    s
}

/// A flex column of texts (an item of a row, or what holds rows).
fn column_style(r: &mut Rng) -> Style {
    let mut s = item_style(r);
    s.display = Display::Flex;
    s.flex_direction = FlexDirection::Column;
    s.align_items = r.pick(&[AlignItems::NORMAL, AlignItems::START, AlignItems::CENTER]);
    s.gap.height = px(r.pick(&[0.0, 2.0]));
    s
}

/// A new style for a container of the same kind as `old`.
fn container_style(r: &mut Rng, old: &Style) -> Style {
    let mut s = match (old.display, old.flex_direction) {
        (Display::Flex, FlexDirection::Row | FlexDirection::RowReverse) => row_style(r),
        (Display::Flex, _) => column_style(r),
        _ => {
            let mut s = old.clone();
            s.padding = rect(r.pick(&[0.0, 3.0]));
            s
        }
    };
    if r.below(2) == 0 {
        let item = item_style(r);
        s.margin = item.margin;
        s.align_self = item.align_self;
        s.flex_grow = item.flex_grow;
        s.flex_shrink = item.flex_shrink;
        s.flex_basis = item.flex_basis;
    }
    s
}

fn text(r: &mut Rng) -> Desc {
    Desc {
        style: item_style(r),
        text: Some(words(r)),
        children: Vec::new(),
    }
}

/// An item of a row: text, a nested row, or a column of texts.
fn item(r: &mut Rng, depth: u32) -> Desc {
    match r.below(7) {
        0 | 1 if depth > 0 => {
            let mut d = row(r, depth - 1);
            let s = item_style(r);
            d.style.margin = s.margin;
            d.style.align_self = s.align_self;
            d.style.flex_grow = s.flex_grow;
            d.style.flex_shrink = s.flex_shrink;
            d.style.flex_basis = s.flex_basis;
            d
        }
        2 if depth > 0 => {
            let n = 1 + r.below(3);
            Desc {
                style: column_style(r),
                text: None,
                children: (0..n).map(|_| text(r)).collect(),
            }
        }
        _ => text(r),
    }
}

/// A row: a badge (one text), a cell (centred text), or a toolbar of items.
fn row(r: &mut Rng, depth: u32) -> Desc {
    let n = match r.below(4) {
        0 | 1 => 1,
        _ => 1 + r.below(6),
    };
    let mut children: Vec<Desc> = (0..n).map(|_| item(r, depth)).collect();
    if r.below(10) == 0 && wild() {
        children.push(Desc {
            style: Style {
                position: Position::Absolute,
                size: Size {
                    width: Dimension::percent(0.5),
                    height: Dimension::length(10.0),
                },
                ..Style::default()
            },
            text: Some(words(r)),
            children: Vec::new(),
        });
    }
    Desc {
        style: row_style(r),
        text: None,
        children,
    }
}

/// A body of rows (a flex column, a flex row, a block or a grid) in a root.
fn table(r: &mut Rng) -> Desc {
    let n = 1 + r.below(16);
    let rows: Vec<Desc> = (0..n).map(|_| row(r, 2)).collect();
    let mut body = match r.below(5) {
        0 | 1 => Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: r.pick(&[AlignItems::NORMAL, AlignItems::START, AlignItems::CENTER]),
            ..Style::default()
        },
        2 => Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            align_items: r.pick(&[AlignItems::NORMAL, AlignItems::START, AlignItems::CENTER]),
            ..Style::default()
        },
        3 => Style {
            display: Display::Block,
            ..Style::default()
        },
        _ => Style {
            display: Display::Grid,
            grid_template_columns: vec![
                GridTemplateComponent::Single(TrackSizingFunction {
                    min: MinTrackSizingFunction::length(0.0),
                    max: MaxTrackSizingFunction::fr(1.0),
                }),
                GridTemplateComponent::Single(TrackSizingFunction {
                    min: MinTrackSizingFunction::auto(),
                    max: MaxTrackSizingFunction::auto(),
                }),
            ],
            ..Style::default()
        },
    };
    body.padding = rect(r.pick(&[0.0, 3.0]));
    body.gap = Size {
        width: px(r.pick(&[0.0, 2.0])),
        height: px(r.pick(&[0.0, 2.0])),
    };
    body.size.height = r.pick(&[Dimension::auto(), Dimension::auto(), Dimension::length(300.0)]);
    let o = r.pick(&[Overflow::Visible, Overflow::Visible, Overflow::Scroll]);
    body.overflow = Point { x: o, y: o };
    body.scrollbar_width = 10.0;
    let body = Desc {
        style: body,
        text: None,
        children: rows,
    };
    let root = Style {
        display: r.pick(&[Display::Flex, Display::Flex, Display::Block]),
        flex_direction: r.pick(&[FlexDirection::Column, FlexDirection::Row]),
        ..Style::default()
    };
    Desc {
        style: root,
        text: None,
        children: vec![body],
    }
}

fn build_ours(d: &Desc) -> Ours {
    let mut n = Node::new(d.style.clone(), d.text.clone());
    n.children = d.children.iter().map(build_ours).collect();
    n
}

fn build_taffy(d: &Desc, tree: &mut TaffyTree<Text>) -> NodeId {
    if d.children.is_empty() {
        let id = tree.new_leaf(d.style.clone()).expect("leaf");
        if let Some(t) = &d.text {
            tree.set_node_context(id, Some(t.clone())).expect("context");
        }
        return id;
    }
    let ids: Vec<NodeId> = d.children.iter().map(|c| build_taffy(c, tree)).collect();
    tree.new_with_children(d.style.clone(), &ids).expect("node")
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

fn compare(ours: &Ours, tree: &TaffyTree<Text>, id: NodeId, at: &str) -> Option<String> {
    let theirs = tree.layout(id).ok()?;
    if !same(ours.layout(), theirs) {
        return Some(format!("{at}: ours {:?}, Taffy's {:?}", ours.layout(), theirs));
    }
    let kids = tree.children(id).ok()?;
    ours.children
        .iter()
        .zip(kids)
        .enumerate()
        .find_map(|(i, (c, k))| compare(c, tree, k, &format!("{at}/{i}")))
}

/// The paths of every node below the root's body, leaves and containers apart.
fn paths(d: &Desc, at: &mut Vec<usize>, leaves: &mut Vec<Vec<usize>>, boxes: &mut Vec<Vec<usize>>) {
    if d.children.is_empty() {
        leaves.push(at.clone());
    } else if at.len() > 1 {
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

/// Apply a change at `path` to `ours`, settling the path as the tree's owner does.
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
            AvailableSpace::Definite(800.0),
            AvailableSpace::MaxContent,
            AvailableSpace::MinContent,
        ]),
        height: r.pick(&[
            AvailableSpace::Definite(600.0),
            AvailableSpace::Definite(150.0),
            AvailableSpace::MaxContent,
            AvailableSpace::MinContent,
        ]),
    }
}

fn run(seed: u64, schedule: Schedule, steps: usize) -> Option<String> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut desc = table(&mut r);
    let mut space = space(&mut r);
    let mut ours = build_ours(&desc);
    ours.recount();
    let check = |ours: &Ours, desc: &Desc, space: Size<AvailableSpace>, what: &str| {
        let mut tree = TaffyTree::<Text>::new();
        tree.disable_rounding();
        let id = build_taffy(desc, &mut tree);
        tree.compute_layout_with_measure(id, space, measure).ok()?;
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
            // Another space: answers kept for one input meet another.
            0 => {
                space = self::space(&mut r);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} (space {space:?})")
            }
            1 | 2 if !boxes.is_empty() => {
                let path = boxes[r.below(boxes.len())].clone();
                let d = desc_at(&mut desc, &path);
                d.style = container_style(&mut r, &d.style);
                change(&mut ours, &path, &desc, true);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} (style at {path:?})")
            }
            k => {
                let path = leaves[r.below(leaves.len())].clone();
                let style = k == 3;
                {
                    let d = desc_at(&mut desc, &path);
                    if style {
                        d.style = item_style(&mut r);
                    } else {
                        d.text = Some(words(&mut r));
                    }
                }
                change(&mut ours, &path, &desc, style);
                ours.compute_root(space, &mut (), schedule);
                format!("step {step} ({} at {path:?})", if style { "style" } else { "text" })
            }
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

/// Every seed under every schedule, `steps` changes each, with or without
/// [`wild`] cells; the runs that differ, reported.
fn all(seeds: u64, steps: usize, wild: u8) {
    let failed: Vec<String> = (0..seeds)
        .flat_map(|seed| {
            SCHEDULES.iter().filter_map(move |&s| {
                WILD.with(|w| w.set(wild));
                run(seed, s, steps)
            })
        })
        .collect();
    assert!(
        failed.is_empty(),
        "{} runs differ:\n{}",
        failed.len(),
        failed.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn flex_rows_lay_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 0);
}

/// Rows with absolute items (Taffy's own flexbox lays them out), after every change.
#[test]
fn rows_with_absolute_items_lay_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 1);
}

/// Rows with absolute items and baseline alignment, laid out once.
#[test]
fn rows_with_absolute_or_baseline_items_lay_out_as_taffy() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 300 } else { 1200 }, 0, 2);
}

/// The same with baseline-aligned items, after every change (Taffy's own
/// flexbox runs for them, and finds their baselines with a final layout,
/// also in a size probe: see `grid_rows`).
#[test]
fn rows_with_baseline_items_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 2);
}
