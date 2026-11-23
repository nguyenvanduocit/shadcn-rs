//! A grid of rows laid out by Taffy's grid algorithm step for step, the
//! case a table row or a form is (ADR-0014).
//!
//! Taffy's `compute_grid_layout` is general: named lines and areas, the
//! placement of items anywhere, `repeat()`, spans, baselines, the detailed
//! grid information for out-of-flow children. A row of a table uses none of
//! it, and pays for all of it on each of the three runs its parent asks for
//! (a min-content and a max-content probe of its height, then its final
//! layout). This is that algorithm for one case, every step it takes there
//! kept in Taffy's order of float operations, so the result is
//! bit-identical; what it allocates is kept between runs.
//!
//! It takes a left-to-right grid with no areas and no line names, its width
//! known, its columns an explicit list of fixed tracks or `minmax(<fixed>,
//! <n>fr)`, no explicit rows and `auto` implicit ones, `grid-auto-flow:
//! row`, and every child either `display: none` or in flow, auto-placed with
//! a span of one, with no sizing keyword, no baseline alignment and
//! nothing out of flow inside it. Columns then size without
//! asking any item anything (a fixed or `fr` track reads no content), and
//! each item fills the next cell in order. Anything else is Taffy's own
//! algorithm, before any child was asked.
//!
//! It also records whether its answer read the available height (Taffy's
//! cache assumes it does). Without a known or preferred height the space
//! available to the rows is that height, read in two places only:
//!
//! - Resolving intrinsic track sizes, an `auto` minimum takes an item's
//!   minimum contribution under a definite space and the larger of it and
//!   the item's (limited) min-content contribution under a min- or
//!   max-content one (`resolve_intrinsic_track_sizes`, the span-1 path).
//! - Maximising tracks hands free space to tracks whose growth limit
//!   exceeds their base size (`maximise_tracks`): all of it under
//!   max-content, none under min-content, the space left under a definite
//!   one.
//!
//! The rows' expansion steps map an unknown height to max-content and read
//! nothing of it. So an answer stands for every available height when each
//! item's two candidates are the same value and every row's growth limit
//! equals its base size; the cache then matches it whatever the height
//! ([`super::cache::Answer::free_height`]).
//!
//! An item's `aspect-ratio` is its own to resolve (`sizing.rs`), where
//! Taffy's grid carries it through the item's size, min and max as definite
//! sizes: the item is given its stretched width (within its min and max,
//! which its ratio carries into an `auto` axis only), and its ratio gives
//! its height, at least what its content needs; `normal` does not stretch
//! it in the block axis.

use std::cell::RefCell;

use taffy::style_helpers::TaffyZero;
use taffy::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignSelf, AvailableSpace, Baselines,
    BoxGenerationMode, BoxSizing, CompactLength, Contain, CoreStyle, Dimension, Direction,
    ExpandedMaxTrackSizingFunction, ExpandedMinTrackSizingFunction, GenericGridTemplateComponent, GridAutoFlow,
    GridContainerStyle, GridItemStyle, GridPlacement, Layout, LayoutInput, LayoutOutput, LengthPercentage,
    LengthPercentageAuto, Line, MaxTrackSizingFunction, MaybeMath, MaybeResolve, MinTrackSizingFunction, NodeId,
    OofCandidates, OofPositioningArea, Overflow, Point, Position, Rect, RequestedAxis, ResolveOrZero, RunMode, Size,
    SizingMode,
};

use super::sizing::{Laid, Ratio, Reports, ratio_of, transfer, with_ratio};
use super::{Measure, NodeStyle, View};
use crate::convert::usize_to_f32;

/// Taffy's `MAX_GRID_TRACKS`: past it Taffy clamps placements, which this does not.
const MAX_TRACKS: usize = 10_000;

/// One track or gutter (Taffy's `GridTrack`, without what this case never sets).
#[derive(Clone, Copy, Debug)]
struct Track {
    min: MinTrackSizingFunction,
    max: MaxTrackSizingFunction,
    collapsed: bool,
    offset: f32,
    base: f32,
    growth: f32,
    incurred: f32,
    base_planned: f32,
    growth_planned: f32,
}

impl Track {
    fn new(min: MinTrackSizingFunction, max: MaxTrackSizingFunction) -> Self {
        Self {
            min,
            max,
            collapsed: false,
            offset: 0.0,
            base: 0.0,
            growth: 0.0,
            incurred: 0.0,
            base_planned: 0.0,
            growth_planned: 0.0,
        }
    }

    fn gutter(gap: LengthPercentage) -> Self {
        Self::new(MinTrackSizingFunction::from(gap), MaxTrackSizingFunction::from(gap))
    }

    fn collapse(&mut self) {
        self.collapsed = true;
        self.min = MinTrackSizingFunction::ZERO;
        self.max = MaxTrackSizingFunction::ZERO;
    }

    /// `fit_content_limited_growth_limit`: no track here is `fit-content()`.
    fn limited_growth(&self) -> f32 {
        self.growth.min(f32::INFINITY)
    }
}

/// One grid item: its style as Taffy's `GridItem` keeps it, and what it
/// answered in this run (its block-axis contributions, without margins).
#[derive(Clone, Copy, Debug)]
struct Item {
    child: usize,
    /// Its row and column track (an index into the track lists, gutters included).
    row: usize,
    col: usize,
    overflow: Point<Overflow>,
    box_sizing: BoxSizing,
    size: Size<Dimension>,
    min_size: Size<LengthPercentageAuto>,
    max_size: Size<LengthPercentageAuto>,
    padding: Rect<LengthPercentage>,
    border: Rect<LengthPercentage>,
    margin: Rect<LengthPercentageAuto>,
    /// Its ratio: its own to resolve (`sizing.rs`); it carries its min and
    /// max only into an `auto` axis.
    aspect_ratio: Option<f32>,
    align_self: AlignSelf,
    justify_self: AlignSelf,
    crosses_flexible_column: bool,
    area: Option<Size<Option<f32>>>,
    known: Option<Known>,
    min_content: Option<f32>,
    max_content: Option<f32>,
    minimum: Option<f32>,
    /// Its contributions to its row, margins included: its minimum, its
    /// limited min-content one (none for a scroll container), its
    /// max-content one; and the cache generation of the child that gave them.
    min_contribution: f32,
    limited_contribution: Option<f32>,
    max_contribution: f32,
    generation: u32,
    y: f32,
    height: f32,
    baseline: Option<f32>,
}

/// A grid area size and the size the item is measured at in it.
type Known = (Size<Option<f32>>, Size<Option<f32>>);

/// What a run works in, kept between runs (one per nesting level and thread).
#[derive(Default)]
struct Scratch {
    items: Vec<Item>,
    columns: Vec<Track>,
    rows: Vec<Track>,
}

/// What a grid keeps from its last run, so the next (its final layout
/// after its size probes, as a column asks them) does not ask its children
/// their contributions again: its items' contributions to their rows (its
/// columns, sized from its style and width alone, are sized again). Neither depends on the height
/// the grid is given (its rows are `auto`: no track reads it, and an item
/// is measured under min- and max-content alone); the height only decides,
/// as the rows are sized from the contributions again, which of an item's
/// two it adds. It stands while the grid's style and children are
/// unchanged (dropped when they change), its columns are sized for the same
/// width, and no item's cache was cleared since it answered (its
/// generation). Only plain numbers are kept: Taffy's track and length
/// values may not leave their thread.
#[derive(Debug, Default)]
pub struct Memo {
    /// `None`: nothing kept.
    key: Option<[u32; 3]>,
    items: Vec<Contribution>,
}

impl Memo {
    /// Drop what is kept (its grid's style or children changed).
    pub fn clear(&mut self) {
        self.key = None;
        self.items.clear();
    }
}

