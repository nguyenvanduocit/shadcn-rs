//! Grids of rows laid out by the retained tree's own grid pass (ADR-0014,
//! `layout::engine::grid`) against `TaffyTree` built anew: random tables
//! and forms of the case the pass takes and some just outside it (a
//! `repeat()`, an `auto` column, a span, a template row, right to left),
//! inside flex columns and rows, blocks and grids, under definite, min- and
//! max-content space; then random changes to their text and their style,
//! each laid out and compared box by box, bit for bit (place, size,
//! scrollable overflow), serially, in parallel and in reverse, with
//! absolute cells and baseline alignment too. An aspect ratio is not
//! generated: gui resolves it as Chrome does, not as Taffy
//! (`engine/sizing.rs`), and is compared with Chrome in `chrome_layout`.

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
    GridTemplateComponent, GridTemplateRepetition, Layout, LayoutInput, LayoutOutput, MaxTrackSizingFunction,
    MinTrackSizingFunction, Overflow, Point, RepetitionCount, TaffyTree, TrackSizingFunction,
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

/// A generator, with what grids generate besides the pass's case: 1
/// out-of-flow cells, 2 baseline alignment as well.
struct Rng(u64, u8);

impl Rng {
    fn wild(&self) -> bool {
        self.1 >= 1
    }

