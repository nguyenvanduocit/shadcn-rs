//! Flex columns laid out by the retained tree's own column pass (ADR-0014,
//! `layout::engine::stack`), which keeps each item's answers between runs,
//! against `TaffyTree` built anew: random columns of items of every kind
//! the pass takes and some it does not (laid out by the ported flexbox,
//! `layout::engine::flex`: reversed, multi-line, right to left, aligned
//! columns; items aligned to their baseline or an edge, sized by keyword or
//! with `auto` margins), then random changes to their text and their style,
//! each laid out and compared box by box, bit for bit (place, size,
//! scrollable overflow), serially, in parallel and in reverse.

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

/// A tree both sides are built from.
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
    let n = 1 + r.below(8);
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

/// An item's style: mostly what the column pass takes, sometimes not.
fn item_style(r: &mut Rng) -> Style {
    let mut s = Style::default();
    let m = r.pick(&[0.0, 0.0, 3.0, -2.0, 1.5]);
    s.margin = Rect {
        left: LengthPercentageAuto::length(r.pick(&[0.0, m])),
        right: LengthPercentageAuto::length(r.pick(&[0.0, 2.0])),
        top: LengthPercentageAuto::length(m),
        bottom: LengthPercentageAuto::length(r.pick(&[0.0, 4.0, m])),
    };
    s.padding = rect(r.pick(&[0.0, 0.0, 4.0, 2.5]));
    s.border = rect(r.pick(&[0.0, 0.0, 1.0]));
    s.flex_grow = r.pick(&[0.0, 0.0, 0.0, 1.0, 2.0]);
    s.flex_shrink = r.pick(&[1.0, 1.0, 0.0, 3.0]);
    s.flex_basis = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(30.0),
        Dimension::percent(0.1),
    ]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(25.0),
        Dimension::percent(0.2),
    ]);
    s.min_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(30.0),
        LengthPercentageAuto::length(0.0),
    ]);
    s.max_size.height = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(50.0),
    ]);
    s.min_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(40.0),
    ]);
    s.max_size.width = r.pick(&[
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::auto(),
        LengthPercentageAuto::length(150.0),
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
    }
    // Now and then an item the pass does not take: the ported flexbox
    // (`engine::flex`) runs instead.
    match r.below(30) {
        0 => s.size.width = Dimension::length(80.0),
        1 => s.align_self = Some(AlignItems::CENTER),
        2 => s.position = Position::Absolute,
        3 => s.display = Display::None,
        4 => s.align_self = Some(AlignItems::BASELINE),
        5 => s.align_self = Some(AlignItems::FLEX_END),
        6 => s.size.width = Dimension::max_content(),
        7 => s.size.height = Dimension::fit_content_px(30.0),
        8 => s.margin.left = LengthPercentageAuto::auto(),
        9 => s.margin.top = LengthPercentageAuto::auto(),
        _ => {}
    }
    s
}

fn item(r: &mut Rng, depth: u32) -> Desc {
    let style = item_style(r);
    if depth > 0 && r.below(3) == 0 {
        let mut s = style;
        s.display = Display::Flex;
        s.flex_direction = r.pick(&[FlexDirection::Row, FlexDirection::Column]);
        s.gap = Size {
            width: px(r.pick(&[0.0, 4.0])),
            height: px(r.pick(&[0.0, 3.0])),
        };
        let n = 1 + r.below(3);
        return Desc {
            style: s,
            text: None,
            children: (0..n).map(|_| item(r, depth - 1)).collect(),
        };
    }
    Desc {
        style,
        text: Some(words(r)),
        children: Vec::new(),
    }
}