/// An item's contributions to its row, margins included, and the cache
/// generation of the child that gave them.
#[derive(Clone, Copy, Debug)]
struct Contribution {
    minimum: f32,
    limited: Option<f32>,
    max: f32,
    generation: u32,
}

/// What the columns and the contributions were found from.
fn memo_key(c: &Constants) -> [u32; 3] {
    let bits = |v: Option<f32>| v.map_or(u32::MAX, f32::to_bits);
    let space = match c.available_grid_space.width {
        AvailableSpace::Definite(v) => v.to_bits(),
        AvailableSpace::MinContent => u32::MAX - 1,
        AvailableSpace::MaxContent => u32::MAX - 2,
    };
    [bits(c.inner_node_size.width), bits(c.inner_min_size.width), space]
}

/// Fill `s`'s items' contributions from `view`'s memo, if it stands for
/// this run (`s` as [`accept`] filled it).
fn recall<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, c: &Constants, s: &mut Scratch) -> bool {
    let memo = &view.node.grid;
    let kids = &view.node.children;
    if memo.key != Some(memo_key(c))
        || memo.items.len() != s.items.len()
        || !s
            .items
            .iter()
            .zip(&memo.items)
            .all(|(i, m)| kids.get(i.child).is_some_and(|k| k.cache.generation() == m.generation))
    {
        return false;
    }
    for (i, m) in s.items.iter_mut().zip(&memo.items) {
        i.min_contribution = m.minimum;
        i.limited_contribution = m.limited;
        i.max_contribution = m.max;
        i.generation = m.generation;
    }
    // Their size answers are used again: the grid uses more of each item
    // than a final layout at a size it fixed (none is a boundary).
    for item in &s.items {
        if let Some(k) = view.node.children.get_mut(item.child) {
            k.tight = false;
        }
    }
    true
}

/// Keep what this run found for the next.
fn remember<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, c: &Constants, s: &Scratch) {
    let memo = &mut view.node.grid;
    memo.key = Some(memo_key(c));
    memo.items.clear();
    memo.items.extend(s.items.iter().map(|i| Contribution {
        minimum: i.min_contribution,
        limited: i.limited_contribution,
        max: i.max_contribution,
        generation: i.generation,
    }));
}

/// Buffers kept between runs (on a pool thread, until its job group ends),
/// and the most items one may hold to be kept.
const SPARE: usize = 8;
const SPARE_ITEMS: usize = 256;

crate::engine::engine_local! {
    [Fresh] static SCRATCH: RefCell<Vec<Scratch>> = const { RefCell::new(Vec::new()) };
}

fn take_scratch() -> Scratch {
    SCRATCH
        .with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.pop()))
        .unwrap_or_default()
}

/// Drop this thread's kept buffers: a pool thread lets go of them when its
/// job group ends, so what is held after a pass does not depend on which
/// threads the pool happened to run it on.
pub(super) fn release_scratch() {
    SCRATCH.with(|p| {
        if let Ok(mut p) = p.try_borrow_mut() {
            *p = Vec::new();
        }
    });
}

fn give_scratch(mut s: Scratch) {
    s.items.clear();
    s.columns.clear();
    s.rows.clear();
    SCRATCH.with(|p| {
        // A few levels of nesting and a run of common size are kept: a
        // deep chain or a huge container takes fresh buffers rather than
        // leaving every pool thread holding its largest.
        if let Ok(mut p) = p.try_borrow_mut()
            && p.len() < SPARE
            && s.items.capacity() <= SPARE_ITEMS
        {
            p.push(s);
        }
    });
}

/// `a + b` as Taffy's `Rect` adds, side by side.
pub(super) fn rect_add(a: Rect<f32>, b: Rect<f32>) -> Rect<f32> {
    Rect {
        left: a.left + b.left,
        right: a.right + b.right,
        top: a.top + b.top,
        bottom: a.bottom + b.bottom,
    }
}

pub(super) fn size_sub(a: Size<f32>, b: Size<f32>) -> Size<f32> {
    Size {
        width: a.width - b.width,
        height: a.height - b.height,
    }
}

fn is_length(v: LengthPercentage) -> bool {
    v.into_raw().tag() == CompactLength::LENGTH_TAG
}

/// Auto-placed with a span of one.
fn auto_span_one<I: taffy::CheapCloneStr>(line: &Line<GridPlacement<I>>) -> bool {
    let one = |p: &GridPlacement<I>| matches!(p, GridPlacement::Auto | GridPlacement::Span(1));
    one(&line.start) && one(&line.end)
}

fn is_baseline(a: AlignItems) -> bool {
    a.keyword == AlignItemsKeyword::Baseline
}

/// Taffy's `apply_alignment_fallback`.
pub(super) fn alignment_fallback(free_space: f32, items: usize, mode: AlignContent) -> AlignContentKeyword {
    let mut keyword = match mode.keyword {
        AlignContentKeyword::Normal => AlignContentKeyword::Stretch,
        keyword => keyword,
    };
    let mut is_safe = mode.is_safe();
    if items <= 1 || free_space <= 0.0 {
        (keyword, is_safe) = match keyword {
            AlignContentKeyword::Stretch | AlignContentKeyword::SpaceBetween => (AlignContentKeyword::FlexStart, true),
            AlignContentKeyword::SpaceAround | AlignContentKeyword::SpaceEvenly => (AlignContentKeyword::Center, true),
            other => (other, is_safe),
        };
    }
    if free_space <= 0.0 && is_safe {
        keyword = AlignContentKeyword::Start;
    }
    keyword
}

/// Taffy's `compute_alignment_offset` (never reversed, no gap: a grid's gaps are tracks).
fn alignment_offset(free_space: f32, items: usize, mode: AlignContentKeyword, first: bool) -> f32 {
    if first {
        match mode {
            AlignContentKeyword::Start
            | AlignContentKeyword::FlexStart
            | AlignContentKeyword::Normal
            | AlignContentKeyword::Stretch
            | AlignContentKeyword::SpaceBetween => 0.0,
            AlignContentKeyword::End | AlignContentKeyword::FlexEnd => free_space,
            AlignContentKeyword::Center => free_space / 2.0,
            AlignContentKeyword::SpaceAround => {
                if free_space >= 0.0 {
                    (free_space / usize_to_f32(items)) / 2.0
                } else {
                    free_space / 2.0
                }
            }
            AlignContentKeyword::SpaceEvenly => {
                if free_space >= 0.0 {
                    free_space / usize_to_f32(items.saturating_add(1))
                } else {
                    free_space / 2.0
                }
            }
        }
    } else {
        let free_space = free_space.max(0.0);
        0.0 + match mode {
            AlignContentKeyword::Start
            | AlignContentKeyword::FlexStart
            | AlignContentKeyword::End
            | AlignContentKeyword::FlexEnd
            | AlignContentKeyword::Center
            | AlignContentKeyword::Normal
            | AlignContentKeyword::Stretch => 0.0,
            AlignContentKeyword::SpaceBetween => free_space / usize_to_f32(items.saturating_sub(1)),
            AlignContentKeyword::SpaceAround => free_space / usize_to_f32(items),
            AlignContentKeyword::SpaceEvenly => free_space / usize_to_f32(items.saturating_add(1)),
        }
    }
}

/// Taffy's `child_first_baseline`.
pub(super) fn child_first_baseline(first: Option<f32>, height: f32, overflow_y: Overflow) -> Option<f32> {
    if overflow_y.is_scroll_container() {
        Some(first.unwrap_or(height).min(height).max(0.0))
    } else {
        first
    }
}