    fn baseline(&self) -> bool {
        self.1 >= 2
    }

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

fn single(t: TrackSizingFunction) -> GridTemplateComponent<String> {
    GridTemplateComponent::Single(t)
}

fn fr0(f: f32) -> TrackSizingFunction {
    TrackSizingFunction {
        min: MinTrackSizingFunction::length(0.0),
        max: MaxTrackSizingFunction::fr(f),
    }
}

/// A column: mostly what the pass takes, sometimes not.
fn track(r: &mut Rng) -> GridTemplateComponent<String> {
    let fixed = |min: MinTrackSizingFunction, max: MaxTrackSizingFunction| single(TrackSizingFunction { min, max });
    match r.below(24) {
        0..=5 => single(fr0(1.0)),
        6..=7 => single(fr0(2.0)),
        8 => single(fr0(0.5)),
        9..=11 => fixed(
            MinTrackSizingFunction::length(40.0),
            MaxTrackSizingFunction::length(40.0),
        ),
        12 => fixed(
            MinTrackSizingFunction::length(25.5),
            MaxTrackSizingFunction::length(25.5),
        ),
        13 => fixed(
            MinTrackSizingFunction::percent(0.2),
            MaxTrackSizingFunction::percent(0.2),
        ),
        14 => fixed(
            MinTrackSizingFunction::length(10.0),
            MaxTrackSizingFunction::length(60.0),
        ),
        15 => fixed(MinTrackSizingFunction::length(30.0), MaxTrackSizingFunction::fr(1.0)),
        16 => fixed(
            MinTrackSizingFunction::length(0.0),
            MaxTrackSizingFunction::percent(0.3),
        ),
        // Outside the case.
        17 => fixed(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::fr(1.0)),
        18 => fixed(MinTrackSizingFunction::auto(), MaxTrackSizingFunction::auto()),
        19 => fixed(
            MinTrackSizingFunction::min_content(),
            MaxTrackSizingFunction::max_content(),
        ),
        20 => GridTemplateComponent::Repeat(GridTemplateRepetition {
            count: RepetitionCount::Count(2),
            tracks: vec![fr0(1.0)],
            line_names: Vec::new(),
        }),
        _ => single(fr0(1.0)),
    }
}

fn align(r: &mut Rng) -> Option<AlignItems> {
    r.pick(&[
        None,
        None,
        None,
        Some(AlignItems::START),
        Some(AlignItems::END),
        Some(AlignItems::CENTER),
        Some(AlignItems::STRETCH),
        Some(AlignItems::FLEX_END),
        Some(AlignItems::SAFE_CENTER),
        Some(AlignItems::SELF_END),
    ])
}

fn align_items(r: &mut Rng) -> AlignItems {
    align(r).unwrap_or(AlignItems::NORMAL)
}

/// A cell's own style as an item of its row.
fn cell_style(r: &mut Rng) -> Style {
    let mut s = Style::default();
    let m = r.pick(&[0.0, 0.0, 3.0, -2.0, 1.5]);
    s.margin = Rect {
        left: LengthPercentageAuto::length(r.pick(&[0.0, m])),
        right: LengthPercentageAuto::length(r.pick(&[0.0, 2.0])),
        top: LengthPercentageAuto::length(m),
        bottom: LengthPercentageAuto::length(r.pick(&[0.0, 4.0, m])),
    };
    if r.below(12) == 0 {
        s.margin.left = LengthPercentageAuto::auto();
    }
    if r.below(12) == 0 {
        s.margin.top = LengthPercentageAuto::auto();
    }
    if r.below(15) == 0 {
        s.margin.right = LengthPercentageAuto::percent(0.05);
    }
    s.padding = rect(r.pick(&[0.0, 0.0, 4.0, 2.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    s.align_self = align(r);
    s.justify_self = align(r);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(25.0),
        Dimension::percent(0.5),
    ]);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(50.0),
        Dimension::percent(0.5),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
        LengthPercentageAuto::length(0.0),
        LengthPercentageAuto::length(5.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(40.0),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(70.0),
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
    }
    // Now and then a cell the pass does not take, or hides.
    match r.below(40) {
        0 => {
            s.grid_column = Line {
                start: GridPlacement::Auto,
                end: GridPlacement::Span(2),
            }
        }
        1 if r.wild() => s.position = Position::Absolute,
        2 if r.baseline() => s.align_self = Some(AlignItems::BASELINE),
        3 => s.size.width = Dimension::fit_content(),
        4 | 5 => s.display = Display::None,
        6 => {
            s.grid_row = Line {
                start: GridPlacement::Span(1),
                end: GridPlacement::Auto,
            }
        }
        _ => {}
    }
    s
}

/// What a row of a grid is: its columns, gaps and alignment.
fn grid_style(r: &mut Rng, cells: usize) -> Style {
    let n = if r.below(4) == 0 { 1 + r.below(4) } else { cells.max(1) };
    let mut s = Style {
        display: Display::Grid,
        grid_template_columns: (0..n).map(|_| track(r)).collect(),
        ..Style::default()
    };
    s.gap = Size {
        width: px(r.pick(&[0.0, 0.0, 8.0, 2.5])),
        height: px(r.pick(&[0.0, 0.0, 4.0])),
    };
    s.padding = rect(r.pick(&[0.0, 0.0, 6.0]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    s.align_items = align_items(r);
    s.justify_items = align_items(r);
    s.justify_content = r.pick(&[
        JustifyContent::NORMAL,
        JustifyContent::NORMAL,
        JustifyContent::CENTER,
        JustifyContent::END,
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
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(48.0),
    ]);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(260.0),
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
        Overflow::Scroll,
    ]);
    s.overflow = Point { x: o, y: o };
    s.scrollbar_width = 8.0;
    s.box_sizing = r.pick(&[BoxSizing::BorderBox, BoxSizing::BorderBox, BoxSizing::ContentBox]);
    if r.below(6) == 0 {
        s.grid_auto_rows = vec![TrackSizingFunction::AUTO];
    }
    match r.below(30) {
        0 => s.gap.width = LengthPercentage::percent(0.02),
        1 => s.grid_template_rows = vec![single(fr0(1.0))],
        2 => s.direction = taffy::Direction::Rtl,
        3 if r.baseline() => s.align_items = AlignItems::BASELINE,
        4 => s.grid_auto_flow = GridAutoFlow::RowDense,
        5 => s.grid_auto_rows = vec![TrackSizingFunction::from_length(30.0)],
        6 => s.position = Position::Relative,
        _ => {}
    }
    s
}

fn text_cell(r: &mut Rng) -> Desc {
    Desc {
        style: cell_style(r),
        text: Some(words(r)),
        children: Vec::new(),
    }
}

/// A cell: text, a flex row of texts centred, or a nested grid.
fn cell(r: &mut Rng, depth: u32) -> Desc {
    match r.below(6) {
        0 | 1 if depth > 0 => {
            let mut s = cell_style(r);
            s.display = Display::Flex;
            s.flex_direction = FlexDirection::Row;
            s.align_items = r.pick(&[AlignItems::CENTER, AlignItems::CENTER, AlignItems::NORMAL]);
            s.gap.width = px(r.pick(&[0.0, 4.0]));
            let n = 1 + r.below(2);
            let mut children: Vec<Desc> = (0..n)
                .map(|_| Desc {
                    style: Style::default(),
                    text: Some(words(r)),
                    children: Vec::new(),
                })
                .collect();
            if r.below(10) == 0 && r.wild() {
                children.push(Desc {
                    style: Style {
                        position: Position::Absolute,
                        ..Style::default()
                    },
                    text: Some(words(r)),
                    children: Vec::new(),
                });
            }
            Desc {
                style: s,
                text: None,
                children,
            }
        }
        2 if depth > 0 => {
            let n = 1 + r.below(3);
            let children: Vec<Desc> = (0..n).map(|_| cell(r, depth - 1)).collect();
            let mut s = grid_style(r, n);
            let item = cell_style(r);
            s.margin = item.margin;
            s.align_self = item.align_self;
            s.justify_self = item.justify_self;
            Desc {
                style: s,
                text: None,
                children,
            }
        }
        _ => text_cell(r),
    }
}

fn row(r: &mut Rng) -> Desc {
    let n = 1 + r.below(6);
    let children: Vec<Desc> = (0..n).map(|_| cell(r, 2)).collect();
    Desc {
        style: grid_style(r, n),
        text: None,
        children,
    }
}

/// A body of rows (a flex column, a flex row, a block or a grid) in a root.
fn table(r: &mut Rng) -> Desc {
    let n = 1 + r.below(24);
    let rows: Vec<Desc> = (0..n).map(|_| row(r)).collect();
    let mut body = match r.below(5) {
        0 | 1 => Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
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
        _ => {
            let mut g = grid_style(r, 1);
            g.grid_template_columns = vec![single(fr0(1.0)), track(r)];
            g
        }
    };
    body.padding = rect(r.pick(&[0.0, 3.0]));
    body.gap.height = px(r.pick(&[0.0, 2.0]));
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

fn run(seed: u64, schedule: Schedule, steps: usize, wild: u8) -> Option<String> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1, wild);
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
                let cells = d.children.len();
                d.style = if d.style.display == Display::Grid {
                    let mut s = grid_style(&mut r, cells);
                    if r.below(2) == 0 {
                        let item = cell_style(&mut r);
                        s.margin = item.margin;
                        s.align_self = item.align_self;
                    }
                    s
                } else {
                    let mut s = cell_style(&mut r);
                    s.display = d.style.display;
                    s.flex_direction = d.style.flex_direction;
                    s.align_items = d.style.align_items;
                    s
                };
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
                        d.style = cell_style(&mut r);
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
/// wild cells ([`Rng::wild`]); the runs that differ, reported.
fn all(seeds: u64, steps: usize, wild: u8) {
    let failed: Vec<String> = (0..seeds)
        .flat_map(|seed| SCHEDULES.iter().filter_map(move |&s| run(seed, s, steps, wild)))
        .collect();
    assert!(
        failed.is_empty(),
        "{} runs differ:\n{}",
        failed.len(),
        failed.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn grid_rows_lay_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 0);
}

/// Grids with absolute cells (Taffy's own grid lays them out), after every change.
#[test]
fn grids_with_absolute_cells_lay_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 1);
}