/// A column of many items, in a root of a random size.
fn column(r: &mut Rng) -> Desc {
    let mut s = Style {
        display: Display::Flex,
        flex_direction: FlexDirection::Column,
        ..Style::default()
    };
    s.gap.height = r.pick(&[px(0.0), px(0.0), px(5.0), LengthPercentage::percent(0.01)]);
    s.padding = rect(r.pick(&[0.0, 3.0]));
    s.border = rect(r.pick(&[0.0, 1.0]));
    s.justify_content = r.pick(&[
        JustifyContent::NORMAL,
        JustifyContent::NORMAL,
        JustifyContent::CENTER,
        JustifyContent::SPACE_BETWEEN,
        JustifyContent::FLEX_END,
        JustifyContent::SPACE_EVENLY,
    ]);
    s.align_content = r.pick(&[AlignContent::NORMAL, AlignContent::CENTER, AlignContent::FLEX_END]);
    s.size.height = r.pick(&[
        Dimension::auto(),
        Dimension::auto(),
        Dimension::length(400.0),
        Dimension::length(90.0),
    ]);
    s.min_size.height = r.pick(&[LengthPercentageAuto::auto(), LengthPercentageAuto::length(200.0)]);
    s.max_size.height = r.pick(&[LengthPercentageAuto::auto(), LengthPercentageAuto::length(700.0)]);
    let o = r.pick(&[Overflow::Visible, Overflow::Visible, Overflow::Scroll]);
    s.overflow = Point { x: o, y: o };
    s.scrollbar_width = 10.0;
    // Now and then a column the pass does not take: the ported flexbox
    // lays it out, multi-line, reversed, right to left or aligned.
    match r.below(10) {
        0 => s.flex_direction = FlexDirection::ColumnReverse,
        1 => {
            s.flex_wrap = r.pick(&[FlexWrap::Wrap, FlexWrap::WrapReverse]);
            s.align_content = r.pick(&[
                AlignContent::NORMAL,
                AlignContent::CENTER,
                AlignContent::SPACE_BETWEEN,
                AlignContent::STRETCH,
                AlignContent::FLEX_END,
            ]);
            s.gap.width = px(r.pick(&[0.0, 6.0]));
        }
        2 => s.direction = taffy::Direction::Rtl,
        3 => {
            s.align_items = r.pick(&[
                AlignItems::CENTER,
                AlignItems::FLEX_START,
                AlignItems::END,
                AlignItems::BASELINE,
            ]);
        }
        _ => {}
    }
    let n = 1 + r.below(60);
    let body = Desc {
        style: s,
        text: None,
        children: (0..n).map(|_| item(r, 2)).collect(),
    };
    let root = Style {
        display: Display::Flex,
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
    // Display-none boxes keep whatever their last layout was in Taffy too.
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

/// The paths of every leaf.
fn leaves(d: &Desc, at: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    if d.children.is_empty() {
        out.push(at.clone());
    }
    for (i, c) in d.children.iter().enumerate() {
        at.push(i);
        leaves(c, at, out);
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
        width: AvailableSpace::Definite(r.pick(&[300.0, 520.0, 800.0])),
        height: r.pick(&[AvailableSpace::Definite(600.0), AvailableSpace::MaxContent]),
    }
}

fn run(seed: u64, schedule: Schedule) -> Option<String> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut desc = column(&mut r);
    let space = space(&mut r);
    let mut ours = build_ours(&desc);
    ours.recount();
    let check = |ours: &Ours, desc: &Desc, what: &str| {
        let mut tree = TaffyTree::<Text>::new();
        tree.disable_rounding();
        let id = build_taffy(desc, &mut tree);
        tree.compute_layout_with_measure(id, space, measure).ok()?;
        compare(ours, &tree, id, "").map(|d| format!("seed {seed} {schedule:?} {what}{d}"))
    };
    ours.compute_root(space, &mut (), schedule);
    if let Some(d) = check(&ours, &desc, "cold") {
        return Some(d);
    }
    let mut paths = Vec::new();
    leaves(&desc, &mut Vec::new(), &mut paths);
    for step in 0..40 {
        let path = paths[r.below(paths.len())].clone();
        let style = r.below(4) == 0;
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
        let what = format!("step {step} ({} at {path:?})", if style { "style" } else { "text" });
        if let Some(d) = check(&ours, &desc, &what) {
            return Some(d);
        }
    }
    None
}

#[test]
fn flex_columns_lay_out_as_taffy_after_every_change() {
    gui::layout::engine::set_memo_min(0);
    let schedules = [
        Schedule::default(),
        Schedule {
            parallel: true,
            reverse: false,
        },
        Schedule {
            parallel: false,
            reverse: true,
        },
    ];
    let failed: Vec<String> = (0..300_u64)
        .flat_map(|seed| schedules.iter().filter_map(move |&s| run(seed, s)))
        .collect();
    assert!(
        failed.is_empty(),
        "{} runs differ:\n{}",
        failed.len(),
        failed.iter().take(20).cloned().collect::<Vec<_>>().join("\n")
    );
}