/// Taffy's `compute_scrollable_overflow_contribution`.
pub(super) fn overflow_contribution(
    location: Point<f32>,
    size: Size<f32>,
    rect: Rect<f32>,
    overflow: Point<Overflow>,
    contain: Contain,
    parent_is_scroll_container: bool,
) -> Rect<f32> {
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let contained = contain.contains_scrollable_overflow();
    let propagates = Point {
        x: !is_scroll_container && !contained && overflow.x == Overflow::Visible,
        y: !is_scroll_container && !contained && overflow.y == Overflow::Visible,
    };
    let end = Size {
        width: if propagates.x {
            size.width.max(rect.right)
        } else {
            size.width
        },
        height: if propagates.y {
            size.height.max(rect.bottom)
        } else {
            size.height
        },
    };
    if end.width <= 0.0 || end.height <= 0.0 {
        return Rect::ZERO;
    }
    let start = Point {
        x: if propagates.x { 0.0_f32.min(rect.left) } else { 0.0 },
        y: if propagates.y { 0.0_f32.min(rect.top) } else { 0.0 },
    };
    let contribution = Rect {
        left: location.x + start.x,
        right: location.x + end.width,
        top: location.y + start.y,
        bottom: location.y + end.height,
    };
    let unreachable = contribution.right <= 0.0 || contribution.bottom <= 0.0;
    if parent_is_scroll_container && unreachable {
        Rect::ZERO
    } else {
        contribution
    }
}

/// The container's resolved box and available space (Taffy's `compute_container_constants`).
struct Constants {
    contain: Contain,
    padding: Rect<f32>,
    border: Rect<f32>,
    padding_border_size: Size<f32>,
    min_size: Size<Option<f32>>,
    max_size: Size<Option<f32>>,
    preferred_size: Size<Option<f32>>,
    scrollbar_gutter: Point<f32>,
    is_scroll_container: bool,
    content_box_inset: Rect<f32>,
    align_content: AlignContent,
    justify_content: AlignContent,
    align_items: AlignItems,
    justify_items: AlignItems,
    available_grid_space: Size<AvailableSpace>,
    outer_node_size: Size<Option<f32>>,
    inner_min_size: Size<Option<f32>>,
    inner_node_size: Size<Option<f32>>,
}

fn constants<S: NodeStyle>(style: &S::Ref<'_>, inputs: &LayoutInput) -> Constants {
    let calc = S::calc;
    let LayoutInput {
        known_dimensions,
        parent_size,
        available_space,
        ..
    } = *inputs;
    let aspect_ratio = style.aspect_ratio();
    let padding = style.padding().resolve_or_zero(parent_size.width, calc);
    let border = style.border().resolve_or_zero(parent_size.width, calc);
    let padding_border = rect_add(padding, border);
    let padding_border_size = padding_border.sum_axes();
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        padding_border_size
    } else {
        Size::ZERO
    };
    let min_size = style
        .min_size()
        .maybe_resolve(parent_size, calc)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let max_size = style
        .max_size()
        .maybe_resolve(parent_size, calc)
        .maybe_apply_aspect_ratio(aspect_ratio)
        .maybe_add(box_sizing_adjustment);
    let preferred_size = if inputs.sizing_mode == SizingMode::InherentSize {
        style
            .size()
            .maybe_resolve(parent_size, calc)
            .maybe_apply_aspect_ratio(aspect_ratio)
            .maybe_add(box_sizing_adjustment)
    } else {
        Size::NONE
    };
    let overflow = style.overflow();
    let scrollbar_width = style.scrollbar_width();
    let scrollbar_gutter = Point {
        x: if overflow.y == Overflow::Scroll {
            scrollbar_width
        } else {
            0.0
        },
        y: if overflow.x == Overflow::Scroll {
            scrollbar_width
        } else {
            0.0
        },
    };
    let is_scroll_container = overflow.x.is_scroll_container() || overflow.y.is_scroll_container();
    let mut content_box_inset = padding_border;
    content_box_inset.bottom += scrollbar_gutter.y;
    // Left to right (the only direction this takes).
    content_box_inset.right += scrollbar_gutter.x;
    let given = known_dimensions.or(preferred_size);
    let constrained = Size {
        width: given.width.map_or(available_space.width, AvailableSpace::Definite),
        height: given.height.map_or(available_space.height, AvailableSpace::Definite),
    }
    .maybe_clamp(min_size, max_size)
    .maybe_max(padding_border_size);
    let available_grid_space = Size {
        width: constrained
            .width
            .map_definite_value(|space| space - content_box_inset.horizontal_axis_sum()),
        height: constrained
            .height
            .map_definite_value(|space| space - content_box_inset.vertical_axis_sum()),
    };
    let outer_node_size = given.maybe_clamp(min_size, max_size).maybe_max(padding_border_size);
    let inner_min_size = min_size.maybe_sub(content_box_inset.sum_axes());
    let inner_node_size = Size {
        width: outer_node_size
            .width
            .map(|space| space - content_box_inset.horizontal_axis_sum()),
        height: outer_node_size
            .height
            .map(|space| space - content_box_inset.vertical_axis_sum()),
    };
    Constants {
        contain: style.contain(),
        padding,
        border,
        padding_border_size,
        min_size,
        max_size,
        preferred_size,
        scrollbar_gutter,
        is_scroll_container,
        content_box_inset,
        align_content: style.grid_align_content(taffy::AbstractAxis::Block),
        justify_content: style.grid_align_content(taffy::AbstractAxis::Inline),
        align_items: GridContainerStyle::align_items(style),
        justify_items: GridContainerStyle::justify_items(style),
        available_grid_space,
        outer_node_size,
        inner_min_size,
        inner_node_size,
    }
}

/// Lay `view`'s node out as a grid for `inputs`, as
/// `taffy::compute_grid_layout` would; `None` when it is not a grid this
/// takes (before any child was asked: Taffy's own algorithm runs).
pub(super) fn compute<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: LayoutInput,
) -> Option<LayoutOutput> {
    let c = {
        let style = view.node.style.view();
        constants::<S>(&style, &inputs)
    };
    // Taffy's short-circuits: the size is all that was asked and it is known.
    if inputs.run_mode == RunMode::ComputeSize {
        if let Size {
            width: Some(width),
            height: Some(height),
        } = c.outer_node_size
        {
            view.node.free_answer = true;
            return Some(LayoutOutput::from_outer_size(Size { width, height }));
        }
        if inputs.axis == RequestedAxis::Horizontal
            && let Some(width) = c.outer_node_size.width
        {
            view.node.free_answer = true;
            return Some(LayoutOutput::from_outer_size(Size { width, height: 0.0 }));
        }
    }
    let mut scratch = take_scratch();
    let out = run(view, &inputs, &c, &mut scratch);
    give_scratch(scratch);
    out
}