/// Grids with absolute cells and baseline alignment (Taffy's own grid lays
/// them out), laid out once.
#[test]
fn grids_with_absolute_or_baseline_cells_lay_out_as_taffy() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 300 } else { 1200 }, 0, 2);
}

/// The same with baseline-aligned cells, after every change (Taffy's own
/// grid and flexbox run for them).
#[test]
fn grids_with_baseline_cells_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    all(if cfg!(debug_assertions) { 160 } else { 600 }, 24, 2);
}

/// A size probe of a grid lays its baseline-aligned cells out (Taffy finds
/// their baselines with a final layout at min-content), which places their
/// own children for that probe. The grid's final layout, still cached, must
/// not stand on them: the cells are laid out again where it puts them.
#[test]
fn a_probe_that_lays_out_a_baseline_cell_does_not_leave_its_children_stale() {
    let text = |t: &str| Desc {
        style: Style::default(),
        text: Some(Text(t.to_owned())),
        children: Vec::new(),
    };
    let cell = Desc {
        style: Style {
            display: Display::Flex,
            ..Style::default()
        },
        text: None,
        children: vec![text("xxx xxx xxx xxx")],
    };
    let grid = Desc {
        style: Style {
            display: Display::Grid,
            grid_template_columns: vec![single(fr0(1.0)), single(fr0(1.0))],
            align_items: AlignItems::BASELINE,
            ..Style::default()
        },
        text: None,
        children: vec![cell, text("xx")],
    };
    let desc = Desc {
        style: Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            ..Style::default()
        },
        text: None,
        children: vec![grid],
    };
    let mut ours = build_ours(&desc);
    ours.recount();
    let schedule = Schedule::default();
    for height in [
        AvailableSpace::MaxContent,
        AvailableSpace::Definite(600.0),
        AvailableSpace::MinContent,
        AvailableSpace::MaxContent,
    ] {
        let space = Size {
            width: AvailableSpace::Definite(400.0),
            height,
        };
        ours.compute_root(space, &mut (), schedule);
        let mut tree = TaffyTree::<Text>::new();
        tree.disable_rounding();
        let id = build_taffy(&desc, &mut tree);
        tree.compute_layout_with_measure(id, space, measure).expect("layout");
        assert_eq!(compare(&ours, &tree, id, ""), None, "in {space:?}");
    }
}
