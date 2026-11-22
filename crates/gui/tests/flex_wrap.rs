//! Flexbox on the retained tree beyond the row and column passes
//! (ADR-0014, `layout::engine::flex`, Taffy 0.15.1's flexbox ported)
//! against `TaffyTree` built anew: random multi-line containers (`wrap`,
//! `wrap-reverse`, `balance` and `balance-reverse` with `flex-line-count`),
//! in every direction and its reverse, left to right and right to left,
//! with every `align-content`, `justify-content` and `align-items`, gaps,
//! growing and shrinking items, `auto` margins, minimum and maximum sizes,
//! flex bases and sizing keywords, baseline alignment, relative and
//! absolute items, nested containers and text leaves, under definite, min-
//! and max-content space; then random changes to their text and their
//! style, each laid out and compared box by box, bit for bit (place, size,
//! scrollable overflow), serially, in parallel and in reverse. An aspect
//! ratio is not generated: gui resolves it as Chrome does, not as Taffy
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
use taffy::{FlexWrap, Layout, LayoutInput, LayoutOutput, Overflow, Point, TaffyTree};

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
        Some(AlignItems::STRETCH),
        Some(AlignItems::FLEX_START),
        Some(AlignItems::FLEX_END),
        Some(AlignItems::SAFE_CENTER),
        Some(AlignItems::BASELINE),
        Some(AlignItems::SELF_END),
    ])
}

/// An item's own style.
fn item_style(r: &mut Rng) -> Style {
    let mut s = Style {
        margin: Rect {
            left: lpa(r, &[0.0, 0.0, 4.0, -2.0]),
            right: lpa(r, &[0.0, 0.0, 3.0]),
            top: lpa(r, &[0.0, 0.0, 2.0]),
            bottom: lpa(r, &[0.0, 1.5]),
        },
        ..Style::default()
    };
    match r.below(12) {
        0 => s.margin.left = LengthPercentageAuto::auto(),
        1 => s.margin.top = LengthPercentageAuto::auto(),
        2 => {
            s.margin.top = LengthPercentageAuto::auto();
            s.margin.bottom = LengthPercentageAuto::auto();
        }
        _ => {}
    }
    s.padding = rect(r.pick(&[0.0, 0.0, 4.0, 2.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    s.align_self = align(r);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(50.0),
        Dimension::length(90.0),
        Dimension::percent(0.3),
        Dimension::min_content(),
        Dimension::fit_content(),
    ]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(25.0),
        Dimension::percent(0.4),
        Dimension::max_content(),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(40.0),
        LengthPercentageAuto::length(0.0),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(120.0),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(45.0),
    ]);
    s.flex_grow = r.pick(&[0.0, 0.0, 1.0, 2.0, 0.5]);
    s.flex_shrink = r.pick(&[1.0, 1.0, 0.0, 2.0, 0.3]);
    s.flex_basis = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(0.0),
        Dimension::length(60.0),
        Dimension::percent(0.25),
        Dimension::content(),
        Dimension::max_content(),
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
    match r.below(14) {
        0 => {
            s.position = Position::Relative;
            s.inset.top = LengthPercentageAuto::length(5.0);
            s.inset.left = LengthPercentageAuto::percent(0.1);
        }
        1 => {
            s.position = Position::Absolute;
            s.inset.right = LengthPercentageAuto::length(3.0);
            s.inset.bottom = LengthPercentageAuto::percent(0.1);
        }
        2 => s.direction = taffy::Direction::Rtl,
        3 => s.display = Display::None,
        _ => {}
    }
    s
}

/// A multi-line flex container's style.
fn container_style(r: &mut Rng) -> Style {
    let mut s = item_style(r);
    s.display = Display::Flex;
    s.flex_direction = r.pick(&[
        FlexDirection::Row,
        FlexDirection::Row,
        FlexDirection::RowReverse,
        FlexDirection::Column,
        FlexDirection::ColumnReverse,
    ]);
    s.flex_wrap = r.pick(&[
        FlexWrap::Wrap,
        FlexWrap::Wrap,
        FlexWrap::WrapReverse,
        FlexWrap::Balance,
        FlexWrap::BalanceReverse,
        FlexWrap::NoWrap,
    ]);
    s.flex_line_count = r.pick(&[1, 1, 1, 2, 3]);
    if r.below(4) == 0 {
        s.direction = taffy::Direction::Rtl;
    }
    s.gap = Size {
        width: r.pick(&[px(0.0), px(0.0), px(8.0), px(2.5), LengthPercentage::percent(0.02)]),
        height: r.pick(&[px(0.0), px(4.0), px(10.0)]),
    };
    s.align_items = r.pick(&[
        AlignItems::NORMAL,
        AlignItems::CENTER,
        AlignItems::START,
        AlignItems::END,
        AlignItems::FLEX_END,
        AlignItems::STRETCH,
        AlignItems::BASELINE,
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
        JustifyContent::SAFE_CENTER,
    ]);
    s.align_content = r.pick(&[
        AlignContent::NORMAL,
        AlignContent::NORMAL,
        AlignContent::CENTER,
        AlignContent::END,
        AlignContent::FLEX_END,
        AlignContent::SPACE_BETWEEN,
        AlignContent::SPACE_AROUND,
        AlignContent::SPACE_EVENLY,
        AlignContent::STRETCH,
        AlignContent::SAFE_END,
    ]);
    s.size.width = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(260.0),
        Dimension::percent(0.6),
    ]);
    s.size.height = r.pick(&[Dimension::auto(), Dimension::auto(), Dimension::length(120.0)]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(200.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(150.0),
    ]);
    s.scrollbar_width = 8.0;
    s
}

fn text(r: &mut Rng) -> Desc {
    Desc {
        style: item_style(r),
        text: Some(words(r)),
        children: Vec::new(),
    }
}

fn container(r: &mut Rng, depth: u32) -> Desc {
    let n = match r.below(6) {
        0 => 0,
        1 => 1,
        _ => 1 + r.below(9),
    };
    let children = (0..n)
        .map(|_| {
            if depth > 0 && r.below(4) == 0 {
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
    let n = 1 + r.below(4);
    let children = (0..n).map(|_| container(r, 2)).collect();
    let style = Style {
        display: r.pick(&[Display::Block, Display::Flex, Display::Flex]),
        flex_direction: r.pick(&[FlexDirection::Column, FlexDirection::Row]),
        flex_wrap: r.pick(&[FlexWrap::NoWrap, FlexWrap::Wrap]),
        position: Position::Relative,
        ..Style::default()
    };
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
                        d.style = item_style(&mut r);
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
fn wrapping_flex_lays_out_as_taffy_after_every_change() {
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