/// Read the container's tracks and its children into `s`; whether it is a grid this takes.
fn accept<S: NodeStyle, L: Measure>(view: &View<'_, S, L>, c: &Constants, s: &mut Scratch) -> Option<()> {
    let style = view.node.style.view();
    if style.direction() != Direction::Ltr
        || style.grid_auto_flow() != GridAutoFlow::Row
        || is_baseline(c.align_items)
        || is_baseline(c.justify_items)
    {
        return None;
    }
    // The width sizes the columns without asking any item.
    c.inner_node_size.width?;
    let gap = GridContainerStyle::gap(&style);
    if !is_length(gap.width) || !is_length(gap.height) {
        return None;
    }
    if style
        .grid_template_areas()
        .is_some_and(|a| a.into_iter().next().is_some())
        || style
            .grid_template_column_names()
            .is_some_and(|mut n| n.any(|mut set| set.next().is_some()))
        || style
            .grid_template_row_names()
            .is_some_and(|mut n| n.any(|mut set| set.next().is_some()))
        || style.grid_template_rows().is_some_and(|mut t| t.next().is_some())
        || !style
            .grid_auto_rows()
            .all(|t| t.min_sizing_function().is_auto() && t.max_sizing_function().is_auto())
    {
        return None;
    }
    // Explicit columns: fixed, or `minmax(<fixed>, <n>fr)`.
    s.columns.push(Track::gutter(gap.width));
    let mut count = 0_usize;
    for component in style.grid_template_columns()? {
        let GenericGridTemplateComponent::Single(f) = component else {
            return None;
        };
        let (min, max) = (f.min_sizing_function(), f.max_sizing_function());
        let fixed_min = matches!(
            min.expand(),
            ExpandedMinTrackSizingFunction::Length(_) | ExpandedMinTrackSizingFunction::Percent(_)
        );
        let fixed_or_fr_max = matches!(
            max.expand(),
            ExpandedMaxTrackSizingFunction::Length(_)
                | ExpandedMaxTrackSizingFunction::Percent(_)
                | ExpandedMaxTrackSizingFunction::Fr(_)
        );
        if !fixed_min || !fixed_or_fr_max {
            return None;
        }
        s.columns.push(Track::new(min, max));
        s.columns.push(Track::gutter(gap.width));
        count = count.saturating_add(1);
    }
    if count == 0 || count > MAX_TRACKS {
        return None;
    }
    drop(style);
    // The items, each in the next cell, row by row.
    for (i, child) in view.node.children.iter().enumerate() {
        let cs = child.style.view();
        if cs.box_generation_mode() == BoxGenerationMode::None {
            continue;
        }
        if cs.position().is_out_of_flow()
            || child.oof
            || cs.is_replaced()
            || cs.is_compressible_replaced()
            || cs.size().width.is_sizing_keyword()
            || cs.size().height.is_sizing_keyword()
            || !auto_span_one(&GridItemStyle::grid_row(&cs))
            || !auto_span_one(&GridItemStyle::grid_column(&cs))
        {
            return None;
        }
        let align_self = GridItemStyle::align_self(&cs).unwrap_or(c.align_items);
        let justify_self = GridItemStyle::justify_self(&cs).unwrap_or(c.justify_items);
        if is_baseline(align_self) || is_baseline(justify_self) {
            return None;
        }
        let k = s.items.len();
        let r = k.checked_div(count)?;
        let col = k.checked_rem(count)?;
        let col = col.checked_mul(2)?.checked_add(1)?;
        let crosses_flexible_column = s.columns.get(col)?.max.is_fr();
        s.items.push(Item {
            child: i,
            row: r.checked_mul(2)?.checked_add(1)?,
            col,
            overflow: cs.overflow(),
            box_sizing: cs.box_sizing(),
            size: cs.size(),
            min_size: cs.min_size(),
            max_size: cs.max_size(),
            padding: cs.padding(),
            border: cs.border(),
            margin: cs.margin(),
            aspect_ratio: ratio_of(&cs),
            align_self,
            justify_self,
            crosses_flexible_column,
            area: None,
            known: None,
            min_content: None,
            max_content: None,
            minimum: None,
            min_contribution: 0.0,
            limited_contribution: None,
            max_contribution: 0.0,
            generation: 0,
            y: 0.0,
            height: 0.0,
            baseline: None,
        });
    }
    let rows = s.items.len().div_ceil(count);
    if rows == 0 || rows > MAX_TRACKS {
        return None;
    }
    if let Some(first) = s.columns.first_mut() {
        first.collapse();
    }
    if let Some(last) = s.columns.last_mut() {
        last.collapse();
    }
    s.rows.push(Track::gutter(gap.height));
    for _ in 0..rows {
        s.rows.push(Track::new(
            MinTrackSizingFunction::auto(),
            MaxTrackSizingFunction::auto(),
        ));
        s.rows.push(Track::gutter(gap.height));
    }
    if let Some(first) = s.rows.first_mut() {
        first.collapse();
    }
    if let Some(last) = s.rows.last_mut() {
        last.collapse();
    }
    Some(())
}

/// 11.4 Initialise track sizes.
fn initialize(tracks: &mut [Track], basis: Option<f32>, calc: fn(*const (), f32) -> f32) {
    for t in tracks {
        t.base = t.min.definite_value(basis, calc).unwrap_or(0.0);
        t.growth = t.max.definite_value(basis, calc).unwrap_or(f32::INFINITY);
        if t.growth < t.base {
            t.growth = t.base;
        }
    }
}

/// Whether the track sizes are final already (Taffy's skip of the rest of the algorithm).
fn sized(tracks: &[Track], basis: Option<f32>, calc: fn(*const (), f32) -> f32) -> bool {
    tracks
        .iter()
        .all(|t| t.base == t.growth && t.min.definite_value(basis, calc).is_some())
}

fn flush_base(tracks: &mut [Track]) {
    for t in tracks {
        t.base += t.base_planned;
        t.base_planned = 0.0;
    }
}

/// The flush that ends the span-1 step of resolving intrinsic track sizes.
fn flush_growth_span_one(tracks: &mut [Track]) {
    for t in tracks {
        if t.growth_planned > 0.0 {
            t.growth = if t.growth == f32::INFINITY {
                t.growth_planned
            } else {
                t.growth.max(t.growth_planned)
            };
        }
        t.growth_planned = 0.0;
        if t.growth < t.base {
            t.growth = t.base;
        }
    }
}

/// Taffy's `distribute_space_up_to_limits`, as `maximise_tracks` calls it.
fn distribute_up_to_growth(space: f32, tracks: &mut [Track]) -> f32 {
    const THRESHOLD: f32 = 0.01;
    let mut space = space;
    for _ in 0..=tracks.len() {
        if space <= THRESHOLD {
            break;
        }
        let open = |t: &Track| t.base + t.incurred < t.limited_growth();
        let proportion_sum: f32 = tracks.iter().filter(|t| open(t)).map(|_| 1.0).sum();
        if proportion_sum == 0.0 {
            break;
        }
        let Some(min_increase_limit) = tracks
            .iter()
            .filter(|t| open(t))
            .map(|t| (t.limited_growth() - t.base - t.incurred) / 1.0)
            .min_by(f32::total_cmp)
        else {
            break;
        };
        let increase = min_increase_limit.min(space / proportion_sum);
        for t in tracks.iter_mut() {
            let increase = increase * 1.0;
            if increase > 0.0 && t.base + t.incurred + increase <= t.limited_growth() + THRESHOLD {
                t.incurred += increase;
                space -= increase;
            }
        }
    }
    space
}

/// 11.6 Maximise tracks.
fn maximise(tracks: &mut [Track], available: AvailableSpace) {
    let used: f32 = tracks.iter().map(|t| t.base).sum();
    let free = available.compute_free_space(used);
    if free == f32::INFINITY {
        for t in tracks.iter_mut() {
            t.base = t.growth;
        }
    } else if free > 0.0 {
        distribute_up_to_growth(free, tracks);
        for t in tracks.iter_mut() {
            t.base += t.incurred;
            t.incurred = 0.0;
        }
    }
}

/// 11.7.1 Find the size of an fr.
fn find_size_of_fr(tracks: &[Track], space_to_fill: f32) -> f32 {
    if space_to_fill == 0.0 {
        return 0.0;
    }
    let mut hypothetical = f32::INFINITY;
    for _ in 0..=tracks.len() {
        let mut used_space = 0.0;
        let mut naive_flex_factor_sum = 0.0;
        for t in tracks {
            if t.max.is_fr() && t.max.into_raw().value() * hypothetical >= t.base {
                naive_flex_factor_sum += t.max.into_raw().value();
            } else {
                used_space += t.base;
            }
        }
        let leftover = space_to_fill - used_space;
        let flex_factor = f32::max(naive_flex_factor_sum, 1.0);
        let previous = hypothetical;
        hypothetical = leftover / flex_factor;
        let valid = tracks.iter().all(|t| {
            if t.max.is_fr() {
                let f = t.max.into_raw().value();
                f * hypothetical >= t.base || f * previous < t.base
            } else {
                true
            }
        });
        if valid {
            break;
        }
    }
    hypothetical
}

/// 11.8 Stretch `auto` tracks.
fn stretch_auto(tracks: &mut [Track], axis_min_size: Option<f32>, expansion: AvailableSpace) {
    let count = tracks.iter().filter(|t| t.max.is_auto()).count();
    if count == 0 {
        return;
    }
    let used: f32 = tracks.iter().map(|t| t.base).sum();
    let free = if expansion.is_definite() {
        expansion.compute_free_space(used)
    } else {
        match axis_min_size {
            Some(size) => size - used,
            None => 0.0,
        }
    };
    if free > 0.0 {
        let extra = free / usize_to_f32(count);
        for t in tracks.iter_mut().filter(|t| t.max.is_auto()) {
            t.base += extra;
        }
    }
}

/// The track sizing algorithm for the columns, whose tracks read no item.
fn size_columns(s: &mut Scratch, c: &Constants, calc: fn(*const (), f32) -> f32) -> Option<()> {
    let inner = c.inner_node_size.width?;
    let tracks = s.columns.as_mut_slice();
    let basis = Some(inner).or(c.inner_min_size.width);
    initialize(tracks, basis, calc);
    if sized(tracks, basis, calc) {
        return Some(());
    }
    // 11.5 Resolve intrinsic track sizes. Items in fixed columns, one span-1
    // batch: a fixed minimum and maximum take nothing from them.
    if s.items.iter().any(|i| !i.crosses_flexible_column) {
        flush_growth_span_one(tracks);
    }
    // Items crossing an `fr` column, the flexible batch: none crosses an
    // intrinsic track or one with a content-based minimum, and the space is
    // definite, so only its flushes run.
    if s.items.iter().any(|i| i.crosses_flexible_column) {
        flush_base(tracks);
        flush_base(tracks);
        flush_base(tracks);
        for t in tracks.iter_mut() {
            if t.growth < t.base {
                t.growth = t.base;
            }
        }
    }
    for t in tracks.iter_mut().filter(|t| t.growth == f32::INFINITY) {
        t.growth = t.base;
    }
    maximise(tracks, c.available_grid_space.width);
    // 11.7 Expand flexible tracks (the width is known: a definite space).
    let used: f32 = tracks.iter().map(|t| t.base).sum();
    let free = inner - used;
    let fraction = if free <= 0.0 {
        0.0
    } else {
        find_size_of_fr(tracks, inner)
    };
    for t in tracks.iter_mut().filter(|t| t.max.is_fr()) {
        let f = t.max.into_raw().value();
        t.base = f32::max(t.base, f * fraction);
    }
    if c.justify_content == AlignContent::STRETCH {
        stretch_auto(tracks, c.inner_min_size.width, AvailableSpace::Definite(inner));
    }
    Some(())
}

/// The child's id in the view.
fn id(child: usize) -> NodeId {
    NodeId::new(u64::try_from(child).unwrap_or(u64::MAX))
}

/// `margins_axis_sums_with_baseline_shims` (no shim: no baseline alignment).
fn margin_sums(item: &Item, width: Option<f32>, calc: fn(*const (), f32) -> f32) -> Size<f32> {
    Rect {
        left: item.margin.left.resolve_or_zero(Some(0.0), calc),
        right: item.margin.right.resolve_or_zero(Some(0.0), calc),
        top: item.margin.top.resolve_or_zero(width, calc) + 0.0,
        bottom: item.margin.bottom.resolve_or_zero(width, calc),
    }
    .sum_axes()
}

/// The item's grid area size in the rows' pass (`grid_area_size_cached`).
fn area(
    item: &mut Item,
    rows: &[Track],
    columns: &[Track],
    inner: Size<Option<f32>>,
    calc: fn(*const (), f32) -> f32,
) -> Option<Size<Option<f32>>> {
    if let Some(a) = item.area {
        return Some(a);
    }
    let row = rows.get(item.row)?;
    let height = std::iter::once(fixed_size(row, inner.height, calc)).sum::<Option<f32>>();
    // A track's content alignment adjustment is zero (only gutters get one,
    // and a span-1 item crosses none).
    let width = std::iter::once(Some(columns.get(item.col)?.base).map(|size| size + 0.0)).sum::<Option<f32>>();
    let a = Size { width, height };
    item.area = Some(a);
    Some(a)
}

/// A track's size while its sizes are being found, if its minimum and maximum are one fixed size.
fn fixed_size(t: &Track, basis: Option<f32>, calc: fn(*const (), f32) -> f32) -> Option<f32> {
    let min = t.min.definite_value(basis, calc)?;
    let max = t.max.definite_value(basis, calc)?;
    (min == max).then_some(t.base)
}

/// The size the item is measured at (`known_dimensions_cached`).
fn known(item: &mut Item, area: Size<Option<f32>>, calc: fn(*const (), f32) -> f32) -> Size<Option<f32>> {
    if let Some((a, k)) = item.known
        && a == area
    {
        return k;
    }
    let margins = margin_sums(item, area.width, calc);
    let padding = item.padding.resolve_or_zero(area.width, calc);
    let border = item.border.resolve_or_zero(area.width, calc);
    let padding_border_size = rect_add(padding, border).sum_axes();
    let box_sizing_adjustment = if item.box_sizing == BoxSizing::ContentBox {
        padding_border_size
    } else {
        Size::ZERO
    };
    let inherent = item.size.maybe_resolve(area, calc).maybe_add(box_sizing_adjustment);
    let min_size = transfer(item.min_size.maybe_resolve(area, calc), item.aspect_ratio, item.size)
        .maybe_add(box_sizing_adjustment)
        .or(padding_border_size.map(Some))
        .maybe_max(padding_border_size);
    let max_size = transfer(item.max_size.maybe_resolve(area, calc), item.aspect_ratio, item.size)
        .maybe_add(box_sizing_adjustment);
    let minus_margins = area.maybe_sub(margins).maybe_max(Size::ZERO);
    let stretch = |a: AlignSelf| matches!(a.keyword, AlignItemsKeyword::Stretch | AlignItemsKeyword::Normal);
    let width = inherent.width.or_else(|| {
        (!item.margin.left.is_auto() && !item.margin.right.is_auto() && stretch(item.justify_self))
            .then_some(minus_margins.width)
            .flatten()
    });
    let height = inherent.height.or_else(|| {
        (!item.margin.top.is_auto() && !item.margin.bottom.is_auto() && stretch(item.align_self))
            .then_some(minus_margins.height)
            .flatten()
    });
    let k = Size { width, height }.maybe_clamp(min_size, max_size);
    item.known = Some((area, k));
    k
}

/// The item's min- or max-content height, without margins (`min_content_contribution`).
fn content_height<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    item: &mut Item,
    area: Size<Option<f32>>,
    max: bool,
) -> f32 {
    let calc = S::calc;
    let cached = if max { item.max_content } else { item.min_content };
    if let Some(v) = cached {
        return v;
    }
    let k = known(item, area, calc);
    let v = match k.height {
        Some(h) => h,
        None => {
            let width = area.width.map(|w| {
                let margin = item.margin.resolve_or_zero(area.width, calc);
                f32::max(w - margin.horizontal_axis_sum(), 0.0)
            });
            let constraint = if max {
                AvailableSpace::MaxContent
            } else {
                AvailableSpace::MinContent
            };
            let input = LayoutInput {
                known_dimensions: k,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: area,
                available_space: Size {
                    width: width.map_or(constraint, AvailableSpace::Definite),
                    height: constraint,
                },
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Vertical,
                run_mode: RunMode::ComputeSize,
                vertical_margins_are_collapsible: Line::FALSE,
            };
            view.child_layout(id(item.child), input, None).size.height
        }
    };
    if max {
        item.max_content = Some(v);
    } else {
        item.min_content = Some(v);
    }
    v
}

/// The item's minimum contribution to its row, without margins (`minimum_contribution`).
fn minimum<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    item: &mut Item,
    area: Size<Option<f32>>,
    row: &Track,
    inner: Size<Option<f32>>,
) -> f32 {
    let calc = S::calc;
    if let Some(v) = item.minimum {
        return v;
    }
    let padding = item.padding.resolve_or_zero(area.width, calc);
    let border = item.border.resolve_or_zero(area.width, calc);
    let padding_border_size = rect_add(padding, border).sum_axes();
    let box_sizing_adjustment = if item.box_sizing == BoxSizing::ContentBox {
        padding_border_size
    } else {
        Size::ZERO
    };
    let preferred = item
        .size
        .maybe_resolve(area, calc)
        .maybe_add(box_sizing_adjustment)
        .height
        .map(|size| {
            let (min, max) = (item.min_size.height, item.max_size.height);
            if min.is_auto() && max.is_auto() {
                return size;
            }
            let basis = area.height;
            let adjustment = box_sizing_adjustment.height;
            let min = min.maybe_resolve(basis, calc);
            let max = max.maybe_resolve(basis, calc);
            size.maybe_clamp(min.maybe_add(adjustment), max.maybe_add(adjustment))
        });
    let v = match preferred
        .or_else(|| {
            transfer(
                item.min_size.maybe_resolve(area.map(|size| size.unwrap_or(0.0)), calc),
                item.aspect_ratio,
                item.size,
            )
            .maybe_add(box_sizing_adjustment)
            .height
        })
        .or_else(|| item.overflow.y.is_scroll_container().then_some(0.0))
    {
        Some(v) => v,
        // The automatic minimum size: content-based in an `auto`-minimum track.
        None if row.min.behaves_as_auto(inner.height) => {
            let min_content = content_height(view, item, area, false);
            let limit = row
                .max
                .definite_value(inner.height, calc)
                .map(|v| std::iter::once(v).sum::<f32>());
            min_content.maybe_min(limit.map(|limit| f32::max(limit, padding_border_size.height)))
        }
        None => 0.0,
    };
    item.minimum = Some(v);
    v
}

/// Ask each item its contributions to its row (`auto`, so none reads the
/// grid's height), in document order, as Taffy's span-1 batch asks them.
fn contribute<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    s: &mut Scratch,
    inner: Size<Option<f32>>,
) -> Option<()> {
    let calc = S::calc;
    for item in &mut s.items {
        item.area = None;
    }
    for item in &mut s.items {
        let a = area(item, &s.rows, &s.columns, inner, calc)?;
        let row = *s.rows.get(item.row)?;
        let margins = margin_sums(item, a.width, calc).height;
        let scroll = item.overflow.y.is_scroll_container();
        // Base size (`auto` minimum).
        let minimum_size = minimum(view, item, a, &row, inner) + margins;
        item.limited_contribution = if scroll {
            None
        } else {
            let min_content = content_height(view, item, a, false) + margins;
            let limit = row.max.definite_limit(inner.height, calc);
            Some(min_content.maybe_min(limit).max(minimum_size))
        };
        item.min_contribution = minimum_size;
        // Growth limit (`auto` maximum: the max-content contribution). Its
        // min-content contribution is asked first, as Taffy asks it.
        content_height(view, item, a, false);
        item.max_contribution = content_height(view, item, a, true) + margins;
        item.generation = view.node.children.get(item.child)?.cache.generation();
    }
    Some(())
}

/// The track sizing algorithm for the rows: `auto` tracks, span-1 items.
/// Returns whether the result stands for any available height.
fn size_rows<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    s: &mut Scratch,
    c: &Constants,
    inner: Size<Option<f32>>,
    recalled: bool,
) -> Option<bool> {
    let calc = S::calc;
    let available = c.available_grid_space.height;
    let basis = inner.height.or(c.inner_min_size.height);
    initialize(&mut s.rows, basis, calc);
    if sized(&s.rows, basis, calc) {
        return Some(true);
    }
    let mut free = true;
    // 11.5 Resolve intrinsic track sizes: one span-1 batch, in document order.
    if !recalled {
        contribute(view, s, inner)?;
    }
    for item in &s.items {
        let minimum_size = item.min_contribution;
        let limited_min_content = item.limited_contribution;
        let space = match (available, limited_min_content) {
            (AvailableSpace::MinContent | AvailableSpace::MaxContent, Some(v)) => v,
            _ => minimum_size,
        };
        // Under a definite space and under a min- or max-content one the same value?
        if let Some(v) = limited_min_content
            && v.to_bits() != minimum_size.to_bits()
        {
            free = false;
        }
        let track = s.rows.get_mut(item.row)?;
        track.base = f32::max(track.base, space);
        // Growth limit (`auto` maximum: the max-content contribution).
        track.growth_planned = f32::max(track.growth_planned, item.max_contribution);
    }
    flush_growth_span_one(&mut s.rows);
    for t in s.rows.iter_mut().filter(|t| t.growth == f32::INFINITY) {
        t.growth = t.base;
    }
    // Maximising is the same under any space when no row can grow.
    free &= s.rows.iter().all(|t| {
        t.base.is_finite() && t.base.to_bits() != (-0.0_f32).to_bits() && t.base.to_bits() == t.growth.to_bits()
    });
    maximise(&mut s.rows, available);
    // 11.7 Expand flexible tracks: no row is flexible. 11.8 Stretch `auto` tracks.
    let expansion = inner
        .height
        .map_or(AvailableSpace::MaxContent, AvailableSpace::Definite);
    if c.align_content == AlignContent::STRETCH {
        stretch_auto(&mut s.rows, c.inner_min_size.height, expansion);
    }
    Some(free)
}

/// Taffy's `align_tracks`, left to right.
fn align_tracks(size: f32, padding_start: f32, border_start: f32, tracks: &mut [Track], style: AlignContent) {
    let used: f32 = tracks.iter().map(|t| t.base).sum();
    let free_space = size - used;
    let origin = padding_start + border_start;
    let count = tracks.iter().skip(1).step_by(2).filter(|t| !t.collapsed).count();
    let alignment = alignment_fallback(free_space, count, style);
    let empty = if count == 0 {
        alignment_offset(free_space, count, alignment, true)
    } else {
        0.0
    };
    let mut total = origin + empty;
    let mut seen = false;
    for (i, t) in tracks.iter_mut().enumerate() {
        let open = (i & 1) == 1 && !t.collapsed;
        let first = open && !seen;
        let offset = if open {
            alignment_offset(free_space, count, alignment, first)
        } else {
            0.0
        };
        t.offset = total + offset;
        total = total + offset + t.base;
        if open {
            seen = true;
        }
    }
}

/// Taffy's `resolve_self_alignment_safety`.
fn safety(alignment: AlignItems, overflows: bool) -> AlignItemsKeyword {
    if alignment.is_safe() && overflows {
        AlignItemsKeyword::Start
    } else {
        alignment.keyword
    }
}

/// Taffy's `align_item_within_area`, for an item in flow, left to right.
fn align_within(area: Line<f32>, alignment: AlignSelf, size: f32, margin: Line<Option<f32>>) -> (f32, Line<f32>) {
    let non_auto = Line {
        start: margin.start.unwrap_or(0.0) + 0.0,
        end: margin.end.unwrap_or(0.0),
    };
    let area_size = f32::max(area.end - area.start, 0.0);
    let free_space = f32::max(area_size - size - (non_auto.start + non_auto.end), 0.0);
    let autos = usize::from(margin.start.is_none()).saturating_add(usize::from(margin.end.is_none()));
    let auto_size = if autos > 0 {
        free_space / usize_to_f32(autos)
    } else {
        0.0
    };
    let resolved = Line {
        start: margin.start.unwrap_or(auto_size) + 0.0,
        end: margin.end.unwrap_or(auto_size),
    };
    let overflows = size + (non_auto.start + non_auto.end) > area_size;
    let offset = match safety(alignment, overflows) {
        AlignItemsKeyword::End | AlignItemsKeyword::FlexEnd => area_size - size - resolved.end,
        AlignItemsKeyword::Center => (area_size - size + resolved.start - resolved.end) / 2.0,
        // Start, and what aligns as start (`normal`, `self-start` and
        // `self-end` are resolved before this).
        _ => resolved.start,
    };
    (area.start + offset, resolved)
}

/// Taffy's `relative_offset`, left to right.
fn relative_offset(position: Position, inset: Line<Option<f32>>) -> f32 {
    if position != Position::Relative {
        return 0.0;
    }
    inset.start.or(inset.end.map(|pos| -pos)).unwrap_or(0.0)
}

/// Taffy's `align_and_position_item` for item `item`, in flow; returns its
/// scrollable overflow contribution, and fills its position, height and baseline.
fn place<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    item: &mut Item,
    order: u32,
    area: Rect<f32>,
    c: &Constants,
) -> Option<Rect<f32>> {
    let calc = S::calc;
    let cb = Size {
        width: area.right - area.left,
        height: area.bottom - area.top,
    };
    let child = view.node.children.get(item.child)?;
    let style = child.style.view();
    let overflow = style.overflow();
    let contain = style.contain();
    let scrollbar_width = style.scrollbar_width();
    let item_direction = style.direction();
    let justify_self = GridItemStyle::justify_self(&style)
        .unwrap_or(c.justify_items)
        .resolve_self_relative(item_direction, Direction::Ltr, true);
    let align_self = GridItemStyle::align_self(&style)
        .unwrap_or(c.align_items)
        .resolve_self_relative(item_direction, Direction::Ltr, false);
    let position = style.position();
    let inset = style.inset();
    let inset_horizontal = inset
        .horizontal_components()
        .map(|size| size.resolve_to_option(cb.width, calc));
    let inset_vertical = inset
        .vertical_components()
        .map(|size| size.resolve_to_option(cb.height, calc));
    let padding = style.padding().map(|p| p.resolve_or_zero(Some(cb.width), calc));
    let border = style.border().map(|p| p.resolve_or_zero(Some(cb.width), calc));
    let padding_border_size = rect_add(padding, border).sum_axes();
    let box_sizing_adjustment = if style.box_sizing() == BoxSizing::ContentBox {
        padding_border_size
    } else {
        Size::ZERO
    };
    let size_style = style.size();
    let inherent = size_style.maybe_resolve(cb, calc).maybe_add(box_sizing_adjustment);
    let min_size = transfer(style.min_size().maybe_resolve(cb, calc), item.aspect_ratio, size_style)
        .maybe_add(box_sizing_adjustment)
        .or(padding_border_size.map(Some))
        .maybe_max(padding_border_size);
    let max_size = transfer(style.max_size().maybe_resolve(cb, calc), item.aspect_ratio, size_style)
        .maybe_add(box_sizing_adjustment);
    let horizontal = match justify_self.keyword {
        AlignItemsKeyword::Normal if inherent.width.is_some() => AlignSelf::START,
        AlignItemsKeyword::Normal => AlignSelf::STRETCH,
        _ => justify_self,
    };
    // An item with a ratio is not stretched in the block axis by `normal`
    // (css-grid-1 §6.2): its ratio gives its height from its width.
    let vertical = match align_self.keyword {
        AlignItemsKeyword::Normal if inherent.height.is_some() || item.aspect_ratio.is_some() => AlignSelf::START,
        AlignItemsKeyword::Normal => AlignSelf::STRETCH,
        _ => align_self,
    };
    let margin = style.margin().map(|m| m.resolve_to_option(cb.width, calc));
    drop(style);
    let minus_margins = Size {
        width: cb.width.maybe_sub(margin.left).maybe_sub(margin.right).max(0.0),
        height: (cb.height.maybe_sub(margin.top).maybe_sub(margin.bottom) - 0.0).max(0.0),
    };
    let width = inherent.width.or_else(|| {
        (margin.left.is_some() && margin.right.is_some() && horizontal == AlignSelf::STRETCH)
            .then_some(minus_margins.width)
    });
    let height = inherent.height.or_else(|| {
        (margin.top.is_some() && margin.bottom.is_some() && vertical == AlignSelf::STRETCH)
            .then_some(minus_margins.height)
    });
    let size = Size { width, height }.maybe_clamp(min_size, max_size);
    let mut out = view.child_layout(
        id(item.child),
        LayoutInput {
            known_dimensions: size,
            known_dimensions_are_definite: Size {
                width: true,
                height: true,
            },
            parent_size: cb.map(Some),
            available_space: minus_margins.map(AvailableSpace::Definite),
            sizing_mode: SizingMode::InherentSize,
            axis: RequestedAxis::Both,
            run_mode: RunMode::PerformLayout,
            vertical_margins_are_collapsible: Line::FALSE,
        },
        None,
    );
    // Something out of flow came up after all: Taffy records its grid for it.
    if !out.oof_candidates.take().is_empty() {
        return None;
    }
    let Size { width, height } = size.unwrap_or(out.size).maybe_clamp(min_size, max_size);
    let (x, x_margin) = align_within(
        Line {
            start: area.left,
            end: area.right,
        },
        horizontal,
        width,
        margin.horizontal_components(),
    );
    let (y, y_margin) = align_within(
        Line {
            start: area.top,
            end: area.bottom,
        },
        vertical,
        height,
        margin.vertical_components(),
    );
    let location = Point {
        x: x + relative_offset(position, inset_horizontal),
        y: y + relative_offset(position, inset_vertical),
    };
    let layout = Layout {
        order,
        location,
        size: Size { width, height },
        scrollable_overflow_rect: out.scrollable_overflow_rect,
        scrollbar_size: Size {
            width: if overflow.y == Overflow::Scroll {
                scrollbar_width
            } else {
                0.0
            },
            height: if overflow.x == Overflow::Scroll {
                scrollbar_width
            } else {
                0.0
            },
        },
        padding,
        border,
        margin: Rect {
            left: x_margin.start,
            right: x_margin.end,
            top: y_margin.start,
            bottom: y_margin.end,
        },
    };
    if let Some(child) = view.node.children.get_mut(item.child)
        && child.layout != layout
    {
        child.layout = layout;
        child.moved = true;
    }
    item.y = y;
    item.height = height;
    item.baseline = child_first_baseline(out.baselines.first, height, overflow.y);
    Some(overflow_contribution(
        Point {
            x: location.x - c.border.left,
            y: location.y - c.border.top,
        },
        Size { width, height },
        out.scrollable_overflow_rect,
        overflow,
        contain,
        c.is_scroll_container,
    ))
}

#[expect(clippy::too_many_lines, reason = "Taffy's algorithm, step by step, in its order")]
fn run<S: NodeStyle, L: Measure>(
    view: &mut View<'_, S, L>,
    inputs: &LayoutInput,
    c: &Constants,
    s: &mut Scratch,
) -> Option<LayoutOutput> {
    let calc = S::calc;
    accept(view, c, s)?;
    size_columns(s, c, calc)?;
    let recalled = recall(view, c, s);
    let mut inner = c.inner_node_size;
    let column_sum: f32 = s.columns.iter().map(|t| t.base).sum();
    inner.width = inner.width.or(Some(column_sum));
    let free = size_rows(view, s, c, inner, recalled)?;
    // Sizing the rows leaves nothing a next run's sizing would read (it
    // sets each base and growth limit afresh, and ends with nothing planned).
    if !recalled {
        remember(view, c, s);
    }
    let row_sum: f32 = s.rows.iter().map(|t| t.base).sum();
    // The available height is read only when no height is given.
    let given = inputs.known_dimensions.or(c.preferred_size);
    if given.height.is_some() || free {
        view.node.free_answer = true;
    }
    let inset = c.content_box_inset;
    let border_box = Size {
        width: given
            .width
            .unwrap_or_else(|| column_sum + inset.horizontal_axis_sum())
            .maybe_clamp(c.min_size.width, c.max_size.width)
            .max(c.padding_border_size.width),
        height: given
            .height
            .unwrap_or_else(|| row_sum + inset.vertical_axis_sum())
            .maybe_clamp(c.min_size.height, c.max_size.height)
            .max(c.padding_border_size.height),
    };
    if inputs.run_mode == RunMode::ComputeSize {
        return Some(LayoutOutput::from_outer_size(border_box));
    }
    let content_box = Size {
        width: f32::max(0.0, border_box.width - inset.horizontal_axis_sum()),
        height: f32::max(0.0, border_box.height - inset.vertical_axis_sum()),
    };
    // Neither axis sizes again: the width was known, no column is intrinsic
    // and no track or gap is a percentage of the height.

    // 8. Track alignment.
    // (The scrollbar gutter Taffy works out here moves the end, which aligning reads not.)
    align_tracks(
        content_box.width,
        c.padding.left + 0.0,
        c.border.left,
        &mut s.columns,
        c.justify_content,
    );
    align_tracks(
        content_box.height,
        c.padding.top,
        c.border.top,
        &mut s.rows,
        c.align_content,
    );

    // 9. Size, align and position the items.
    let mut overflow_rect = Rect::ZERO;
    for (order, item) in s.items.iter_mut().enumerate() {
        let area = Rect {
            top: s.rows.get(item.row)?.offset,
            bottom: s.rows.get(item.row.checked_add(1)?)?.offset,
            left: s.columns.get(item.col)?.offset,
            right: s.columns.get(item.col.checked_add(1)?)?.offset,
        };
        let contribution = place(view, item, u32::try_from(order).ok()?, area, c)?;
        overflow_rect = overflow_rect.union(contribution);
    }
    // Hidden children, after the items in order.
    let mut order = u32::try_from(s.items.len()).ok()?;
    let mut next = s.items.iter().map(|i| i.child).peekable();
    for child in 0..view.node.children.len() {
        if next.next_if_eq(&child).is_some() {
            continue;
        }
        let hidden = Layout::with_order(order);
        if let Some(n) = view.node.children.get_mut(child)
            && n.layout != hidden
        {
            n.layout = hidden;
            n.moved = true;
        }
        view.child_layout(
            id(child),
            LayoutInput {
                known_dimensions: Size::NONE,
                known_dimensions_are_definite: Size {
                    width: true,
                    height: true,
                },
                parent_size: Size::NONE,
                available_space: Size {
                    width: AvailableSpace::MaxContent,
                    height: AvailableSpace::MaxContent,
                },
                sizing_mode: SizingMode::InherentSize,
                axis: RequestedAxis::Both,
                run_mode: RunMode::PerformLayout,
                vertical_margins_are_collapsible: Line::FALSE,
            },
            None,
        );
        order = order.saturating_add(1);
    }
    // What out-of-flow children would read: none is inside it.
    view.node.info = taffy::DetailedLayoutInfo::None;

    let position_inset = rect_add(
        c.border,
        Rect {
            left: 0.0,
            right: c.scrollbar_gutter.x,
            top: 0.0,
            bottom: c.scrollbar_gutter.y,
        },
    );
    let baseline = if c.contain.suppresses_baseline() {
        None
    } else {
        // The first item is in the first row, and none is aligned by its baseline.
        let first = s.items.first()?;
        Some(first.y + first.baseline.unwrap_or(first.height))
    };
    if c.is_scroll_container {
        overflow_rect.right += c.padding.right;
        overflow_rect.bottom += c.padding.bottom;
    }
    let mut output = LayoutOutput::from_sizes_and_baselines(border_box, overflow_rect, Baselines::from_first(baseline));
    output.oof_candidates = OofCandidates::new();
    output.oof_positioning_area = Some(OofPositioningArea {
        size: size_sub(border_box, position_inset.sum_axes()),
        offset: Point {
            x: position_inset.left,
            y: position_inset.top,
        },
    });
    Some(output)
}

/// Taffy's own grid (`compute_grid_layout`), for what [`compute`] does not
/// take, with the container's own ratio resolved as `sizing.rs` has it:
/// Taffy takes the ratio's size as definite, so it is asked first for the
/// content's size without it (a content-size probe, which does not read
/// the preferred size the ratio is carried from), and then laid out at the
/// larger.
pub(super) fn fallback<S: NodeStyle, L: Measure>(view: &mut View<'_, S, L>, inputs: LayoutInput) -> LayoutOutput {
    let me = NodeId::new(super::SELF);
    let calc = S::calc;
    let (styled, ratio) = {
        let style = view.node.style.view();
        let aspect_ratio = ratio_of(&style);
        if aspect_ratio.is_none() {
            drop(style);
            return taffy::compute_grid_layout(view, me, inputs);
        }
        let parent_size = inputs.parent_size;
        let padding = style.padding().resolve_or_zero(parent_size.width, calc);
        let border = style.border().resolve_or_zero(parent_size.width, calc);
        let padding_border_size = rect_add(padding, border).sum_axes();
        let adjustment = if style.box_sizing() == BoxSizing::ContentBox {
            padding_border_size
        } else {
            Size::ZERO
        };
        let raw_min = style.min_size().maybe_resolve(parent_size, calc).maybe_add(adjustment);
        let raw_max = style.max_size().maybe_resolve(parent_size, calc).maybe_add(adjustment);
        let preferred = if inputs.sizing_mode == SizingMode::InherentSize {
            style
                .size()
                .maybe_resolve(parent_size, calc)
                .maybe_add(adjustment)
                .maybe_clamp(raw_min, raw_max)
        } else {
            Size::NONE
        };
        let styled = inputs.known_dimensions.or(preferred);
        let overflow = style.overflow();
        let ratio = Ratio::find(
            aspect_ratio,
            styled,
            adjustment,
            raw_min,
            raw_max,
            padding_border_size,
            overflow.x.is_scroll_container() || overflow.y.is_scroll_container(),
        );
        (styled, ratio)
    };
    let Some(ratio) = ratio else {
        return taffy::compute_grid_layout(view, me, inputs);
    };
    with_ratio(
        view,
        LayoutInput {
            known_dimensions: styled,
            ..inputs
        },
        ratio,
        Reports::Nothing,
        |view, inputs, _| {
            let probe = inputs.known_dimensions.width.is_none() || inputs.known_dimensions.height.is_none();
            let inputs = if probe {
                LayoutInput {
                    sizing_mode: SizingMode::ContentSize,
                    ..inputs
                }
            } else {
                inputs
            };
            Laid {
                output: taffy::compute_grid_layout(view, me, inputs),
                content: None,
            }
        },
    )
}
